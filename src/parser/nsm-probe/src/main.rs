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
//! - `GET /health` — 200 only while a successful attestation is cached and
//!   younger than the ~3h lifetime of the doc's NSM-issued certificate; 503
//!   otherwise. So a replica whose first `/dev/nsm` call fails never goes
//!   healthy.
//! - `GET /probe`  — runs a fresh `/dev/nsm` attestation call right now and
//!   reports the result as JSON. Deliberately not cached: hit it a few times
//!   over the deployment's lifetime (not just once at boot) to catch a
//!   conflict with qos_core's own concurrent NSM use, not just prove the
//!   call worked once.
//! - `GET /proof`  — the cached attestation doc (what #452 should serve). The
//!   cache is keyed on (ephemeral pubkey, manifest hash) and guarded by one
//!   async mutex held across check-and-attest, so the watcher and concurrent
//!   requests never race duplicate NSM calls. A background watcher re-checks
//!   every `--watch-interval-secs` (default 10): it re-attests as soon as the
//!   ephemeral key or manifest changes (0.12.1 rotates the setup key to the
//!   live one) and once the doc is `--refresh-after-secs` old (default 2h),
//!   ahead of the ~3h certificate expiry.
//!
//! At startup it also benchmarks the call (`--bench-iterations`, default 50)
//! and logs min/p50/p95/max for both the file reads and the `/dev/nsm` call,
//! so the cost that `/proof` avoids is measured, not assumed.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

/// Upper bound on one `/dev/nsm` call, so a hung device call reports as an
/// error instead of an HTTP request that never returns.
const NSM_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Validity of the certificate NSM embeds in an attestation doc. A cached doc
/// older than this is expired for verifiers, so `/health` stops passing.
const ATTESTATION_CERT_LIFETIME: Duration = Duration::from_secs(3 * 60 * 60);

#[derive(Parser, Debug)]
struct Args {
    /// HTTP port to listen on.
    #[arg(long, env = "HTTP_PORT", default_value_t = 3000)]
    port: u16,

    /// Number of back-to-back `/dev/nsm` attestation calls to time at startup.
    #[arg(long, env = "BENCH_ITERATIONS", default_value_t = 50)]
    bench_iterations: usize,

    /// Re-attest once the cached doc is this old; must stay below the ~3h
    /// certificate lifetime.
    #[arg(long, env = "REFRESH_AFTER_SECS", default_value_t = 2 * 60 * 60)]
    refresh_after_secs: u64,

    /// How often the watcher re-reads the ephemeral key and manifest.
    #[arg(long, env = "WATCH_INTERVAL_SECS", default_value_t = 10)]
    watch_interval_secs: u64,
}

struct AppState {
    attempt: AtomicU64,
    /// Held across the whole check-and-attest in [`ensure_fresh`].
    cache: AsyncMutex<Option<CachedProof>>,
    /// When the cached doc was attested, mirrored outside `cache` so `/health`
    /// never waits behind an in-flight NSM call.
    attested_at: Mutex<Option<Instant>>,
    refresh_after: Duration,
}

impl AppState {
    fn set_attested_at(&self, at: Option<Instant>) {
        *self.attested_at.lock().unwrap_or_else(|e| e.into_inner()) = at;
    }

    fn healthy(&self) -> bool {
        self.attested_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|at| at.elapsed() < ATTESTATION_CERT_LIFETIME)
    }
}

/// A successful attestation plus the inputs it attested to; valid for as long
/// as the on-disk ephemeral key and manifest hash are unchanged.
struct CachedProof {
    ephemeral_public_key: Vec<u8>,
    user_data: Vec<u8>,
    attested_at: Instant,
    result: ProbeResult,
}

impl CachedProof {
    fn matches(&self, inputs: &ProbeInputs) -> bool {
        self.ephemeral_public_key == inputs.ephemeral_public_key
            && self.user_data == inputs.user_data
    }
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

#[derive(Serialize, Clone)]
struct ProbeResult {
    ok: bool,
    error: Option<String>,
    attempt: u64,
    /// Wall time of the `/dev/nsm` call that produced this document.
    latency_us: u128,
    /// True when `/proof` served this from the startup cache.
    cached: bool,
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
    let latency_us = start.elapsed().as_micros();

    let (ok, error, document) = match response {
        NsmResponse::Attestation { document } => (true, None, document),
        other => (false, Some(format!("{other:?}")), Vec::new()),
    };

    ProbeResult {
        ok,
        error,
        attempt,
        latency_us,
        cached: false,
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
        latency_us: 0,
        cached: false,
        document_len: 0,
        aws_attestation_doc_b64: String::new(),
        ephemeral_public_key_hex: String::new(),
        user_data_source: String::new(),
        manifest_version: None,
        manifest_hash_hex: None,
    }
}

/// Runs `f` on a blocking thread bounded by [`NSM_CALL_TIMEOUT`], so neither
/// a slow file read nor a hung `/dev/nsm` call stalls the async runtime.
async fn bounded<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(NSM_CALL_TIMEOUT, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(format!("probe task failed: {e}")),
        Err(_) => Err(format!("/dev/nsm call exceeded {NSM_CALL_TIMEOUT:?}")),
    }
}

