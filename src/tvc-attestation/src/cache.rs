//! A cached, verified NSM attestation that tracks the enclave's live inputs.
//!
//! Attesting per request costs ~1ms of NSM time plus ~10ms to verify the doc
//! (measured on TVC, QOS 0.12.1); a cache hit costs ~0.4ms, the re-read of the
//! two input files. The cache is keyed on (ephemeral public key, manifest
//! hash): QOS 0.12.1 rotates the setup ephemeral key to the live one after
//! provisioning, and a doc attesting a stale key must not be served.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qos_core::handles::EphemeralKeyHandle;
use qos_nsm::NsmProvider;
use qos_nsm::types::{NsmRequest, NsmResponse};
use tokio::sync::Mutex as AsyncMutex;

use crate::AttestationError;
use crate::cert::{CertValidity, cert_validity};
use crate::manifest::read_manifest_envelope;

/// Upper bound on one load-and-attest, so a hung `/dev/nsm` call surfaces as
/// an error instead of a request that never returns.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Re-attest once this little validity is left. The NSM leaf lives ~3h.
pub const DEFAULT_REFRESH_MARGIN: Duration = Duration::from_secs(30 * 60);

/// What an attestation commits to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// Goes in the attestation's `public_key`.
    pub ephemeral_public_key: Vec<u8>,
    /// `VersionedManifestEnvelope::manifest_hash()`; goes in `user_data`.
    pub manifest_hash: Vec<u8>,
}

/// Read the current inputs from QOS's files.
///
/// # Errors
///
/// [`AttestationError::EphemeralKey`] / [`AttestationError::Manifest`] if
/// either file is missing or undecodable.
pub fn load_inputs(
    ephemeral_key_path: &str,
    manifest_path: &Path,
) -> Result<Inputs, AttestationError> {
    let key = EphemeralKeyHandle::new(ephemeral_key_path.to_string())
        .get_ephemeral_key()
        .map_err(|e| AttestationError::EphemeralKey(format!("{ephemeral_key_path}: {e:?}")))?;
    let envelope = read_manifest_envelope(manifest_path)?;
    Ok(Inputs {
        ephemeral_public_key: key.public_key().to_bytes(),
        manifest_hash: envelope.manifest_hash().to_vec(),
    })
}

/// A verified attestation and what it attests to.
#[derive(Debug)]
pub struct Attestation {
    /// COSE Sign1 attestation document from NSM.
    pub document: Vec<u8>,
    pub inputs: Inputs,
    pub cert: CertValidity,
    /// Wall time of the `/dev/nsm` call alone.
    pub nsm_latency: Duration,
    /// Monotonic deadline: attested-at + `cert.remaining`.
    valid_until: Instant,
}

/// Why the cache re-attested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshReason {
    Initial,
    EphemeralKeyChanged,
    ManifestChanged,
    NearExpiry,
}

/// Why the cache can't serve `inputs` at `now`, or `None` if it can. Input
/// changes win over freshness; the expiry margin is inclusive.
#[must_use]
pub fn refresh_reason(
    cached: Option<&Attestation>,
    inputs: &Inputs,
    now: Instant,
    margin: Duration,
) -> Option<RefreshReason> {
    match cached {
        None => Some(RefreshReason::Initial),
        Some(c) if c.inputs.ephemeral_public_key != inputs.ephemeral_public_key => {
            Some(RefreshReason::EphemeralKeyChanged)
        }
        Some(c) if c.inputs.manifest_hash != inputs.manifest_hash => {
            Some(RefreshReason::ManifestChanged)
        }
        Some(c) if c.valid_until.saturating_duration_since(now) <= margin => {
            Some(RefreshReason::NearExpiry)
        }
        Some(_) => None,
    }
}

/// Healthy strictly before `valid_until`.
#[must_use]
pub fn healthy_at(valid_until: Option<Instant>, now: Instant) -> bool {
    valid_until.is_some_and(|until| now < until)
}

/// Loads the current [`Inputs`]; blocking, run off the async runtime.
pub type InputLoader = Arc<dyn Fn() -> Result<Inputs, AttestationError> + Send + Sync>;

/// A single verified attestation, refreshed on input change or near expiry.
pub struct AttestationCache<A> {
    attestor: Arc<A>,
    load: InputLoader,
    refresh_margin: Duration,
    call_timeout: Duration,
    /// Held across the whole check-and-attest, so concurrent callers
    /// single-flight one refresh instead of racing duplicate NSM calls or
    /// storing an older doc over a newer one.
    current: AsyncMutex<Option<Arc<Attestation>>>,
    /// `current`'s deadline, mirrored so [`Self::healthy`] never waits behind
    /// an in-flight NSM call.
    valid_until: Mutex<Option<Instant>>,
}

