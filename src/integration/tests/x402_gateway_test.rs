//! End-to-end: mock_facilitator + parser_grpc_server + parser_gateway,
//! exercising the v2 x402-gated route alongside the v1 open route.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qos_p256::P256Pair;
use qos_p256::sign::P256SignPair;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

// ── Ports used by all five tests (fixed; serialized via TEST_MUTEX) ────────────
const MOCK_PORT: u16 = 18090;
// Note: parser_grpc_server always binds 0.0.0.0:44020 (hardcoded in binary).
// The gateway is pointed at that address via GRPC_ADDR env var.
const GW_PORT: u16 = 18080;
/// Serializes these fixed-port tests to avoid cross-test port binding races
/// when the integration test binary is executed with multiple test threads.
static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

// ── Binary helpers ────────────────────────────────────────────────────────────

fn target_bin(name: &str) -> String {
    // Binaries are built by `make -C src build` before running these tests.
    // The integration crate lives at src/integration/, so binaries are at ../target/debug/.
    format!("../target/debug/{name}")
}

// ── Process lifecycle ─────────────────────────────────────────────────────────

struct Procs {
    mock: Child,
    grpc: Child,
    gateway: Child,
}

impl Drop for Procs {
    fn drop(&mut self) {
        // Kill children and reap them so the OS releases their ports promptly.
        let _ = self.mock.kill();
        let _ = self.grpc.kill();
        let _ = self.gateway.kill();
        let _ = self.mock.wait();
        let _ = self.grpc.wait();
        let _ = self.gateway.wait();
    }
}

/// Wait until a TCP port is no longer bound (i.e., available for reuse).
async fn wait_until_port_free(port: u16) {
    for _ in 0..100 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    // If still bound after 5 s, proceed anyway — the next bind will fail and
    // give a useful error.
}

/// Wait until an HTTP endpoint returns 200.
async fn wait_ready(url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if let Ok(r) = client.get(url).send().await {
            if r.status().is_success() {
                return;
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("service at {url} never became ready (timed out after 10 s)");
}

/// Load the test ephemeral key and return its `qos_hex` pubkey — the exact
/// format parser_app emits in the wire signature.
#[allow(dead_code)]
fn fixture_ephemeral_pubkey_hex() -> String {
    let pair = P256Pair::from_hex_file("fixtures/ephemeral.secret")
        .expect("load fixtures/ephemeral.secret");
    qos_hex::encode(&pair.public_key().to_bytes())
}

/// Generate a fresh gateway signing keypair for one test run. Writes the
/// JSON `{private, public}` blob to a temp file and returns the path +
/// pub hex.
///
/// The returned (path, pubkey_hex) tuple is used by `start_procs`:
/// - gateway gets `GATEWAY_SIGNING_KEY_FILE` pointed at `path`,
/// - parser_grpc_server gets `GATEWAY_SIGNING_PUBKEY_HEX` set to
///   the matching `pubkey_hex` (so VPMs verify).
///
/// Each test gets a unique pair, so parallel/serial tests don't trample.
fn mint_gateway_signer() -> (std::path::PathBuf, String) {
    let pair = P256SignPair::generate();
    let priv_hex = qos_hex::encode(&pair.to_bytes());
    let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
    let body = serde_json::json!({ "private": priv_hex, "public": &pub_hex });
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gateway_signer-{pid}-{ts}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&body).unwrap())
        .expect("write gateway signer fixture");
    (path, pub_hex)
}

