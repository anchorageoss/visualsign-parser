//! THROWAWAY diagnostic pivot — not for production, not meant to merge to
//! `main` long-term. Delete the TVC deployment once the probe result is
//! recorded, and close/delete this branch afterward.
//!
//! Exists to answer the one question PR #452 (`NsmBootProof`) is explicitly
//! blocked on: "whether a pivot process can reach `/dev/nsm` under TVC, and
//! whether concurrent use alongside qos_core is safe, has not been verified
//! against a real deployment." This binary makes the exact same
//! `NsmRequest::Attestation` call #452 makes — manifest hash in `user_data`,
//! ephemeral pubkey in `public_key`, `nonce: None` — from a real TVC pivot,
//! with no other surface: it doesn't parse transactions, doesn't serve
//! wallet traffic, and carries no ABI-trust posture.
//!
//! Routes:
//! - `GET /health` — 200 OK, for TVC's HTTP health check.
//! - `GET /probe`  — runs a fresh `/dev/nsm` attestation call right now and
//!   reports the result as JSON. Deliberately not cached: hit it a few times
//!   over the deployment's lifetime (not just once at boot) to catch a
//!   conflict with qos_core's own concurrent NSM use, not just prove the
//!   call worked once.
//!
//! Built against QOS 0.12.1 (what TVC boots): the manifest is decoded as
//! `VersionedManifestEnvelope` and `user_data` is its `manifest_hash()`
//! (canonical-JSON hash for v2, `qos_hash` for v0/v1) -- byte-identical to
//! qos_core's own post-boot attestation. Both the manifest and the ephemeral
//! key are re-read on every call, since 0.12.1 rotates the setup ephemeral key
//! to the live one after provisioning.
//!
//! Configuration: `--port <u16>` / `HTTP_PORT` (default 3000), matching
//! `parser_http_server`'s convention.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use base64::Engine as _;
use clap::Parser;
use qos_core::handles::EphemeralKeyHandle;
use qos_core::protocol::services::boot::VersionedManifestEnvelope;
use qos_nsm::NsmProvider;
use qos_nsm::types::{NsmRequest, NsmResponse};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Upper bound on one `/dev/nsm` call, so a hung device call reports as an
/// error instead of an HTTP request that never returns.
const NSM_CALL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Parser, Debug)]
struct Args {
    /// HTTP port to listen on.
    #[arg(long, env = "HTTP_PORT", default_value_t = 3000)]
    port: u16,
}

struct AppState {
    attempt: AtomicU64,
}

/// Inputs for one attestation call, read fresh from the qos_core-owned files.
struct ProbeInputs {
    ephemeral_public_key: Vec<u8>,
    user_data: Vec<u8>,
    /// Where `user_data` came from, so a reader can tell a real manifest hash
    /// from the fallback marker.
    user_data_source: String,
    /// `v0` / `v1` / `v2`, or `None` if the manifest couldn't be decoded.
    manifest_version: Option<&'static str>,
}

#[derive(Serialize)]
struct ProbeResult {
    ok: bool,
    error: Option<String>,
    attempt: u64,
    latency_ms: u128,
    document_len: usize,
    aws_attestation_doc_b64: String,
    ephemeral_public_key_hex: String,
    user_data_source: String,
    manifest_version: Option<&'static str>,
    manifest_hash_hex: Option<String>,
}

fn load_inputs() -> Result<ProbeInputs, String> {
    let ephemeral_key = EphemeralKeyHandle::new(qos_core::EPHEMERAL_KEY_FILE.to_string())
        .get_ephemeral_key()
        .map_err(|e| {
            format!(
                "failed to load ephemeral key from {}: {e:?}",
                qos_core::EPHEMERAL_KEY_FILE
            )
        })?;
    let ephemeral_public_key = ephemeral_key.public_key().to_bytes();

    let decoded = std::fs::read(qos_core::MANIFEST_FILE)
        .map_err(|e| format!("{} unreadable: {e}", qos_core::MANIFEST_FILE))
        .and_then(|bytes| {
            VersionedManifestEnvelope::try_from_slice_compat(&bytes)
                .map_err(|e| format!("{} undecodable: {e}", qos_core::MANIFEST_FILE))
        });

    Ok(match decoded {
        Ok(envelope) => {
            let manifest_version = match &envelope {
                VersionedManifestEnvelope::V2(_) => "v2",
                VersionedManifestEnvelope::V1(_) => "v1",
                VersionedManifestEnvelope::V0(_) => "v0",
            };
            ProbeInputs {
                ephemeral_public_key,
                user_data: envelope.manifest_hash().to_vec(),
                user_data_source: format!("real qos.manifest {manifest_version} manifest_hash()"),
                manifest_version: Some(manifest_version),
            }
        }
        Err(e) => {
            eprintln!("nsm_probe: {e}, using fixed probe user_data");
            ProbeInputs {
                ephemeral_public_key,
                user_data: b"nsm-probe-fixed-user-data".to_vec(),
                user_data_source: format!("fixed fallback ({e})"),
                manifest_version: None,
            }
        }
    })
}