impl<A: NsmProvider + Send + Sync + 'static> AttestationCache<A> {
    #[must_use]
    pub fn new(attestor: Arc<A>, load: InputLoader) -> Self {
        Self {
            attestor,
            load,
            refresh_margin: DEFAULT_REFRESH_MARGIN,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            current: AsyncMutex::new(None),
            valid_until: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn with_refresh_margin(mut self, margin: Duration) -> Self {
        self.refresh_margin = margin;
        self
    }

    #[must_use]
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// True while a verified attestation is cached and its chain unexpired.
    /// Gate readiness on this: a replica whose first NSM call fails never
    /// reports healthy.
    pub fn healthy(&self) -> bool {
        let until = *self.valid_until.lock().unwrap_or_else(|e| e.into_inner());
        healthy_at(until, Instant::now())
    }

    /// The current attestation, re-attesting first if the inputs changed or
    /// the cached doc is within the refresh margin of expiry.
    ///
    /// If a near-expiry refresh fails, the still-valid cached doc is returned.
    /// If a refresh after an input change fails, the stale doc is dropped (and
    /// [`Self::healthy`] goes false) rather than served.
    ///
    /// # Errors
    ///
    /// Any [`AttestationError`] from loading inputs or attesting, when there's
    /// no valid doc for the current inputs to fall back to.
    pub async fn get(&self) -> Result<(Arc<Attestation>, Option<RefreshReason>), AttestationError> {
        let mut current = self.current.lock().await;

        let load = Arc::clone(&self.load);
        let inputs = self.bounded(move || load()).await?;
        let Some(reason) = refresh_reason(
            current.as_deref(),
            &inputs,
            Instant::now(),
            self.refresh_margin,
        ) else {
            if let Some(attestation) = current.as_ref() {
                return Ok((Arc::clone(attestation), None));
            }
            return Err(AttestationError::Task(
                "cache emptied under lock".to_string(),
            ));
        };

        let (load, attestor) = (Arc::clone(&self.load), Arc::clone(&self.attestor));
        match self.bounded(move || attest(&*attestor, load()?)).await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                self.set_valid_until(Some(fresh.valid_until));
                *current = Some(Arc::clone(&fresh));
                Ok((fresh, Some(reason)))
            }
            Err(e) => match current.as_ref() {
                Some(old)
                    if reason == RefreshReason::NearExpiry
                        && healthy_at(Some(old.valid_until), Instant::now()) =>
                {
                    Ok((Arc::clone(old), None))
                }
                _ => {
                    *current = None;
                    self.set_valid_until(None);
                    Err(e)
                }
            },
        }
    }

    /// Spawn a tokio task that calls [`Self::get`] every `interval`, so key
    /// rotation and expiry are handled without waiting for a request.
    ///
    /// The task only awaits; file reads and the NSM call run on the blocking
    /// pool. Keep the handle: abort it on shutdown, or `select!` on it to
    /// notice if it ever exits. If it dies, nothing extends the cached
    /// deadline, so [`Self::healthy`] goes false at certificate expiry
    /// (fail-closed). A zero `interval` is treated as 1ms.
    pub fn spawn_watcher(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(Arc::clone(self).watch(interval.max(Duration::from_millis(1))))
    }

    async fn watch(self: Arc<Self>, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match self.get().await {
                Ok((a, Some(reason))) => eprintln!(
                    "attestation: refreshed ({reason:?}); cert valid {}s, nsm {}us",
                    a.cert.remaining.as_secs(),
                    a.nsm_latency.as_micros()
                ),
                Ok((_, None)) => {}
                Err(e) => eprintln!("attestation: refresh failed: {e}"),
            }
        }
    }

    fn set_valid_until(&self, until: Option<Instant>) {
        *self.valid_until.lock().unwrap_or_else(|e| e.into_inner()) = until;
    }

    async fn bounded<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> Result<T, AttestationError> + Send + 'static,
    ) -> Result<T, AttestationError> {
        match tokio::time::timeout(self.call_timeout, tokio::task::spawn_blocking(f)).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(AttestationError::Task(format!("panicked: {e}"))),
            Err(_) => Err(AttestationError::Task(format!(
                "exceeded {:?}",
                self.call_timeout
            ))),
        }
    }
}

