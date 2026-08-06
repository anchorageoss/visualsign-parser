#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::fs;
use std::process::Command;
use std::time::Duration;

use integration::{ChildWrapper, find_free_port, wait_until_port_is_bound};
use qos_p256::P256Pair;
use turnkey_api_key_stamper::{Stamp, TurnkeyP256ApiKey};

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
        Self::start_with_args(&[]).await
    }

    /// Same as `start`, but with extra CLI args appended (e.g.
    /// `--allowed-stamp-pubkeys-hex <csv>`).
    async fn start_with_args(extra_args: &[&str]) -> Self {
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
            .args(extra_args)
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

#[tokio::test]
async fn http_server_serves_health_parse_and_errors() {
    let server = RunningServer::start().await;
    let client = reqwest::Client::new();

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
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    let malformed_value: serde_json::Value = malformed
        .json()
        .await
        .expect("malformed response was not valid JSON");
    assert!(
        malformed_value.get("bootProof").is_some(),
        "error response must still carry bootProof"
    );
    let mut malformed_keys = boot_proof_keys(&malformed_value);
    malformed_keys.sort();
    assert_eq!(malformed_keys, expected_keys);
}

#[tokio::test]
async fn http_server_enforces_x_stamp_when_allowlist_is_configured() {
    let allowed = TurnkeyP256ApiKey::generate();
    let other = TurnkeyP256ApiKey::generate();
    let allowlist_hex = hex::encode(allowed.compressed_public_key());

    let server =
        RunningServer::start_with_args(&["--allowed-stamp-pubkeys-hex", &allowlist_hex]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "request": {
            "chain": "CHAIN_ETHEREUM",
            "unsigned_payload": ETH_TX_HEX,
        }
    });
    // Sign the exact bytes sent over the wire, not a re-serialized copy:
    // this is what the pivot verifies against.
    let raw_body = serde_json::to_vec(&body).expect("failed to serialize body");

    // 1. No X-Stamp header: 401 with bootProof present.
    let unstamped = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .header("content-type", "application/json")
        .body(raw_body.clone())
        .send()
        .await
        .expect("unstamped request failed");
    assert_eq!(unstamped.status(), reqwest::StatusCode::UNAUTHORIZED);
    let unstamped_value: serde_json::Value = unstamped
        .json()
        .await
        .expect("unstamped response was not valid JSON");
    assert!(
        unstamped_value.get("bootProof").is_some(),
        "401 response must still carry bootProof"
    );

    // 2. Stamped by a listed key: 200.
    let stamp = allowed.stamp(&raw_body).expect("failed to stamp body");
    let listed = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .header("content-type", "application/json")
        .header(stamp.name, stamp.value)
        .body(raw_body.clone())
        .send()
        .await
        .expect("listed-key request failed");
    assert_eq!(listed.status(), reqwest::StatusCode::OK);

    // 3. Stamped by an unlisted key: 401.
    let other_stamp = other.stamp(&raw_body).expect("failed to stamp body");
    let unlisted = client
        .post(format!("{}/visualsign/api/v1/parse", server.base_url))
        .header("content-type", "application/json")
        .header(other_stamp.name, other_stamp.value)
        .body(raw_body)
        .send()
        .await
        .expect("unlisted-key request failed");
    assert_eq!(unlisted.status(), reqwest::StatusCode::UNAUTHORIZED);
}
