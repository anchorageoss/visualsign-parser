//! THROWAWAY soak-test pivot for `tvc-attestation` — not for production.
//! Delete the TVC deployment once the soak is recorded.
//!
//! A thin TVC pivot over [`tvc_attestation::cache::AttestationCache`], run for
//! days on a real deployment to confirm, before #452 depends on it, that the
//! watcher re-attests near certificate expiry (~every 2.5h with the default
//! 30m margin on the ~3h NSM leaf), follows the setup -> live ephemeral key
//! rotation, and keeps the replica healthy throughout.
//!
//! Routes:
//! - `GET /health` — 200 while a verified, unexpired attestation is cached,
//!   503 otherwise (a replica whose first NSM call fails never goes healthy).
//! - `GET /proof`  — the cached attestation (what #452 will serve).
//! - `GET /status` — uptime, refresh/failure counters, and the current doc's
//!   NSM timestamp and remaining validity. Polled externally: `near_expiry`
//!   rising and `doc_timestamp_ms` advancing show certs rotating.
//!
//! Configuration: `--port` / `HTTP_PORT` (default 3000), `--watch-interval-secs`
//! (default 10), `--refresh-margin-secs` (default 1800).

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use base64::Engine as _;
use clap::Parser;
use serde::Serialize;
use tvc_attestation::cache::{AttestationCache, CacheStats, InputLoader, load_inputs};
use tvc_attestation::paths;

#[derive(Parser, Debug)]
struct Args {
    /// HTTP port to listen on.
    #[arg(long, env = "HTTP_PORT", default_value_t = 3000)]
    port: u16,
    /// How often the watcher re-checks the ephemeral key, manifest and expiry.
    #[arg(long, env = "WATCH_INTERVAL_SECS", default_value_t = 10)]
    watch_interval_secs: u64,
    /// Re-attest once the certificate chain has this little validity left.
    #[arg(long, env = "REFRESH_MARGIN_SECS", default_value_t = 30 * 60)]
    refresh_margin_secs: u64,
}

struct AppState {
    cache: Arc<AttestationCache<qos_nsm::Nsm>>,
    started: Instant,
}

#[derive(Serialize)]
struct Proof {
    aws_attestation_doc_b64: String,
    ephemeral_public_key_hex: String,
    manifest_hash_hex: String,
    doc_timestamp_ms: u64,
    cert_not_after_unix: u64,
    cert_seconds_left: u64,
}

#[derive(Serialize)]
struct Status {
    uptime_secs: u64,
    healthy: bool,
    refreshes: Refreshes,
    failures: u64,
    last_error: Option<String>,
    /// `None` if there's no valid attestation right now.
    current: Option<Current>,
}

#[derive(Serialize)]
struct Refreshes {
    initial: u64,
    ephemeral_key_changed: u64,
    manifest_changed: u64,
    near_expiry: u64,
}

#[derive(Serialize)]
struct Current {
    doc_timestamp_ms: u64,
    cert_not_after_unix: u64,
    cert_seconds_left: u64,
    ephemeral_public_key_hex: String,
    manifest_hash_hex: String,
}

async fn health(State(state): State<Arc<AppState>>) -> StatusCode {
    if state.cache.healthy() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn proof(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.cache.get().await {
        Ok((a, _)) => Json(Proof {
            aws_attestation_doc_b64: base64::engine::general_purpose::STANDARD.encode(&a.document),
            ephemeral_public_key_hex: qos_hex::encode(&a.inputs.ephemeral_public_key),
            manifest_hash_hex: qos_hex::encode(&a.inputs.manifest_hash),
            doc_timestamp_ms: a.cert.timestamp_ms,
            cert_not_after_unix: a.cert.not_after_unix,
            cert_seconds_left: a.valid_for().as_secs(),
        })
        .into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    }
}

async fn status(State(state): State<Arc<AppState>>) -> Json<Status> {
    // `get` is a cheap cache hit unless a refresh is due, which is fine here.
    let current = state.cache.get().await.ok().map(|(a, _)| Current {
        doc_timestamp_ms: a.cert.timestamp_ms,
        cert_not_after_unix: a.cert.not_after_unix,
        cert_seconds_left: a.valid_for().as_secs(),
        ephemeral_public_key_hex: qos_hex::encode(&a.inputs.ephemeral_public_key),
        manifest_hash_hex: qos_hex::encode(&a.inputs.manifest_hash),
    });
    let CacheStats {
        initial,
        ephemeral_key_changed,
        manifest_changed,
        near_expiry,
        failures,
        last_error,
    } = state.cache.stats();
    Json(Status {
        uptime_secs: state.started.elapsed().as_secs(),
        healthy: state.cache.healthy(),
        refreshes: Refreshes {
            initial,
            ephemeral_key_changed,
            manifest_changed,
            near_expiry,
        },
        failures,
        last_error,
        current,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let load: InputLoader =
        Arc::new(|| load_inputs(paths::EPHEMERAL_KEY_FILE, Path::new(paths::MANIFEST_FILE)));
    let cache = Arc::new(
        AttestationCache::new(Arc::new(qos_nsm::Nsm), load)
            .with_refresh_margin(Duration::from_secs(args.refresh_margin_secs)),
    );

    // Attest before serving; if this fails, /health stays 503 and the
    // watcher keeps retrying.
    match cache.get().await {
        Ok((a, _)) => eprintln!(
            "nsm_probe: boot attestation ok; cert valid {}s (notAfter {}), nsm {}us",
            a.cert.remaining.as_secs(),
            a.cert.not_after_unix,
            a.nsm_latency.as_micros()
        ),
        Err(e) => eprintln!("nsm_probe: boot attestation failed: {e}"),
    }
    let watcher = cache.spawn_watcher(Duration::from_secs(args.watch_interval_secs));

    let state = Arc::new(AppState {
        cache,
        started: Instant::now(),
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/proof", get(proof))
        .route("/status", get(status))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    eprintln!("nsm_probe listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::select! {
        served = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()) => served?,
        exited = watcher => eprintln!("nsm_probe: attestation watcher exited: {exited:?}"),
    }
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