fn log_result(route: &str, result: &ProbeResult) {
    eprintln!(
        "nsm_probe: route={route} attempt={} ok={} cached={} latency_us={} document_len={} manifest_version={:?} manifest_hash={:?} ephemeral_pub={} error={:?}",
        result.attempt,
        result.ok,
        result.cached,
        result.latency_us,
        result.document_len,
        result.manifest_version,
        result.manifest_hash_hex,
        result.ephemeral_public_key_hex,
        result.error
    );
}

/// Fresh inputs + a live `/dev/nsm` call. Returns the inputs too, so callers
/// can cache the result against them.
async fn attest_live(attempt: u64) -> Result<(ProbeInputs, ProbeResult), String> {
    bounded(move || {
        let inputs = load_inputs()?;
        let result = run_probe(&qos_nsm::Nsm, &inputs, attempt);
        Ok((inputs, result))
    })
    .await
}

async fn probe_once(state: &AppState) -> ProbeResult {
    let attempt = state.attempt.fetch_add(1, Ordering::SeqCst) + 1;
    let result = match attest_live(attempt).await {
        Ok((_, result)) => result,
        Err(e) => failed(attempt, e),
    };
    log_result("probe", &result);
    result
}

/// Returns the cached attestation if it still matches the on-disk inputs and
/// is younger than `refresh_after`; otherwise attests live and replaces it.
///
/// The cache lock is held across the check *and* the NSM call, so concurrent
/// callers (the watcher and `/proof` requests) single-flight one refresh
/// instead of racing duplicate calls or storing an older doc over a newer one.
async fn ensure_fresh(state: &AppState, route: &str) -> ProbeResult {
    let attempt = state.attempt.fetch_add(1, Ordering::SeqCst) + 1;
    let mut cache = state.cache.lock().await;

    let inputs = match bounded(load_inputs).await {
        Ok(inputs) => inputs,
        Err(e) => return failed(attempt, e),
    };
    let reason = match cache.as_ref() {
        None => "initial",
        Some(c) if !c.matches(&inputs) => "ephemeral key or manifest changed",
        Some(c) if c.attested_at.elapsed() >= state.refresh_after => "refresh_after elapsed",
        Some(c) => {
            let mut result = c.result.clone();
            result.cached = true;
            return result;
        }
    };
    eprintln!("nsm_probe: route={route} re-attesting: {reason}");

    let result = match attest_live(attempt).await {
        Ok((inputs, result)) if result.ok => {
            let attested_at = Instant::now();
            *cache = Some(CachedProof {
                ephemeral_public_key: inputs.ephemeral_public_key,
                user_data: inputs.user_data,
                attested_at,
                result: result.clone(),
            });
            state.set_attested_at(Some(attested_at));
            result
        }
        Ok((_, result)) => result,
        Err(e) => failed(attempt, e),
    };
    if !result.ok && cache.as_ref().is_some_and(|c| !c.matches(&inputs)) {
        // The cached doc attests a key/manifest that's no longer current;
        // don't keep serving it (or reporting healthy) after a failed refresh.
        *cache = None;
        state.set_attested_at(None);
    }
    log_result(route, &result);
    result
}