/// One NSM attestation over `inputs`, verified; only docs with validity left
/// are accepted.
fn attest<A: NsmProvider>(attestor: &A, inputs: Inputs) -> Result<Attestation, AttestationError> {
    let start = Instant::now();
    let response = attestor.nsm_process_request(NsmRequest::Attestation {
        user_data: Some(inputs.manifest_hash.clone()),
        nonce: None,
        public_key: Some(inputs.ephemeral_public_key.clone()),
    });
    let nsm_latency = start.elapsed();
    let document = match response {
        NsmResponse::Attestation { document } => document,
        other => return Err(AttestationError::Nsm(format!("{other:?}"))),
    };
    let cert = cert_validity(&document)?;
    if cert.remaining.is_zero() {
        return Err(AttestationError::Certificate(format!(
            "chain already expired at attestation (notAfter {})",
            cert.not_after_unix
        )));
    }
    Ok(Attestation {
        valid_until: Instant::now() + cert.remaining,
        document,
        inputs,
        cert,
        nsm_latency,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cert::tests::NITRO_DOC;
    use qos_nsm::nitro::AttestError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Returns `document` for every attestation request and counts calls.
    struct FakeNsm {
        document: Mutex<Vec<u8>>,
        calls: AtomicUsize,
    }

    impl FakeNsm {
        fn new(document: &[u8]) -> Arc<Self> {
            Arc::new(Self {
                document: Mutex::new(document.to_vec()),
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn set_document(&self, document: &[u8]) {
            *self.document.lock().unwrap() = document.to_vec();
        }
    }

    impl NsmProvider for FakeNsm {
        fn nsm_process_request(&self, request: NsmRequest) -> NsmResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match request {
                NsmRequest::Attestation { .. } => NsmResponse::Attestation {
                    document: self.document.lock().unwrap().clone(),
                },
                other => panic!("unexpected NSM request: {other:?}"),
            }
        }

        fn timestamp_ms(&self) -> Result<u64, AttestError> {
            Ok(0)
        }
    }

    fn inputs(key: u8, manifest: u8) -> Inputs {
        Inputs {
            ephemeral_public_key: vec![key; 33],
            manifest_hash: vec![manifest; 32],
        }
    }

    /// A loader whose inputs the test can change, to simulate key rotation.
    fn loader(initial: Inputs) -> (InputLoader, Arc<Mutex<Inputs>>) {
        let shared = Arc::new(Mutex::new(initial));
        let handle = Arc::clone(&shared);
        let load: InputLoader = Arc::new(move || Ok(handle.lock().unwrap().clone()));
        (load, shared)
    }

    fn cached_at(inputs: &Inputs, valid_until: Instant) -> Attestation {
        Attestation {
            document: Vec::new(),
            inputs: inputs.clone(),
            cert: cert_validity(NITRO_DOC).unwrap(),
            nsm_latency: Duration::ZERO,
            valid_until,
        }
    }

    #[test]
    fn refresh_reason_margin_boundaries() {
        let margin = Duration::from_secs(30 * 60);
        let now = Instant::now();
        let i = inputs(1, 1);
        let at = |left: Duration| cached_at(&i, now + left);

        assert_eq!(
            refresh_reason(None, &i, now, margin),
            Some(RefreshReason::Initial)
        );
        let c = at(margin + Duration::from_secs(1));
        assert_eq!(
            refresh_reason(Some(&c), &i, now, margin),
            None,
            "just outside margin"
        );
        let c = at(margin);
        assert_eq!(
            refresh_reason(Some(&c), &i, now, margin),
            Some(RefreshReason::NearExpiry),
            "margin is inclusive"
        );
        let c = at(Duration::ZERO);
        assert_eq!(
            refresh_reason(Some(&c), &i, now + Duration::from_secs(1), margin),
            Some(RefreshReason::NearExpiry),
            "past expiry"
        );
        let c = at(Duration::from_secs(1));
        assert_eq!(
            refresh_reason(Some(&c), &i, now, Duration::ZERO),
            None,
            "zero margin"
        );
        let c = at(Duration::ZERO);
        assert_eq!(
            refresh_reason(Some(&c), &i, now, Duration::ZERO),
            Some(RefreshReason::NearExpiry),
            "zero margin, at expiry"
        );
    }

    #[test]
    fn refresh_reason_input_changes_win_over_freshness() {
        let now = Instant::now();
        let fresh = cached_at(&inputs(1, 1), now + Duration::from_secs(3 * 60 * 60));
        assert_eq!(
            refresh_reason(Some(&fresh), &inputs(2, 1), now, Duration::ZERO),
            Some(RefreshReason::EphemeralKeyChanged)
        );
        assert_eq!(
            refresh_reason(Some(&fresh), &inputs(1, 2), now, Duration::ZERO),
            Some(RefreshReason::ManifestChanged)
        );
    }

    #[test]
    fn healthy_at_boundaries() {
        let now = Instant::now();
        assert!(!healthy_at(None, now));
        assert!(healthy_at(Some(now + Duration::from_secs(1)), now));
        assert!(
            !healthy_at(Some(now), now),
            "unhealthy at the instant of expiry"
        );
        assert!(!healthy_at(Some(now), now + Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn caches_until_inputs_change() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, current) = loader(inputs(1, 1));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        assert!(!cache.healthy(), "not healthy before first attestation");

        let (first, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::Initial));
        assert_eq!(first.cert.remaining, Duration::from_secs(3 * 60 * 60));
        assert!(cache.healthy());

        let (again, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None, "served from cache");
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(nsm.calls(), 1, "no second NSM call on a hit");

        *current.lock().unwrap() = inputs(2, 1);
        let (rotated, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::EphemeralKeyChanged));
        assert_eq!(rotated.inputs, inputs(2, 1));
        assert_eq!(nsm.calls(), 2);
    }

    #[tokio::test]
    async fn concurrent_gets_single_flight_one_nsm_call() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, _) = loader(inputs(1, 1));
        let cache = Arc::new(AttestationCache::new(Arc::clone(&nsm), load));
        let gets = (0..8).map(|_| {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move { cache.get().await.map(|(a, _)| a) })
        });
        for g in gets {
            g.await.unwrap().unwrap();
        }
        assert_eq!(nsm.calls(), 1);
    }

    #[tokio::test]
    async fn unverifiable_doc_is_never_cached_or_healthy() {
        let nsm = FakeNsm::new(b"not a cose sign1");
        let (load, _) = loader(inputs(1, 1));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::Certificate(_)), "{err}");
        assert!(!cache.healthy());
        // Nothing cached: the next call attests again.
        cache.get().await.unwrap_err();
        assert_eq!(nsm.calls(), 2);
    }

    #[tokio::test]
    async fn failed_refresh_after_rotation_drops_stale_doc() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, current) = loader(inputs(1, 1));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        cache.get().await.unwrap();
        assert!(cache.healthy());

        nsm.set_document(b"broken");
        *current.lock().unwrap() = inputs(2, 1);
        cache.get().await.unwrap_err();
        assert!(
            !cache.healthy(),
            "doc for the old key must not keep us healthy"
        );
    }

    #[tokio::test]
    async fn failed_near_expiry_refresh_keeps_serving_valid_doc() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, _) = loader(inputs(1, 1));
        // A margin longer than the 3h lifetime makes every get a NearExpiry refresh.
        let cache = AttestationCache::new(Arc::clone(&nsm), load)
            .with_refresh_margin(Duration::from_secs(4 * 60 * 60));
        let (first, _) = cache.get().await.unwrap();

        nsm.set_document(b"broken");
        let (served, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None);
        assert!(
            Arc::ptr_eq(&first, &served),
            "still-valid doc served on failed refresh"
        );
        assert!(cache.healthy());
    }

    #[tokio::test]
    async fn loader_failure_is_an_error_not_a_hang() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let load: InputLoader =
            Arc::new(|| Err(AttestationError::EphemeralKey("missing".to_string())));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::EphemeralKey(_)));
        assert_eq!(nsm.calls(), 0);
    }

    #[tokio::test]
    async fn slow_loader_times_out() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let load: InputLoader = Arc::new(|| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(inputs(1, 1))
        });
        let cache = AttestationCache::new(Arc::clone(&nsm), load)
            .with_call_timeout(Duration::from_millis(20));
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::Task(_)), "{err}");
    }

    #[tokio::test]
    async fn watcher_refreshes_on_rotation_without_requests() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, current) = loader(inputs(1, 1));
        let cache = Arc::new(AttestationCache::new(Arc::clone(&nsm), load));
        let watcher = cache.spawn_watcher(Duration::from_millis(5));

        // The NSM call is counted before the doc is verified and stored, so
        // wait on the stored effect, not the counter.
        wait_for(|| cache.healthy()).await;
        assert_eq!(nsm.calls(), 1, "first tick attests once");

        *current.lock().unwrap() = inputs(2, 1);
        wait_for(|| nsm.calls() == 2).await;
        // `get` takes the cache lock, so it waits out the watcher's in-flight
        // refresh and then sees the rotated doc as current.
        let (served, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None, "watcher already refreshed");
        assert_eq!(served.inputs, inputs(2, 1));

        watcher.abort();
        assert!(watcher.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn zero_interval_watcher_does_not_panic() {
        let nsm = FakeNsm::new(NITRO_DOC);
        let (load, _) = loader(inputs(1, 1));
        let cache = Arc::new(AttestationCache::new(Arc::clone(&nsm), load));
        let watcher = cache.spawn_watcher(Duration::ZERO);
        wait_for(|| nsm.calls() >= 1).await;
        assert!(!watcher.is_finished(), "still running");
        watcher.abort();
    }

    /// Poll `cond` for up to 5s; the watcher runs on real time and the
    /// blocking pool, so tests wait for its effect instead of sleeping blind.
    async fn wait_for(cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < deadline, "condition not met within 5s");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    #[test]
    fn load_inputs_reports_missing_files() {
        let err =
            load_inputs("/nonexistent/qos.ephemeral.key", Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(err, AttestationError::EphemeralKey(_)), "{err}");
    }
}