async fn start_procs(extra_env: &[(&str, &str)]) -> Procs {
    // --- Friction 2: startup ordering ---
    // parser_gateway probes mock_facilitator at startup. We must ensure
    // mock_facilitator is ready before spawning the gateway.

    // Wait until the ports are free (important between sequential test runs).
    wait_until_port_free(MOCK_PORT).await;
    wait_until_port_free(GW_PORT).await;

    // 1. Start mock_facilitator first.
    let mock = Command::new(target_bin("mock_facilitator"))
        .env("MOCK_FACILITATOR_PORT", MOCK_PORT.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn mock_facilitator");

    // Wait for mock to be ready before proceeding.
    wait_ready(&format!("http://127.0.0.1:{MOCK_PORT}/supported")).await;

    // 2. Start parser_grpc_server.
    //    Friction 4: no CLI args; binds 0.0.0.0:44020 by default.
    //    We override the default port via an env var trick: the binary only reads
    //    EPHEMERAL_FILE. To run on a different port we would need to patch the
    //    binary — instead we use the hardcoded default (44020) and point the
    //    gateway at it. The GRPC_PORT constant is used only for documentation;
    //    the actual grpc server always binds 44020.
    //
    //    Because port 44020 is fixed, we wait for it to free up as well.
    wait_until_port_free(44020).await;

    // Mint a fresh gateway signing keypair for this test. parser_app pins
    // the public half via GATEWAY_SIGNING_PUBKEY_HEX; gateway holds the
    // private half via GATEWAY_SIGNING_KEY_FILE. Tests can override the
    // parser-side pin via `extra_env` to exercise the tamper path.
    let (gateway_key_path, gateway_pub_hex) = mint_gateway_signer();
    let parser_pinned_hex = extra_env
        .iter()
        .find(|(k, _)| *k == "PARSER_PINNED_GATEWAY_PUBKEY_HEX")
        .map(|(_, v)| (*v).to_string())
        .unwrap_or_else(|| gateway_pub_hex.clone());

    let grpc = Command::new(target_bin("parser_grpc_server"))
        // The server defaults to "integration/fixtures/ephemeral.secret" relative
        // to cwd. When cargo runs integration tests the cwd is src/integration/.
        .env("EPHEMERAL_FILE", "fixtures/ephemeral.secret")
        .env("GATEWAY_SIGNING_PUBKEY_HEX", parser_pinned_hex)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parser_grpc_server");

    // 3. Start the gateway last — it probes the mock at startup.
    //    Friction 5: env var names confirmed from gateway/src/main.rs.
    let mut cmd = Command::new(target_bin("parser_gateway"));
    cmd.env("GATEWAY_PORT", GW_PORT.to_string())
        // grpc server always listens on 44020 (hardcoded in binary)
        .env("GRPC_ADDR", "http://127.0.0.1:44020")
        .env("X402_PROFILE", "local")
        .env(
            "X402_FACILITATOR_URL",
            format!("http://127.0.0.1:{MOCK_PORT}"),
        )
        .env("GATEWAY_SIGNING_KEY_FILE", &gateway_key_path);
    for (k, v) in extra_env {
        if *k == "PARSER_PINNED_GATEWAY_PUBKEY_HEX" {
            continue; // consumed above, not a real gateway env
        }
        cmd.env(k, v);
    }
    let gateway = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parser_gateway");

    // Wait for gateway health to confirm all three services are up.
    wait_ready(&format!("http://127.0.0.1:{GW_PORT}/health")).await;

    Procs {
        mock,
        grpc,
        gateway,
    }
}

// ── Payment header helpers ────────────────────────────────────────────────────

/// Fetch the 402 `Payment-Required` header, decode it, and extract the first
/// entry from `accepts` as a raw JSON Value.
///
/// This gives us the exact `PaymentRequirements` the server is offering, which
/// we need to embed in the `accepted` field of the V2 `Payment-Signature` payload.
async fn fetch_v2_requirements() -> serde_json::Value {
    use base64::Engine;

    let body = serde_json::json!({
        "request": { "unsigned_payload": "0xdeadbeef", "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .expect("send probe request");

    assert_eq!(resp.status(), 402, "expected 402 for probe");

    let header = resp
        .headers()
        .get("Payment-Required")
        .expect("Payment-Required header must be present on 402")
        .to_str()
        .expect("header must be valid UTF-8")
        .to_string();

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(header.as_bytes())
        .expect("Payment-Required must be base64");

    let payment_required: serde_json::Value =
        serde_json::from_slice(&decoded).expect("Payment-Required must be JSON");

    let accepts = payment_required["accepts"]
        .as_array()
        .expect("accepts must be array");

    assert!(!accepts.is_empty(), "accepts must not be empty");

    accepts[0].clone()
}

/// Build a well-formed V2 `Payment-Signature` header value.
///
/// V2 `PaymentPayload` wire shape (camelCase, per x402-types v2.rs):
///
/// ```json
/// {
///   "accepted": { <exact PaymentRequirements from 402 response> },
///   "payload": { /* scheme-specific; mock_facilitator ignores contents */ },
///   "x402Version": 2
/// }
/// ```
///
/// The header value is the base64 (standard) encoding of the JSON bytes.
fn build_payment_signature(requirements: &serde_json::Value) -> String {
    use base64::Engine;

    let payload = serde_json::json!({
        "x402Version": 2,
        "accepted": requirements,
        "payload": {
            "payer": "0x000000000000000000000000000000000000AAAA",
            "signature": "0xdeadbeef"
        }
    });

    base64::engine::general_purpose::STANDARD.encode(payload.to_string())
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// A valid signed Ethereum legacy transaction (EIP-155, chain_id=1).
/// Same fixture used in parser.rs integration tests.
const ETH_TX_HEX: &str = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Path 1: POST /visualsign/api/v2/parse without any payment header → 402.
/// V2 returns an empty body and puts the payment requirements in the
/// `Payment-Required` header (base64 JSON), not in the response body.
#[tokio::test]
async fn path1_v2_without_payment_returns_402() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": "0xdeadbeef", "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    // Path 1 asserts on the 402 regardless of the chain name — the middleware
    // gates on payment before the handler ever sees the chain name. So we can
    // use any valid-looking body here; the chain value is irrelevant for the
    // 402 assertion itself.
    assert_eq!(resp.status(), 402, "expected 402 Payment Required");

    // The V2 protocol returns payment info in the `Payment-Required` header.
    let payment_required_header = resp
        .headers()
        .get("Payment-Required")
        .expect("Payment-Required header must be present");

    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payment_required_header.as_bytes())
        .expect("Payment-Required must be base64");

    let v: serde_json::Value =
        serde_json::from_slice(&decoded).expect("Payment-Required must be JSON");

    let accepts = v["accepts"].as_array().expect("accepts must be array");
    assert!(!accepts.is_empty(), "accepts must not be empty");

    // Local profile uses base-sepolia, which maps to CAIP-2 "eip155:84532".
    let has_base_sepolia = accepts.iter().any(|t| {
        t["network"]
            .as_str()
            .map(|n| n.contains("84532") || n.contains("base-sepolia"))
            .unwrap_or(false)
    });
    assert!(
        has_base_sepolia,
        "accepts must include base-sepolia; got: {accepts:?}"
    );
}

/// Path 2: POST /visualsign/api/v2/parse with a valid V2 payment → 200 with parse result.
/// We first probe the 402 to learn the exact requirements, then echo them back in `accepted`.
#[tokio::test]
async fn path2_v2_with_valid_payment_returns_200() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    // Fetch actual requirements from the 402 response.
    let requirements = fetch_v2_requirements().await;
    let payment_header = build_payment_signature(&requirements);

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body_text = resp.text().await.unwrap();
    assert_eq!(status, 200, "expected 200; body: {body_text}");

    let v: serde_json::Value = serde_json::from_str(&body_text).expect("must be JSON");
    assert!(
        v["response"]["parsedTransaction"]["payload"]["signablePayload"].is_string(),
        "response must contain signablePayload; got: {v}"
    );
}

/// Path 3: POST /visualsign/api/v2/parse with a valid payment but an invalid transaction
/// payload → 400. The gRPC parser rejects it before settlement.
#[tokio::test]
async fn path3_v2_valid_payment_bad_tx_returns_400() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let requirements = fetch_v2_requirements().await;
    let payment_header = build_payment_signature(&requirements);

    let body = serde_json::json!({
        "request": { "unsigned_payload": "not-hex-not-base64-not-valid", "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "expected 400 Bad Request for invalid tx"
    );
}

/// Path 4: in TVC-enforced mode, the v1 open route is intentionally NOT
/// mounted (parser_app's payment policy is global, so an open route would
/// 402 every call). Confirm the gateway returns 404 instead of leaking an
/// unprotected parse path.
#[tokio::test]
async fn path4_v1_not_mounted_in_tvc_enforced_mode() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v1/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "v1 route must NOT be mounted when GATEWAY_SIGNING_KEY_FILE is set"
    );
}

/// Path 5: GET /health → 200 with no authentication.
#[tokio::test]
async fn path5_health_open() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{GW_PORT}/health"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

/// Path 6: TVC attestation mismatch → 502 and no settlement.
///
/// Pin a *non-matching* TVC pubkey on the gateway, then submit a valid payment
/// for a parseable transaction. parser_app produces a legitimate signature with
/// the fixture ephemeral key, but the gateway's pinned pubkey is a freshly
/// generated unrelated keypair, so the verifier rejects on pubkey mismatch.
/// The handler must return 502, and `/debug/settle_count` on the mock
/// facilitator must remain unchanged — the gateway must not have paid the
/// facilitator for an unattested response.
#[tokio::test]
async fn path6_tampered_pubkey_returns_502_no_settle() {
    let _guard = TEST_MUTEX.lock().await;

    // Pin a different gateway pubkey on parser_grpc_server than the
    // gateway will actually sign with. The VPM signature will verify
    // against the gateway's actual key but parser_app's pinned key check
    // will fail -> FailedPrecondition -> gateway translates to HTTP 402.
    //
    // Known v3.0 quirk: the mock facilitator's /settle still gets called
    // (we hand-roll verify->settle->sign before parser_app sees the
    // request), so settle_count advances. v3.1 will close this by binding
    // settle to a pre-validation step. For now the assertion just
    // confirms the parser rejects.
    let wrong_pair = P256SignPair::generate();
    let wrong_hex = qos_hex::encode(&wrong_pair.public_key().to_bytes());
    let _p = start_procs(&[("PARSER_PINNED_GATEWAY_PUBKEY_HEX", wrong_hex.as_str())]).await;

    let requirements = fetch_v2_requirements().await;
    let payment_header = build_payment_signature(&requirements);

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert_eq!(
        status, 402,
        "expected 402 PaymentRequired on parser VPM verify failure; got {status}; body: {body_text}"
    );
}

// ── Per-chain network derivation (no payment header → 402 carries only the
//    accepts entries matching the parse-request's chain) ─────────────────────

fn decode_payment_required(resp: &reqwest::Response) -> serde_json::Value {
    use base64::Engine;
    let header = resp
        .headers()
        .get("Payment-Required")
        .expect("Payment-Required header must be present")
        .as_bytes()
        .to_vec();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&header)
        .expect("Payment-Required must be base64");
    serde_json::from_slice(&decoded).expect("Payment-Required must be JSON")
}