/// Re-checks the inputs every `interval` for the life of the process.
async fn watch(state: Arc<AppState>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        ensure_fresh(&state, "watch").await;
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LatencyStats {
    n: usize,
    min_us: u128,
    p50_us: u128,
    p95_us: u128,
    max_us: u128,
    mean_us: u128,
}

/// Nearest-rank percentiles over `samples` (sorted in place).
fn latency_stats(samples: &mut [u128]) -> Option<LatencyStats> {
    let n = samples.len();
    let (&min_us, &max_us) = (samples.iter().min()?, samples.iter().max()?);
    samples.sort_unstable();
    let rank = |p: usize| samples[(n * p).div_ceil(100).saturating_sub(1)];
    Some(LatencyStats {
        n,
        min_us,
        p50_us: rank(50),
        p95_us: rank(95),
        max_us,
        mean_us: samples.iter().sum::<u128>() / n as u128,
    })
}

/// Times `iterations` back-to-back input loads and `/dev/nsm` attestation
/// calls. The load cost is what a cached `/proof` hit still pays; the NSM cost
/// is what it saves.
fn run_benchmark(
    iterations: usize,
) -> Result<(Option<LatencyStats>, Option<LatencyStats>, usize), String> {
    let mut load_us = Vec::with_capacity(iterations);
    let mut nsm_us = Vec::with_capacity(iterations);
    let mut failures = 0;
    for i in 0..iterations {
        let start = Instant::now();
        let inputs = load_inputs()?;
        load_us.push(start.elapsed().as_micros());

        let result = run_probe(&qos_nsm::Nsm, &inputs, i as u64);
        if result.ok {
            nsm_us.push(result.latency_us);
        } else {
            failures += 1;
        }
    }
    Ok((
        latency_stats(&mut load_us),
        latency_stats(&mut nsm_us),
        failures,
    ))
}

async fn health(State(state): State<Arc<AppState>>) -> StatusCode {
    if state.healthy() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn probe(State(state): State<Arc<AppState>>) -> Json<ProbeResult> {
    Json(probe_once(&state).await)
}

async fn proof(State(state): State<Arc<AppState>>) -> Json<ProbeResult> {
    Json(ensure_fresh(&state, "proof").await)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let refresh_after = Duration::from_secs(args.refresh_after_secs);
    if refresh_after >= ATTESTATION_CERT_LIFETIME {
        return Err(format!(
            "--refresh-after-secs must be below the {ATTESTATION_CERT_LIFETIME:?} certificate lifetime"
        )
        .into());
    }
    let state = Arc::new(AppState {
        attempt: AtomicU64::new(0),
        cache: AsyncMutex::new(None),
        attested_at: Mutex::new(None),
        refresh_after,
    });

    let iterations = args.bench_iterations;
    eprintln!("nsm_probe: benchmarking {iterations} attestation calls...");
    // The whole benchmark, not one call, so the timeout doesn't apply here.
    match tokio::task::spawn_blocking(move || run_benchmark(iterations)).await {
        Ok(Ok((load, nsm, failures))) => {
            eprintln!("nsm_probe: bench load_inputs {load:?}");
            eprintln!("nsm_probe: bench nsm_attestation {nsm:?} failures={failures}");
        }
        Ok(Err(e)) => eprintln!("nsm_probe: bench aborted: {e}"),
        Err(e) => eprintln!("nsm_probe: bench task failed: {e}"),
    }

    // Seed the cache before serving; if this fails, /health stays 503 and the
    // watcher keeps retrying.
    ensure_fresh(&state, "boot").await;
    tokio::spawn(watch(
        Arc::clone(&state),
        Duration::from_secs(args.watch_interval_secs),
    ));

    let app = Router::new()
        .route("/health", get(health))
        .route("/probe", get(probe))
        .route("/proof", get(proof))
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
        assert!(!fallback.cached);
        assert!(fallback.manifest_hash_hex.is_none());

        let real = run_probe(&attestor, &test_inputs(Some("v2")), 2);
        assert_eq!(real.manifest_version, Some("v2"));
        assert_eq!(
            real.manifest_hash_hex.as_deref(),
            Some(qos_hex::encode(b"test-user-data").as_str())
        );
    }

    #[test]
    fn latency_stats_nearest_rank() {
        let mut samples: Vec<u128> = (1..=100).rev().collect();
        let stats = latency_stats(&mut samples).unwrap();
        assert_eq!(
            stats,
            LatencyStats {
                n: 100,
                min_us: 1,
                p50_us: 50,
                p95_us: 95,
                max_us: 100,
                mean_us: 50,
            }
        );
        assert!(latency_stats(&mut []).is_none());
    }

    #[test]
    fn cached_proof_matches_only_same_key_and_manifest() {
        let attestor = MockAttestor {
            document: vec![0xCC; 4],
        };
        let inputs = test_inputs(Some("v2"));
        let cached = CachedProof {
            ephemeral_public_key: inputs.ephemeral_public_key.clone(),
            user_data: inputs.user_data.clone(),
            attested_at: Instant::now(),
            result: run_probe(&attestor, &inputs, 1),
        };
        assert!(cached.matches(&inputs));

        let mut rotated = test_inputs(Some("v2"));
        rotated.ephemeral_public_key = vec![0xCD; 33];
        assert!(!cached.matches(&rotated));

        let mut other_manifest = test_inputs(Some("v2"));
        other_manifest.user_data = b"other".to_vec();
        assert!(!cached.matches(&other_manifest));
    }

    #[test]
    fn healthy_only_with_unexpired_attestation() {
        let state = AppState {
            attempt: AtomicU64::new(0),
            cache: AsyncMutex::new(None),
            attested_at: Mutex::new(None),
            refresh_after: Duration::from_secs(60),
        };
        assert!(!state.healthy(), "no attestation yet");

        state.set_attested_at(Some(Instant::now()));
        assert!(state.healthy());

        if let Some(expired) = Instant::now().checked_sub(ATTESTATION_CERT_LIFETIME) {
            state.set_attested_at(Some(expired));
            assert!(!state.healthy(), "cert lifetime elapsed");
        }

        state.set_attested_at(None);
        assert!(!state.healthy(), "cleared after failed refresh");
    }
}