/// Makes one real attestation call, generic over the attestor so tests can
/// substitute a mock without touching `/dev/nsm`.
fn run_probe<A: NsmProvider>(attestor: &A, inputs: &ProbeInputs, attempt: u64) -> ProbeResult {
    let start = Instant::now();
    let response = attestor.nsm_process_request(NsmRequest::Attestation {
        user_data: Some(inputs.user_data.clone()),
        nonce: None,
        public_key: Some(inputs.ephemeral_public_key.clone()),
    });
    let latency_ms = start.elapsed().as_millis();

    let (ok, error, document) = match response {
        NsmResponse::Attestation { document } => (true, None, document),
        other => (false, Some(format!("{other:?}")), Vec::new()),
    };

    ProbeResult {
        ok,
        error,
        attempt,
        latency_ms,
        document_len: document.len(),
        aws_attestation_doc_b64: base64::engine::general_purpose::STANDARD.encode(&document),
        ephemeral_public_key_hex: qos_hex::encode(&inputs.ephemeral_public_key),
        user_data_source: inputs.user_data_source.clone(),
        manifest_version: inputs.manifest_version,
        manifest_hash_hex: inputs
            .manifest_version
            .map(|_| qos_hex::encode(&inputs.user_data)),
    }
}

fn failed(attempt: u64, error: String) -> ProbeResult {
    ProbeResult {
        ok: false,
        error: Some(error),
        attempt,
        latency_ms: 0,
        document_len: 0,
        aws_attestation_doc_b64: String::new(),
        ephemeral_public_key_hex: String::new(),
        user_data_source: String::new(),
        manifest_version: None,
        manifest_hash_hex: None,
    }
}

/// Loads fresh inputs and makes the `/dev/nsm` call off the async runtime,
/// bounded by [`NSM_CALL_TIMEOUT`].
async fn probe_once(state: &AppState) -> ProbeResult {
    let attempt = state.attempt.fetch_add(1, Ordering::SeqCst) + 1;
    let task = tokio::task::spawn_blocking(move || {
        load_inputs().map(|inputs| run_probe(&qos_nsm::Nsm, &inputs, attempt))
    });
    let result = match tokio::time::timeout(NSM_CALL_TIMEOUT, task).await {
        Ok(Ok(Ok(result))) => result,
        Ok(Ok(Err(e))) => failed(attempt, e),
        Ok(Err(e)) => failed(attempt, format!("probe task failed: {e}")),
        Err(_) => failed(
            attempt,
            format!("/dev/nsm call exceeded {NSM_CALL_TIMEOUT:?}"),
        ),
    };
    eprintln!(
        "nsm_probe: attempt={} ok={} latency_ms={} document_len={} manifest_version={:?} manifest_hash={:?} ephemeral_pub={} error={:?}",
        result.attempt,
        result.ok,
        result.latency_ms,
        result.document_len,
        result.manifest_version,
        result.manifest_hash_hex,
        result.ephemeral_public_key_hex,
        result.error
    );
    result
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn probe(State(state): State<Arc<AppState>>) -> Json<ProbeResult> {
    Json(probe_once(&state).await)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let state = Arc::new(AppState {
        attempt: AtomicU64::new(0),
    });

    eprintln!("nsm_probe: running boot-time probe...");
    probe_once(&state).await;

    let app = Router::new()
        .route("/health", get(health))
        .route("/probe", get(probe))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    eprintln!("nsm_probe listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(e) => {
                eprintln!("failed to register SIGTERM handler: {e}; falling back to ctrl-c only");
                if let Err(e) = ctrl_c.await {
                    eprintln!("failed to listen for ctrl-c: {e}");
                }
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = ctrl_c.await {
        eprintln!("failed to listen for ctrl-c: {e}");
    }
    eprintln!("nsm_probe shutting down");
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use qos_nsm::nitro::AttestError;

    struct MockAttestor {
        document: Vec<u8>,
    }

    impl NsmProvider for MockAttestor {
        fn nsm_process_request(&self, request: NsmRequest) -> NsmResponse {
            match request {
                NsmRequest::Attestation { .. } => NsmResponse::Attestation {
                    document: self.document.clone(),
                },
                other => panic!("unexpected NSM request in test: {other:?}"),
            }
        }

        fn timestamp_ms(&self) -> Result<u64, AttestError> {
            Ok(0)
        }
    }

    fn test_inputs(manifest_version: Option<&'static str>) -> ProbeInputs {
        ProbeInputs {
            ephemeral_public_key: vec![0xAB; 33],
            user_data: b"test-user-data".to_vec(),
            user_data_source: "test fixture".to_string(),
            manifest_version,
        }
    }

    #[test]
    fn run_probe_reports_success_and_document_len() {
        let attestor = MockAttestor {
            document: vec![0xCC; 42],
        };
        let result = run_probe(&attestor, &test_inputs(None), 1);
        assert!(result.ok);
        assert!(result.error.is_none());
        assert_eq!(result.document_len, 42);
        assert_eq!(result.attempt, 1);
        assert!(!result.aws_attestation_doc_b64.is_empty());
        assert_eq!(
            result.ephemeral_public_key_hex,
            qos_hex::encode(&[0xAB; 33])
        );
    }

    #[test]
    fn manifest_hash_reported_only_for_decoded_manifest() {
        let attestor = MockAttestor {
            document: vec![0xCC; 4],
        };
        let fallback = run_probe(&attestor, &test_inputs(None), 1);
        assert!(fallback.manifest_hash_hex.is_none());

        let real = run_probe(&attestor, &test_inputs(Some("v2")), 2);
        assert_eq!(real.manifest_version, Some("v2"));
        assert_eq!(
            real.manifest_hash_hex.as_deref(),
            Some(qos_hex::encode(b"test-user-data").as_str())
        );
    }
}