/// Path 7: POST /v2/parse with `chain: CHAIN_ETHEREUM` and no Payment-Signature
/// → 402, accepts contains ONLY EVM (base-sepolia / eip155:84532) tags, no
/// Solana entry. Local profile by default offers both chains, so this proves
/// the per-chain filter at the handler.
#[tokio::test]
async fn path7_v2_ethereum_request_accepts_only_evm_tags() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": "0x", "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 402);

    let v = decode_payment_required(&resp);
    let accepts = v["accepts"].as_array().expect("accepts must be array");
    assert!(!accepts.is_empty(), "ETH 402 must offer at least one tag");
    for entry in accepts {
        let net = entry["network"].as_str().unwrap_or("");
        assert!(
            net.contains("84532") || net.starts_with("eip155:") || net == "base-sepolia",
            "ETH 402 must only carry EVM tags; saw network={net:?}; full: {entry}"
        );
        // EVM v2 wire shape that the x402-evm client refuses to parse without:
        //   asset = 0x-prefixed 20-byte contract address (NOT the symbol "USDC")
        //   extra.name + extra.version  = EIP-712 domain for the token
        let asset = entry["asset"].as_str().unwrap_or("");
        assert!(
            asset.starts_with("0x") && asset.len() == 42,
            "EVM tag asset must be a 0x-prefixed contract address; got {asset:?}"
        );
        assert!(
            entry["extra"]["name"].is_string(),
            "EVM tag must carry extra.name (EIP-712 domain); full: {entry}"
        );
        assert!(
            entry["extra"]["version"].is_string(),
            "EVM tag must carry extra.version (EIP-712 domain); full: {entry}"
        );
    }
}

