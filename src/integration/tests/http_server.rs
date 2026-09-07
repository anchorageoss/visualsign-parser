#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::fs;
use std::process::Command;
use std::time::Duration;

use integration::{ChildWrapper, find_free_port, wait_until_port_is_bound};
use qos_p256::P256Pair;
use qos_test_primitives::PathWrapper;

// Same Ethereum signed legacy transaction used by
// `integration::tests::parser_ethereum_native_transfer_e2e`.
const ETH_TX_HEX: &str = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

// Same Solana transfer message used by
// `visualsign_solana::core::visualsign::tests::intermediate_output_emitted_for_known_transfer`
// and `integration::tests::parser_solana_native_transfer_e2e` - the System
// Program decode path is the one that actually emits a non-empty
// `intermediate_output` blob (Ethereum's decode path never does), so this is
// the fixture that exercises the `include_intermediate_output` parity seam
// over HTTP.
const SOLANA_TRANSFER_MESSAGE_B64: &str = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";

/// Spins up a `parser_http_server` instance under a private working
/// directory (so its default, non-`vsock` ephemeral-key path
/// `./local-enclave/qos.ephemeral.key` doesn't collide with other tests),
/// waits for it to bind, and returns the base URL plus the generated key.
struct RunningServer {
    base_url: String,
    ephemeral_key: P256Pair,
    _child: ChildWrapper,
    _work_dir: PathWrapper<'static>,
}

impl RunningServer {
    async fn start() -> Self {
        let test_id = format!("{:?}", rand::random::<u64>());
        // Kept as a plain `String` (not `PathWrapper`) until `RunningServer`
        // is fully constructed below: `PathWrapper`'s `Drop` deletes this
        // directory, and Rust runs `Drop` for live locals during panic
        // unwinding, so wrapping it this early would delete the directory
        // (ephemeral key, manifest fixture, any server-written artifacts) on
        // any of the `.expect()`/`panic!()` calls in the rest of this
        // function, defeating the fail-fast check below whose whole point is
        // to leave something to inspect after a startup failure.
        let work_dir = format!("./{test_id}-http-server-workdir");
        let enclave_dir = format!("{}/local-enclave", &*work_dir);
        fs::create_dir_all(&enclave_dir).expect("failed to create local-enclave dir");

        let ephemeral_key = P256Pair::generate().expect("failed to generate ephemeral key");
        ephemeral_key
            .to_hex_file(format!("{enclave_dir}/qos.ephemeral.key"))
            .expect("failed to write ephemeral key");

        // `StaticBootProof::from_enclave_files` now fails closed when the
        // manifest is missing, so the spawned server needs a real (if
        // otherwise empty) one at its dev-mode path. `ManifestEnvelope`'s
        // `Default` impl is gated on qos_core's `mock` feature, which this
        // crate (unlike parser_http_server itself) enables.
        let manifest_envelope = qos_core::protocol::services::boot::ManifestEnvelope::default();
        let manifest_json =
            serde_json::to_vec(&manifest_envelope).expect("failed to encode manifest fixture");
        fs::write(format!("{enclave_dir}/qos.manifest"), manifest_json)
            .expect("failed to write manifest fixture");

        let port = find_free_port().expect("no free port available");

        // Unlike the other integration tests, this one also sets
        // `current_dir` on the child (so the server's default,
        // non-`vsock` ephemeral-key path resolves under `work_dir`), and a
        // relative program path is not resolved against that new cwd, so
        // canonicalize it against the test binary's own cwd first.
        let binary = fs::canonicalize("../target/debug/parser_http_server")
            .expect("parser_http_server binary not found; run `cargo build` first");

        let mut child = Command::new(binary)
            .arg("--port")
            .arg(port.to_string())
            .arg("--accept-unsigned-abis")
            .current_dir(&*work_dir)
            .spawn()
            .expect("failed to spawn parser_http_server");

        // Fail fast if the server died instead of binding. `wait_until_port_is_bound`
        // only times out after its internal wait loop's linear backoff runs out, a
        // wall-clock ceiling on the order of two hours (not the 90s that the loop's
        // upper bound alone would suggest), with a generic panic message; without
        // this check a server that panics at startup would still fail the test,
        // just much slower and without saying why. That is
        // exactly what happens when the binary was built with the `vsock` feature:
        // `EPHEMERAL_KEY_FILE` becomes the absolute in-enclave path, the key we wrote
        // under `work_dir` is invisible, and the process exits before it ever listens.
        let mut observed_bind = false;
        for _ in 0..100 {
            if let Some(status) = child.try_wait().expect("failed to poll server status") {
                panic!(
                    "parser_http_server exited before binding to port {port} (status {status}). \
                     If the binary was built with --features vsock, rebuild without it: \
                     the test relies on the dev ephemeral-key path."
                );
            }
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                observed_bind = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // A child that stays alive without ever accepting a connection is not
        // covered by the panic above (it never exited). Without this check,
        // execution falls through into `wait_until_port_is_bound`, whose
        // linear backoff can run for the ~2 hours noted above before it
        // panics, stalling CI instead of failing fast within this loop's own
        // ~5s budget (100 * 50ms).
        if !observed_bind {
            // `child` is still a raw `std::process::Child` here (not yet wrapped
            // in `ChildWrapper`), and `std::process::Child`'s `Drop` does not
            // kill the process. Without an explicit kill, panicking here would
            // unwind past this still-alive server and leak it on the runner.
            let _ = child.kill();
            panic!(
                "parser_http_server did not bind to port {port} within the 5s poll budget \
                 (process is still alive). If the binary was built with --features vsock, \
                 rebuild without it: the test relies on the dev ephemeral-key path."
            );
        }

        let child: ChildWrapper = child.into();
        wait_until_port_is_bound(port);
        // wait_until_port_is_bound only proves the port is no longer free;
        // give the server a brief moment to finish accepting connections.
        tokio::time::sleep(Duration::from_millis(200)).await;

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            ephemeral_key,
            _child: child,
            _work_dir: work_dir.into(),
        }
    }
}

