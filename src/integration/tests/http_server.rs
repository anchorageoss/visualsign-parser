#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::fs;
use std::process::Command;
use std::time::Duration;

use integration::{ChildWrapper, find_free_port, wait_until_port_is_bound};
use qos_p256::P256Pair;

// Same Ethereum signed legacy transaction used by
// `integration::tests::parser_ethereum_native_transfer_e2e`.
const ETH_TX_HEX: &str = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

/// Spins up a `parser_http_server` instance under a private working
/// directory (so its default, non-`vsock` ephemeral-key path
/// `./local-enclave/qos.ephemeral.key` doesn't collide with other tests),
/// waits for it to bind, and returns the base URL plus the generated key.
struct RunningServer {
    base_url: String,
    ephemeral_key: P256Pair,
    _child: ChildWrapper,
    work_dir: String,
}

impl RunningServer {
    async fn start() -> Self {
        let test_id = format!("{:?}", rand::random::<u64>());
        let work_dir = format!("./{test_id}-http-server-workdir");
        let enclave_dir = format!("{work_dir}/local-enclave");
        fs::create_dir_all(&enclave_dir).expect("failed to create local-enclave dir");

        let ephemeral_key = P256Pair::generate().expect("failed to generate ephemeral key");
        ephemeral_key
            .to_hex_file(format!("{enclave_dir}/qos.ephemeral.key"))
            .expect("failed to write ephemeral key");

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
            .current_dir(&work_dir)
            .spawn()
            .expect("failed to spawn parser_http_server");

        // Fail fast if the server died instead of binding. `wait_until_port_is_bound`
        // polls forever, so without this check a server that panics at startup hangs
        // the test run rather than failing it. That is exactly what happens when the
        // binary was built with the `vsock` feature: `EPHEMERAL_KEY_FILE` becomes the
        // absolute in-enclave path, the key we wrote under `work_dir` is invisible,
        // and the process exits before it ever listens.
        for _ in 0..100 {
            if let Some(status) = child.try_wait().expect("failed to poll server status") {
                panic!(
                    "parser_http_server exited before binding to port {port} (status {status}). \
                     If the binary was built with --features vsock, rebuild without it: \
                     the test relies on the dev ephemeral-key path."
                );
            }
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
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
            work_dir,
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.work_dir));
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

/// Asserts `resp` has `status` and carries exactly `expected_keys` under
/// `bootProof`; returns the parsed body for any further per-case checks.
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