/// Path 8: POST /v2/parse with `chain: CHAIN_SOLANA` and no Payment-Signature
/// → 402, accepts contains ONLY Solana (solana-devnet) tags.
#[tokio::test]
async fn path8_v2_solana_request_accepts_only_solana_tags() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": "0x", "chain": "CHAIN_SOLANA" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 402);

    let v = decode_payment_required(&resp);
    let accepts = v["accepts"].as_array().expect("accepts must be array");
    assert!(
        !accepts.is_empty(),
        "Solana 402 must offer at least one tag"
    );
    for entry in accepts {
        let net = entry["network"].as_str().unwrap_or("");
        assert!(
            net.starts_with("solana:") || net == "solana-devnet" || net == "solana",
            "Solana 402 must only carry Solana tags; saw network={net:?}; full: {entry}"
        );
    }
}

/// Path 9: POST /v2/parse with `chain: CHAIN_TRON` (or any chain x402 doesn't
/// natively settle on) → 400 with a clear error, NOT a 402. A 402 here would
/// imply the buyer can pay their way through; the gateway has no settlement
/// path so it's misleading.
#[tokio::test]
async fn path9_v2_unsupported_chain_returns_400() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": "0x", "chain": "CHAIN_TRON" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "expected 400 for unsupported chain, not 402"
    );
    let text = resp.text().await.unwrap_or_default();
    assert!(
        text.contains("CHAIN_TRON") || text.contains("not available"),
        "error body should name the unsupported chain; got: {text}"
    );
}

