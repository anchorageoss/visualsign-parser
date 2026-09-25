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
//! Configuration: `--port <u16>` / `HTTP_PORT` (default 3000), matching
//! `parser_http_server`'s convention.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use base64::Engine as _;
use clap::Parser;
use qos_core::handles::EphemeralKeyHandle;
use qos_core::protocol::QosHash;
use qos_core::protocol::services::boot::ManifestEnvelope;
use qos_nsm::NsmProvider;
use qos_nsm::types::{NsmRequest, NsmResponse};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Parser, Debug)]
struct Args {
    /// HTTP port to listen on.
    #[arg(long, env = "HTTP_PORT", default_value_t = 3000)]
    port: u16,
}

/// Where `user_data` for the attestation call comes from. Reported in every
/// probe result so a human reading it can tell whether this ran against a
/// real provisioned manifest or the throwaway fallback (e.g. run outside a
/// real TVC deployment, or before QOS has provisioned `/qos.manifest` yet).
struct AppState {
    ephemeral_public_key: Vec<u8>,
    ephemeral_public_key_hex: String,
    user_data: Vec<u8>,
    user_data_source: &'static str,
    attempt: AtomicU64,
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
    user_data_source: &'static str,
}

/// Makes one real attestation call, generic over the attestor so tests can
/// substitute a mock without touching `/dev/nsm`.
fn run_probe<A: NsmProvider>(attestor: &A, state: &AppState) -> ProbeResult {
    let attempt = state.attempt.fetch_add(1, Ordering::SeqCst) + 1;
    let start = Instant::now();
    let response = attestor.nsm_process_request(NsmRequest::Attestation {
        user_data: Some(state.user_data.clone()),
        nonce: None,
        public_key: Some(state.ephemeral_public_key.clone()),
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
        ephemeral_public_key_hex: state.ephemeral_public_key_hex.clone(),
        user_data_source: state.user_data_source,
    }
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn probe(State(state): State<Arc<AppState>>) -> Json<ProbeResult> {
    let result = run_probe(&qos_nsm::Nsm, &state);
    eprintln!(
        "nsm_probe: attempt={} ok={} latency_ms={} document_len={} error={:?}",
        result.attempt, result.ok, result.latency_ms, result.document_len, result.error
    );
    Json(result)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let handle = EphemeralKeyHandle::new(qos_core::EPHEMERAL_KEY_FILE.to_string());
    let ephemeral_key = handle.get_ephemeral_key().map_err(|e| {
        format!(
            "failed to load ephemeral key from {}: {e}",
            qos_core::EPHEMERAL_KEY_FILE
        )
    })?;
    let ephemeral_public_key = ephemeral_key.public_key().to_bytes();
    let ephemeral_public_key_hex = qos_hex::encode(&ephemeral_public_key);

    // Best-effort: use the real provisioned manifest's hash as `user_data`
    // (byte-identical to what #452 will send) when it's readable, otherwise
    // fall back to a fixed marker so the probe still runs and clearly labels
    // itself as not using real manifest data.
    let (user_data, user_data_source) = match std::fs::read(qos_core::MANIFEST_FILE) {
        Ok(bytes) => match serde_json::from_slice::<ManifestEnvelope>(&bytes) {
            Ok(envelope) => (
                envelope.manifest.qos_hash().to_vec(),
                "real qos.manifest hash",
            ),
            Err(e) => {
                eprintln!(
                    "nsm_probe: {} unparseable ({e}), using fixed probe user_data",
                    qos_core::MANIFEST_FILE
                );
                (
                    b"nsm-probe-fixed-user-data".to_vec(),
                    "fixed fallback (manifest unparseable)",
                )
            }
        },
        Err(e) => {
            eprintln!(
                "nsm_probe: {} unreadable ({e}), using fixed probe user_data",
                qos_core::MANIFEST_FILE
            );
            (
                b"nsm-probe-fixed-user-data".to_vec(),
                "fixed fallback (manifest unreadable)",
            )
        }
    };

    let state = Arc::new(AppState {
        ephemeral_public_key,
        ephemeral_public_key_hex,
        user_data,
        user_data_source,
        attempt: AtomicU64::new(0),
    });

    eprintln!("nsm_probe: running boot-time probe...");
    let boot_result = run_probe(&qos_nsm::Nsm, &state);
    eprintln!(
        "nsm_probe: boot-time probe result: ok={} latency_ms={} document_len={} error={:?}",
        boot_result.ok, boot_result.latency_ms, boot_result.document_len, boot_result.error
    );

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

    fn test_state() -> AppState {
        AppState {
            ephemeral_public_key: vec![0xAB; 33],
            ephemeral_public_key_hex: qos_hex::encode(&[0xAB; 33]),
            user_data: b"test-user-data".to_vec(),
            user_data_source: "test fixture",
            attempt: AtomicU64::new(0),
        }
    }

    #[test]
    fn run_probe_reports_success_and_document_len() {
        let state = test_state();
        let attestor = MockAttestor {
            document: vec![0xCC; 42],
        };
        let result = run_probe(&attestor, &state);
        assert!(result.ok);
        assert!(result.error.is_none());
        assert_eq!(result.document_len, 42);
        assert_eq!(result.attempt, 1);
        assert!(!result.aws_attestation_doc_b64.is_empty());
    }

    #[test]
    fn run_probe_increments_attempt_across_calls() {
        let state = test_state();
        let attestor = MockAttestor {
            document: vec![0xCC; 4],
        };
        let first = run_probe(&attestor, &state);
        let second = run_probe(&attestor, &state);
        assert_eq!(first.attempt, 1);
        assert_eq!(second.attempt, 2);
    }
}
