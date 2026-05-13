//! End-to-end: mock_facilitator + parser_grpc_server + parser_gateway,
//! exercising the v2 x402-gated route alongside the v1 open route.
//!
//! Run with:
//!   cargo test -p integration --test x402_gateway_test -- --test-threads=1

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

// ── Ports used by all five tests (fixed; tests run single-threaded) ───────────
const MOCK_PORT: u16 = 18090;
// Note: parser_grpc_server always binds 0.0.0.0:44020 (hardcoded in binary).
// The gateway is pointed at that address via GRPC_ADDR env var.
const GW_PORT: u16 = 18080;

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

async fn start_procs() -> Procs {
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

    let grpc = Command::new(target_bin("parser_grpc_server"))
        // The server defaults to "integration/fixtures/ephemeral.secret" relative
        // to cwd. When cargo runs integration tests the cwd is src/integration/.
        .env("EPHEMERAL_FILE", "fixtures/ephemeral.secret")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parser_grpc_server");

    // 3. Start the gateway last — it probes the mock at startup.
    //    Friction 5: env var names confirmed from gateway/src/main.rs.
    let gateway = Command::new(target_bin("parser_gateway"))
        .env("GATEWAY_PORT", GW_PORT.to_string())
        // grpc server always listens on 44020 (hardcoded in binary)
        .env("GRPC_ADDR", "http://127.0.0.1:44020")
        .env("X402_PROFILE", "local")
        .env(
            "X402_FACILITATOR_URL",
            format!("http://127.0.0.1:{MOCK_PORT}"),
        )
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
    let _p = start_procs().await;

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
    let _p = start_procs().await;

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
    let _p = start_procs().await;

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

/// Path 4: POST /visualsign/api/v1/parse without payment header → 200 (open route).
#[tokio::test]
async fn path4_v1_without_payment_returns_200() {
    let _p = start_procs().await;

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

    assert_ne!(resp.status(), 402, "v1 route must not require payment");
    assert_eq!(resp.status(), 200, "v1 route must return 200");
}

/// Path 5: GET /health → 200 with no authentication.
#[tokio::test]
async fn path5_health_open() {
    let _p = start_procs().await;

    let resp = reqwest::get(format!("http://127.0.0.1:{GW_PORT}/health"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}