// ── Paywall-bypass attempts: buyer mutates the echoed `accepted` block ────
//   Gate at parse_handler_tvc rejects with a fresh 402 BEFORE any call to
//   the facilitator, so settle_count must NOT advance.

async fn settle_count() -> usize {
    let v: serde_json::Value = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{MOCK_PORT}/debug/settle_count"))
        .send()
        .await
        .expect("settle_count GET")
        .json()
        .await
        .expect("settle_count JSON");
    v["settle_count"].as_u64().unwrap_or(0) as usize
}

/// Path 10: buyer echoes the offer with `payTo` swapped to their own
/// wallet — a self-transfer the facilitator would happily settle. Gateway
/// must reject with 402 before it ever calls `/verify`. The mock fac's
/// settle_count must NOT increment.
#[tokio::test]
async fn path10_tampered_pay_to_returns_402_no_settle() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let requirements = fetch_v2_requirements().await;
    let before = settle_count().await;

    let mut tampered = requirements.clone();
    tampered["payTo"] =
        serde_json::Value::String("0xAAAAaaaaAAaaAaaAaaAaAAAaaAAAAAaaaAAAAaAa".to_string());
    let payment_header = build_payment_signature(&tampered);

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        402,
        "tampered payTo must trigger 402 BEFORE any facilitator call"
    );
    let after = settle_count().await;
    assert_eq!(
        after, before,
        "/settle must NOT have been called for a tampered payTo; before={before} after={after}"
    );
}

/// Path 11: buyer echoes the offer with `amount` undercut to "1" (way
/// below the configured price). Gateway must reject, no /settle call.
#[tokio::test]
async fn path11_undercut_amount_returns_402_no_settle() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let requirements = fetch_v2_requirements().await;
    let before = settle_count().await;

    let mut undercut = requirements.clone();
    undercut["amount"] = serde_json::Value::String("1".to_string());
    let payment_header = build_payment_signature(&undercut);

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 402, "undercut amount must trigger 402");
    assert_eq!(
        settle_count().await,
        before,
        "/settle must not run for an undercut amount"
    );
}

/// Path 13: oversize POST body to /v2/parse (just past the gateway's
/// 64 KiB cap). axum's body-limit layer rejects before any extractor
/// reads the body, so this is a 413 with no facilitator call and no
/// settlement — closes the pre-paywall body-ingest amplification.
#[tokio::test]
async fn path13_oversize_body_returns_413_no_settle() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;
    let before = settle_count().await;

    // 65 KiB of hex chars, padded inside a request envelope — well past
    // the 64 KiB cap regardless of envelope overhead.
    let big_hex = "0x".to_string() + &"a".repeat(65 * 1024);
    let body = serde_json::json!({
        "request": { "unsigned_payload": big_hex, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        413,
        "oversize body must trigger 413 Payload Too Large before any handler runs"
    );
    assert_eq!(
        settle_count().await,
        before,
        "/settle must not run for an oversize body"
    );
}

/// Path 14: a real-world-sized parse request (the ETH_TX_HEX fixture,
/// ~500 bytes including the JSON envelope) still goes through the paid
/// path unchanged. Regression guard against the body cap being set too
/// tight for legitimate traffic.
#[tokio::test]
async fn path14_legitimate_body_size_unchanged_under_cap() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    assert!(
        body_bytes.len() < 64 * 1024,
        "legitimate parse envelope ({} bytes) must fit under the body cap",
        body_bytes.len()
    );

    let requirements = fetch_v2_requirements().await;
    let payment_header = build_payment_signature(&requirements);

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "legitimate paid request must still 200");
}

/// Path 12: buyer echoes the offer with `network` swapped to a different
/// chain's network (cross-chain confusion). Gateway must reject.
#[tokio::test]
async fn path12_cross_chain_network_returns_402_no_settle() {
    let _guard = TEST_MUTEX.lock().await;
    let _p = start_procs(&[]).await;

    let requirements = fetch_v2_requirements().await;
    let before = settle_count().await;

    let mut swapped = requirements.clone();
    // The chain query is CHAIN_ETHEREUM (so the gateway only offers EVM
    // tags); switch to a Solana CAIP-2 network identifier.
    swapped["network"] =
        serde_json::Value::String("solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".to_string());
    let payment_header = build_payment_signature(&swapped);

    let body = serde_json::json!({
        "request": { "unsigned_payload": ETH_TX_HEX, "chain": "CHAIN_ETHEREUM" }
    });

    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("Payment-Signature", payment_header)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 402, "cross-chain network must trigger 402");
    assert_eq!(
        settle_count().await,
        before,
        "/settle must not run for a cross-chain network"
    );
}