fn boot_proof_keys(value: &serde_json::Value) -> Vec<String> {
    value
        .get("bootProof")
        .and_then(|v| v.as_object())
        .expect("response missing bootProof object")
        .keys()
        .cloned()
        .collect()
}

async fn assert_boot_proof_response(
    resp: reqwest::Response,
    status: reqwest::StatusCode,
    expected_keys: &[String],
) -> serde_json::Value {
    assert_eq!(resp.status(), status);
    let value: serde_json::Value = resp.json().await.expect("response was not valid JSON");
    let mut keys = boot_proof_keys(&value);
    keys.sort();
    assert_eq!(keys, expected_keys);
    value
}

#[tokio::test]
async fn http_server_serves_health_parse_and_errors() {
    let server = RunningServer::start().await;
    // reqwest::Client::new() has no default request timeout, so a server
    // that accepts a connection but never responds would otherwise hang
    // this test indefinitely instead of failing it.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build reqwest client");

    // 1. GET /health returns 200.
    let health = client
        .get(format!("{}/health", server.base_url))
        .send()
        .await
        .expect("health request failed");
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    let body = serde_json::json!({
        "request": {
            "chain": "CHAIN_ETHEREUM",
            "unsigned_payload": ETH_TX_HEX,
        }
    });

    // 2. v1 parse succeeds, signature.publicKey matches the generated key,
    //    and bootProof has exactly six keys.
    let v1 = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .json(&body)
        .send()
        .await
        .expect("v1 request failed");
    assert_eq!(v1.status(), reqwest::StatusCode::OK);
    let v1_value: serde_json::Value = v1.json().await.expect("v1 response was not valid JSON");

    let expected_pubkey_hex = qos_hex::encode(&server.ephemeral_key.public_key().to_bytes());
    let v1_pubkey = v1_value
        .get("response")
        .and_then(|r| r.get("parsedTransaction"))
        .and_then(|t| t.get("signature"))
        .and_then(|s| s.get("publicKey"))
        .and_then(|v| v.as_str())
        .expect("v1 response missing signature.publicKey");
    assert_eq!(v1_pubkey, expected_pubkey_hex);

    let mut v1_boot_proof_keys = boot_proof_keys(&v1_value);
    v1_boot_proof_keys.sort();
    let mut expected_keys = vec![
        "awsAttestationDocB64".to_string(),
        "qosManifestB64".to_string(),
        "qosManifestEnvelopeB64".to_string(),
        "ephemeralPublicKeyHex".to_string(),
        "enclaveApp".to_string(),
        "deploymentLabel".to_string(),
    ];
    expected_keys.sort();
    assert_eq!(v1_boot_proof_keys, expected_keys);

    // 3. v2 behaves identically to v1 (open in this PR).
    let v2 = client
        .post(format!("{}/visualsign/api/v2/parse", server.base_url))
        .json(&body)
        .send()
        .await
        .expect("v2 request failed");
    assert_eq!(v2.status(), reqwest::StatusCode::OK);
    let v2_value: serde_json::Value = v2.json().await.expect("v2 response was not valid JSON");
    assert_eq!(v1_value, v2_value);

    // 3.5. `include_intermediate_output: true` on a Solana request that
    //    actually decodes a transfer emits a non-empty, camelCase
    //    `intermediateOutput` field. The default-false case is already
    //    covered by step 2/3 above (Ethereum, flag omitted); this covers the
    //    opt-in branch so the parity seam can't regress silently.
    let solana_tx = visualsign_solana::utils::create_transaction_with_empty_signatures(
        SOLANA_TRANSFER_MESSAGE_B64,
    );
    let solana_body = serde_json::json!({
        "request": {
            "chain": "CHAIN_SOLANA",
            "unsigned_payload": solana_tx,
            "include_intermediate_output": true,
        }
    });
    let solana_resp = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .json(&solana_body)
        .send()
        .await
        .expect("solana request failed");
    assert_eq!(solana_resp.status(), reqwest::StatusCode::OK);
    let solana_value: serde_json::Value = solana_resp
        .json()
        .await
        .expect("solana response was not valid JSON");
    let intermediate_output_b64 = solana_value
        .get("response")
        .and_then(|r| r.get("parsedTransaction"))
        .and_then(|t| t.get("payload"))
        .and_then(|p| p.get("intermediateOutput"))
        .and_then(|v| v.as_str())
        .expect("solana response missing payload.intermediateOutput");
    assert!(
        !intermediate_output_b64.is_empty(),
        "intermediateOutput must be non-empty when include_intermediate_output is true \
         and the chain decode succeeds"
    );

    // 4. A malformed body returns 400 with a bootProof present.
    let malformed = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .expect("malformed request failed");
    assert_boot_proof_response(malformed, reqwest::StatusCode::BAD_REQUEST, &expected_keys).await;

    // 5. An unmatched route still returns bootProof (axum's default 404
    //    rejection would otherwise bypass the Turnkey envelope entirely).
    let not_found = client
        .get(format!("{}/not-a-real-route", server.base_url))
        .send()
        .await
        .expect("not-found request failed");
    assert_boot_proof_response(not_found, reqwest::StatusCode::NOT_FOUND, &expected_keys).await;

    // 6. A disallowed method on a real route still returns bootProof (axum's
    //    default 405 rejection would otherwise bypass the envelope too).
    let wrong_method = client
        .get(format!("{}/visualsign/api/v1/parse", server.base_url))
        .send()
        .await
        .expect("wrong-method request failed");
    assert_boot_proof_response(
        wrong_method,
        reqwest::StatusCode::METHOD_NOT_ALLOWED,
        &expected_keys,
    )
    .await;

    // 7. A body over the 64 KiB `PIVOT_BODY_LIMIT_BYTES` cap returns 413 with
    //    bootProof (axum's `DefaultBodyLimit` rejection would otherwise bypass
    //    the Turnkey envelope, same gap as the 404/405 cases above, but
    //    handled by `envelope_body_limit_rejection` instead of a fallback
    //    since axum rejects the body before any handler or route-miss fires).
    let oversized_body = vec![b'a'; 65 * 1024];
    let too_large = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .header("content-type", "application/json")
        .body(oversized_body)
        .send()
        .await
        .expect("oversized request failed");
    let too_large_value = assert_boot_proof_response(
        too_large,
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        &expected_keys,
    )
    .await;
    assert_eq!(too_large_value.get("error").unwrap(), "payload too large");
}
