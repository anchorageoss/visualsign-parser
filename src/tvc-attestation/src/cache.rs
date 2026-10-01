//! A cached, verified NSM attestation that tracks the enclave's live inputs.
//!
//! Attesting per request costs ~1ms of NSM time plus ~10ms to verify the doc
//! (measured on TVC, QOS 0.12.1); a cache hit costs ~0.4ms, the re-read of the
//! two input files. The cache is keyed on (ephemeral public key, manifest
//! hash): QOS 0.12.1 rotates the setup ephemeral key to the live one after
//! provisioning, and a doc attesting a stale key must not be served. Every doc
//! is checked to attest exactly the inputs it's cached under before it's
//! stored.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aws_nitro_enclaves_nsm_api::api::AttestationDoc;
use qos_core::handles::EphemeralKeyHandle;
use qos_nsm::NsmProvider;
use qos_nsm::nitro::{ManifestAttestationInput, verify_attestation_doc_against_manifest_live};
use qos_nsm::types::{NsmRequest, NsmResponse};
use tokio::sync::Mutex as AsyncMutex;

use crate::AttestationError;
use crate::cert::{CertValidity, VerifiedDoc, verify_document};
use crate::manifest::read_manifest_envelope;

/// Upper bound on one load-and-attest, so a hung `/dev/nsm` call surfaces as
/// an error instead of a request that never returns.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Re-attest once this little validity is left.
///
/// NSM reuses its ~3h leaf certificate across attestations and only issues a
/// new one shortly before expiry (~15m left, observed on TVC with QOS 0.12.1),
/// so a refresh inside this margin often returns the same leaf; see
/// [`DEFAULT_RETRY_BACKOFF`].
pub const DEFAULT_REFRESH_MARGIN: Duration = Duration::from_secs(30 * 60);

/// After a near-expiry refresh that returned the same leaf (or failed), wait
/// this long before the next near-expiry attempt, instead of re-attesting on
/// every watcher tick until NSM rotates the leaf.
pub const DEFAULT_RETRY_BACKOFF: Duration = Duration::from_secs(60);

/// What an attestation commits to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// Goes in the attestation's `public_key`.
    pub ephemeral_public_key: Vec<u8>,
    /// `VersionedManifestEnvelope::manifest_hash()`; goes in `user_data`.
    pub manifest_hash: Vec<u8>,
    /// The manifest's expected PCR0-3 (`manifest.enclave()`), checked against
    /// the doc. Derived from the manifest, so not part of the cache key.
    pub pcrs: [Vec<u8>; 4],
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
    let manifest_hash = envelope.manifest_hash().to_vec();
    let manifest = envelope.manifest();
    let enclave = manifest.enclave();
    Ok(Inputs {
        ephemeral_public_key: key.public_key().to_bytes(),
        manifest_hash,
        pcrs: [
            enclave.pcr0.clone(),
            enclave.pcr1.clone(),
            enclave.pcr2.clone(),
            enclave.pcr3.clone(),
        ],
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
    /// Monotonic deadline: instant before the NSM call + `cert.remaining`, so
    /// it never runs past the certificate's real expiry.
    valid_until: Instant,
}

impl Attestation {
    /// How long until this doc's certificate chain expires, on the monotonic
    /// clock; zero once expired.
    #[must_use]
    pub fn valid_for(&self) -> Duration {
        self.valid_until.saturating_duration_since(Instant::now())
    }
}

/// Why the cache re-attested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshReason {
    Initial,
    EphemeralKeyChanged,
    ManifestChanged,
    NearExpiry,
}

/// Why `cached` can't serve `inputs` at `now`, or `None` if it can. Input
/// changes win over freshness; the expiry margin is inclusive.
pub(crate) fn refresh_reason(
    cached: &Attestation,
    inputs: &Inputs,
    now: Instant,
    margin: Duration,
) -> Option<RefreshReason> {
    if cached.inputs.ephemeral_public_key != inputs.ephemeral_public_key {
        Some(RefreshReason::EphemeralKeyChanged)
    } else if cached.inputs.manifest_hash != inputs.manifest_hash {
        Some(RefreshReason::ManifestChanged)
    } else if cached.valid_until.saturating_duration_since(now) <= margin {
        Some(RefreshReason::NearExpiry)
    } else {
        None
    }
}

/// Healthy strictly before `valid_until`.
pub(crate) fn healthy_at(valid_until: Option<Instant>, now: Instant) -> bool {
    valid_until.is_some_and(|until| now < until)
}

/// Counters for how the cache has behaved, for status endpoints and soak
/// tests: `near_expiry` rising once per leaf rotation shows certificate-driven
/// refresh is working; `ephemeral_key_changed` shows the setup -> live key
/// rotation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub initial: u64,
    pub ephemeral_key_changed: u64,
    pub manifest_changed: u64,
    /// Near-expiry refreshes that obtained a newer leaf certificate.
    pub near_expiry: u64,
    /// Near-expiry refreshes where NSM returned the same leaf certificate
    /// (not yet rotated); the cached doc was kept and the next attempt
    /// backed off.
    pub near_expiry_same_cert: u64,
    /// Near-expiry refreshes that failed while the still-valid cached doc
    /// kept being served (also recorded in `last_error`).
    pub refresh_failures: u64,
    /// `get` calls that returned an error.
    pub failures: u64,
    pub last_error: Option<String>,
}

impl CacheStats {
    fn record(
        &mut self,
        outcome: &Result<(Arc<Attestation>, Option<RefreshReason>), AttestationError>,
    ) {
        match outcome {
            Ok((_, Some(RefreshReason::Initial))) => self.initial += 1,
            Ok((_, Some(RefreshReason::EphemeralKeyChanged))) => self.ephemeral_key_changed += 1,
            Ok((_, Some(RefreshReason::ManifestChanged))) => self.manifest_changed += 1,
            Ok((_, Some(RefreshReason::NearExpiry))) => self.near_expiry += 1,
            Ok((_, None)) => {}
            Err(e) => {
                self.failures += 1;
                self.last_error = Some(e.to_string());
            }
        }
    }
}

/// Loads the current [`Inputs`]; blocking, run off the async runtime.
pub type InputLoader = Arc<dyn Fn() -> Result<Inputs, AttestationError> + Send + Sync>;

/// Verifies a raw attestation doc; [`verify_document`] outside tests.
type Verifier = fn(&[u8]) -> Result<VerifiedDoc, AttestationError>;

/// Clears the in-flight flag when the NSM call finishes, however it finishes.
/// Moved into the blocking closure, so a call that outlives its timeout keeps
/// the flag set until it actually returns.
struct InFlight(Arc<AtomicBool>);

impl InFlight {
    fn acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        (!flag.swap(true, Ordering::AcqRel)).then(|| Self(Arc::clone(flag)))
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A single verified attestation, refreshed on input change or near expiry.
pub struct AttestationCache<A> {
    attestor: Arc<A>,
    load: InputLoader,
    verify: Verifier,
    refresh_margin: Duration,
    retry_backoff: Duration,
    call_timeout: Duration,
    /// Set while this cache's `/dev/nsm` call is running. The cache lock
    /// already makes its calls one-at-a-time, but a timeout releases that lock
    /// without cancelling the blocking thread, and the kernel driver
    /// serializes ioctls on one mutex: a retry during a hung call would just
    /// park another blocking-pool thread behind it. So new attempts fail fast
    /// until the stuck call returns.
    nsm_in_flight: Arc<AtomicBool>,
    /// Held across the whole check-and-attest, so concurrent callers
    /// single-flight one refresh instead of racing duplicate NSM calls or
    /// storing an older doc over a newer one.
    current: AsyncMutex<Option<Arc<Attestation>>>,
    /// `current`, mirrored so [`Self::healthy`] and [`Self::latest`] never
    /// wait behind an in-flight refresh.
    latest: Mutex<Option<Arc<Attestation>>>,
    /// No near-expiry attempt before this instant (set after one returned the
    /// same leaf or failed). Only touched while `current` is locked.
    near_expiry_retry_after: Mutex<Option<Instant>>,
    stats: Mutex<CacheStats>,
}

impl<A: NsmProvider + Send + Sync + 'static> AttestationCache<A> {
    #[must_use]
    pub fn new(attestor: Arc<A>, load: InputLoader) -> Self {
        Self {
            attestor,
            load,
            verify: verify_document,
            refresh_margin: DEFAULT_REFRESH_MARGIN,
            retry_backoff: DEFAULT_RETRY_BACKOFF,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            nsm_in_flight: Arc::new(AtomicBool::new(false)),
            current: AsyncMutex::new(None),
            latest: Mutex::new(None),
            near_expiry_retry_after: Mutex::new(None),
            stats: Mutex::new(CacheStats::default()),
        }
    }

    #[must_use]
    pub fn with_refresh_margin(mut self, margin: Duration) -> Self {
        self.refresh_margin = margin;
        self
    }

    #[must_use]
    pub fn with_retry_backoff(mut self, backoff: Duration) -> Self {
        self.retry_backoff = backoff;
        self
    }

    #[must_use]
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    #[cfg(test)]
    fn with_verifier(mut self, verify: Verifier) -> Self {
        self.verify = verify;
        self
    }

    /// True while a verified attestation is cached and its chain unexpired.
    /// Gate readiness on this: a replica whose first NSM call fails never
    /// reports healthy.
    pub fn healthy(&self) -> bool {
        self.latest().is_some()
    }

    /// The cached attestation if its chain is unexpired, without loading the
    /// inputs, attesting, or waiting behind a refresh in progress. For
    /// serving a doc on a request path while [`Self::spawn_watcher`] keeps
    /// it fresh; [`Self::get`] is what refreshes it.
    pub fn latest(&self) -> Option<Arc<Attestation>> {
        let latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        latest
            .as_ref()
            .filter(|a| healthy_at(Some(a.valid_until), Instant::now()))
            .map(Arc::clone)
    }

    /// The current attestation, re-attesting first if the inputs changed or
    /// the cached doc is within the refresh margin of expiry.
    ///
    /// If a near-expiry refresh fails, or NSM hands back the same leaf
    /// certificate, the still-valid cached doc is returned and further
    /// near-expiry attempts wait `retry_backoff`.
    /// If a refresh after an input change fails, the stale doc is dropped (and
    /// [`Self::healthy`] goes false) rather than served.
    ///
    /// # Errors
    ///
    /// Any [`AttestationError`] from loading inputs or attesting, when there's
    /// no valid doc for the current inputs to fall back to.
    pub async fn get(&self) -> Result<(Arc<Attestation>, Option<RefreshReason>), AttestationError> {
        let outcome = self.get_inner().await;
        self.lock_stats().record(&outcome);
        outcome
    }

    /// Refresh and failure counts since construction.
    pub fn stats(&self) -> CacheStats {
        self.lock_stats().clone()
    }

    async fn get_inner(
        &self,
    ) -> Result<(Arc<Attestation>, Option<RefreshReason>), AttestationError> {
        let mut current = self.current.lock().await;

        let load = Arc::clone(&self.load);
        let inputs = self.bounded(move || load()).await?;
        let now = Instant::now();
        let reason = match current.as_ref() {
            None => RefreshReason::Initial,
            Some(cached) => match refresh_reason(cached, &inputs, now, self.refresh_margin) {
                None => return Ok((Arc::clone(cached), None)),
                Some(RefreshReason::NearExpiry)
                    if self.retry_after().is_some_and(|until| now < until) =>
                {
                    return Ok((Arc::clone(cached), None));
                }
                Some(reason) => reason,
            },
        };

        let attested = match InFlight::acquire(&self.nsm_in_flight) {
            Some(in_flight) => {
                let (attestor, verify) = (Arc::clone(&self.attestor), self.verify);
                self.bounded(move || {
                    let _in_flight = in_flight;
                    attest(&*attestor, verify, inputs)
                })
                .await
            }
            None => Err(AttestationError::Task(
                "previous NSM call still in flight".to_string(),
            )),
        };

        match attested {
            Ok(fresh) => {
                if reason == RefreshReason::NearExpiry
                    && let Some(old) = current.as_ref()
                    && fresh.cert.not_after_unix <= old.cert.not_after_unix
                {
                    // NSM hasn't rotated its leaf yet: nothing gained.
                    self.set_retry_after(Some(Instant::now() + self.retry_backoff));
                    self.lock_stats().near_expiry_same_cert += 1;
                    return Ok((Arc::clone(old), None));
                }
                self.set_retry_after(None);
                let fresh = Arc::new(fresh);
                self.set_latest(Some(Arc::clone(&fresh)));
                *current = Some(Arc::clone(&fresh));
                Ok((fresh, Some(reason)))
            }
            Err(e) => match current.as_ref() {
                Some(old)
                    if reason == RefreshReason::NearExpiry
                        && healthy_at(Some(old.valid_until), Instant::now()) =>
                {
                    self.set_retry_after(Some(Instant::now() + self.retry_backoff));
                    let mut stats = self.lock_stats();
                    stats.refresh_failures += 1;
                    stats.last_error = Some(e.to_string());
                    Ok((Arc::clone(old), None))
                }
                _ => {
                    *current = None;
                    self.set_latest(None);
                    self.set_retry_after(None);
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
            // `writeln!` rather than `eprintln!`, which panics (killing the
            // watcher) if stderr is gone.
            match self.get().await {
                Ok((a, Some(reason))) => {
                    let _ = writeln!(
                        std::io::stderr(),
                        "attestation: refreshed ({reason:?}); cert valid {}s, nsm {}us",
                        a.cert.remaining.as_secs(),
                        a.nsm_latency.as_micros()
                    );
                }
                Ok((_, None)) => {}
                Err(e) => {
                    let _ = writeln!(std::io::stderr(), "attestation: refresh failed: {e}");
                }
            }
        }
    }

    fn lock_stats(&self) -> std::sync::MutexGuard<'_, CacheStats> {
        self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn retry_after(&self) -> Option<Instant> {
        *self
            .near_expiry_retry_after
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn set_retry_after(&self, until: Option<Instant>) {
        *self
            .near_expiry_retry_after
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = until;
    }

    fn set_latest(&self, latest: Option<Arc<Attestation>>) {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = latest;
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

/// One NSM attestation over `inputs`, verified and checked to attest exactly
/// those inputs; only docs with validity left are accepted.
fn attest<A: NsmProvider>(
    attestor: &A,
    verify: Verifier,
    inputs: Inputs,
) -> Result<Attestation, AttestationError> {
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
    let verified = verify(&document)?;
    check_binding(&verified.doc, &inputs)?;
    let cert = verified.cert;
    if cert.remaining.is_zero() {
        return Err(AttestationError::Certificate(format!(
            "chain already expired at attestation (notAfter {})",
            cert.not_after_unix
        )));
    }
    Ok(Attestation {
        valid_until: start + cert.remaining,
        document,
        inputs,
        cert,
        nsm_latency,
    })
}

/// The doc must attest exactly what we asked for. QOS's own live check
/// (`user_data` is the manifest hash, no nonce, PCR0-3 match the manifest, and
/// PCR17 commits to the manifest and the doc's public key), plus the one thing
/// it leaves to the caller: that public key is our ephemeral key.
fn check_binding(doc: &AttestationDoc, inputs: &Inputs) -> Result<(), AttestationError> {
    if doc.public_key.as_ref().map(|k| k.as_slice()) != Some(inputs.ephemeral_public_key.as_slice())
    {
        return Err(AttestationError::Binding(
            "public_key is not the ephemeral key".to_string(),
        ));
    }
    let [pcr0, pcr1, pcr2, pcr3] = &inputs.pcrs;
    verify_attestation_doc_against_manifest_live(
        doc,
        ManifestAttestationInput {
            manifest_hash: &inputs.manifest_hash,
            pcr0,
            pcr1,
            pcr2,
            pcr3,
        },
    )
    .map_err(|e| AttestationError::Binding(format!("{e:?}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cert::tests::{
        NITRO_DOC, TVC_EPHEMERAL_KEY, TVC_LIVE_DOC, TVC_MANIFEST_HASH, TVC_PCRS,
    };
    use aws_nitro_enclaves_nsm_api::api::Digest;
    use qos_nsm::nitro::{
        ATTESTABLE_PCR_COUNT, AttestError, ManifestCommitmentKind, expected_manifest_commitment_pcr,
    };
    use serde_bytes::ByteBuf;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;

    /// Fixed NSM clock for fake docs (seconds since the unix epoch).
    const T0: u64 = 1_800_000_000;
    const THREE_HOURS: u64 = 3 * 60 * 60;

    /// What the fake NSM returns for an attestation request.
    #[derive(Clone)]
    enum Reply {
        /// A fake doc attesting whatever was requested, with this leaf
        /// `notAfter`; decoded by [`fake_verify`].
        Echo { not_after: u64 },
        /// A fake doc attesting a different key than requested.
        WrongKey,
        /// These exact bytes (real docs, garbage).
        Raw(Vec<u8>),
        /// Block until the test releases it, like a hung `/dev/nsm`.
        Hang(Arc<std::sync::Barrier>),
    }

    struct FakeNsm {
        reply: Mutex<Reply>,
        calls: AtomicUsize,
    }

    impl FakeNsm {
        fn new(reply: Reply) -> Arc<Self> {
            Arc::new(Self {
                reply: Mutex::new(reply),
                calls: AtomicUsize::new(0),
            })
        }

        /// Echoes requests with a fresh 3h leaf.
        fn echo() -> Arc<Self> {
            Self::new(Reply::Echo {
                not_after: T0 + THREE_HOURS,
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn set(&self, reply: Reply) {
            *self.reply.lock().unwrap() = reply;
        }
    }

    impl NsmProvider for FakeNsm {
        fn nsm_process_request(&self, request: NsmRequest) -> NsmResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let NsmRequest::Attestation {
                user_data,
                public_key,
                ..
            } = request
            else {
                panic!("unexpected NSM request: {request:?}");
            };
            let reply = self.reply.lock().unwrap().clone();
            let document = match reply {
                Reply::Echo { not_after } => {
                    fake_doc(&public_key.unwrap(), &user_data.unwrap(), not_after)
                }
                Reply::WrongKey => fake_doc(&[0xEE; 33], &user_data.unwrap(), T0 + THREE_HOURS),
                Reply::Raw(bytes) => bytes,
                Reply::Hang(barrier) => {
                    barrier.wait();
                    fake_doc(&public_key.unwrap(), &user_data.unwrap(), T0 + THREE_HOURS)
                }
            };
            NsmResponse::Attestation { document }
        }

        fn timestamp_ms(&self) -> Result<u64, AttestError> {
            Ok(0)
        }
    }

    /// `[u32 len][public_key][u32 len][user_data][u64 not_after]`, LE.
    fn fake_doc(public_key: &[u8], user_data: &[u8], not_after: u64) -> Vec<u8> {
        let mut doc = Vec::new();
        for field in [public_key, user_data] {
            doc.extend_from_slice(&u32::try_from(field.len()).unwrap().to_le_bytes());
            doc.extend_from_slice(field);
        }
        doc.extend_from_slice(&not_after.to_le_bytes());
        doc
    }

    /// PCR0-3 every fake doc (and every test manifest) carries.
    fn fake_pcrs() -> [Vec<u8>; 4] {
        [
            vec![0xA0; 48],
            vec![0xA1; 48],
            vec![0xA2; 48],
            vec![0xA3; 48],
        ]
    }

    /// A doc the way QOS 0.12.1 leaves it after boot: every attestable PCR
    /// present, PCR0-3 from the manifest, PCR17 the live commitment.
    fn fake_attestation_doc(public_key: Vec<u8>, user_data: Vec<u8>) -> AttestationDoc {
        let mut pcrs: BTreeMap<usize, ByteBuf> = (0..usize::from(ATTESTABLE_PCR_COUNT))
            .map(|i| (i, ByteBuf::from(vec![0; 48])))
            .collect();
        for (i, pcr) in fake_pcrs().into_iter().enumerate() {
            pcrs.insert(i, ByteBuf::from(pcr));
        }
        let live =
            expected_manifest_commitment_pcr(ManifestCommitmentKind::Live, &user_data, &public_key)
                .unwrap();
        pcrs.insert(17, ByteBuf::from(live.to_vec()));
        AttestationDoc {
            module_id: "fake".to_string(),
            digest: Digest::SHA384,
            timestamp: T0 * 1000,
            pcrs,
            certificate: ByteBuf::new(),
            cabundle: Vec::new(),
            public_key: Some(ByteBuf::from(public_key)),
            user_data: Some(ByteBuf::from(user_data)),
            nonce: None,
        }
    }

    /// Decodes [`fake_doc`] as if it verified, timestamped at `T0`; anything
    /// else fails like an unverifiable doc.
    fn fake_verify(doc: &[u8]) -> Result<VerifiedDoc, AttestationError> {
        let bad = || AttestationError::Certificate("decode: not a fake doc".to_string());
        let mut rest = doc;
        let mut field = || -> Result<Vec<u8>, AttestationError> {
            let (len, tail) = rest.split_first_chunk::<4>().ok_or_else(bad)?;
            let len = usize::try_from(u32::from_le_bytes(*len)).map_err(|_| bad())?;
            let (value, tail) = tail.split_at_checked(len).ok_or_else(bad)?;
            rest = tail;
            Ok(value.to_vec())
        };
        let (public_key, user_data) = (field()?, field()?);
        let not_after = u64::from_le_bytes(*rest.first_chunk::<8>().ok_or_else(bad)?);
        Ok(VerifiedDoc {
            cert: CertValidity {
                timestamp_ms: T0 * 1000,
                not_after_unix: not_after,
                remaining: Duration::from_secs(not_after.saturating_sub(T0)),
            },
            doc: fake_attestation_doc(public_key, user_data),
        })
    }

    fn inputs(key: u8, manifest: u8) -> Inputs {
        Inputs {
            ephemeral_public_key: vec![key; 33],
            manifest_hash: vec![manifest; 32],
            pcrs: fake_pcrs(),
        }
    }

    /// A loader whose inputs the test can change, to simulate key rotation.
    fn loader(initial: Inputs) -> (InputLoader, Arc<Mutex<Inputs>>) {
        let shared = Arc::new(Mutex::new(initial));
        let handle = Arc::clone(&shared);
        let load: InputLoader = Arc::new(move || Ok(handle.lock().unwrap().clone()));
        (load, shared)
    }

    /// A cache over `nsm` that verifies with [`fake_verify`].
    fn fake_cache(nsm: &Arc<FakeNsm>, load: InputLoader) -> AttestationCache<FakeNsm> {
        AttestationCache::new(Arc::clone(nsm), load).with_verifier(fake_verify)
    }

    fn cached_at(inputs: &Inputs, valid_until: Instant) -> Attestation {
        Attestation {
            document: Vec::new(),
            inputs: inputs.clone(),
            cert: CertValidity {
                timestamp_ms: T0 * 1000,
                not_after_unix: T0 + THREE_HOURS,
                remaining: Duration::from_secs(THREE_HOURS),
            },
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

        let c = at(margin + Duration::from_secs(1));
        assert_eq!(
            refresh_reason(&c, &i, now, margin),
            None,
            "just outside margin"
        );
        let c = at(margin);
        assert_eq!(
            refresh_reason(&c, &i, now, margin),
            Some(RefreshReason::NearExpiry),
            "margin is inclusive"
        );
        let c = at(Duration::ZERO);
        assert_eq!(
            refresh_reason(&c, &i, now + Duration::from_secs(1), margin),
            Some(RefreshReason::NearExpiry),
            "past expiry"
        );
        let c = at(Duration::from_secs(1));
        assert_eq!(
            refresh_reason(&c, &i, now, Duration::ZERO),
            None,
            "zero margin"
        );
        let c = at(Duration::ZERO);
        assert_eq!(
            refresh_reason(&c, &i, now, Duration::ZERO),
            Some(RefreshReason::NearExpiry),
            "zero margin, at expiry"
        );
    }

    #[test]
    fn refresh_reason_input_changes_win_over_freshness() {
        let now = Instant::now();
        let fresh = cached_at(&inputs(1, 1), now + Duration::from_secs(THREE_HOURS));
        assert_eq!(
            refresh_reason(&fresh, &inputs(2, 1), now, Duration::ZERO),
            Some(RefreshReason::EphemeralKeyChanged)
        );
        assert_eq!(
            refresh_reason(&fresh, &inputs(1, 2), now, Duration::ZERO),
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

    /// What the real TVC doc attests, as the cache would load it.
    fn tvc_inputs() -> Inputs {
        Inputs {
            ephemeral_public_key: qos_hex::decode(TVC_EPHEMERAL_KEY).unwrap(),
            manifest_hash: qos_hex::decode(TVC_MANIFEST_HASH).unwrap(),
            pcrs: TVC_PCRS.map(|p| qos_hex::decode(p).unwrap()),
        }
    }

    fn tvc_doc() -> AttestationDoc {
        verify_document(TVC_LIVE_DOC).unwrap().doc
    }

    #[test]
    fn binding_accepts_real_tvc_live_doc() {
        assert_eq!(check_binding(&tvc_doc(), &tvc_inputs()), Ok(()));
    }

    #[test]
    fn binding_rejects_each_mismatch_on_real_doc() {
        let expect_binding_err = |doc: &AttestationDoc, inputs: &Inputs, what: &str| {
            let err = check_binding(doc, inputs).unwrap_err();
            assert!(matches!(err, AttestationError::Binding(_)), "{what}: {err}");
        };
        let (doc, ok) = (tvc_doc(), tvc_inputs());

        let mut i = ok.clone();
        i.ephemeral_public_key[1] ^= 1;
        expect_binding_err(&doc, &i, "other ephemeral key");

        let mut i = ok.clone();
        i.manifest_hash[0] ^= 1;
        expect_binding_err(&doc, &i, "other manifest hash");

        for idx in 0..4 {
            let mut i = ok.clone();
            i.pcrs[idx][0] ^= 1;
            expect_binding_err(&doc, &i, &format!("manifest PCR{idx} differs"));
        }

        // A doc claiming our key but whose PCR17 commits to something else:
        // the public-key check alone would accept it.
        let mut d = doc.clone();
        let pcr17 = d.pcrs.get_mut(&17).unwrap();
        pcr17[0] ^= 1;
        expect_binding_err(&d, &ok, "PCR17 live commitment tampered");

        let mut d = doc.clone();
        d.nonce = Some(ByteBuf::from(vec![1]));
        expect_binding_err(&d, &ok, "unexpected nonce");

        let mut d = doc;
        d.public_key = None;
        expect_binding_err(&d, &ok, "no public key");
    }

    #[test]
    fn binding_rejects_pre_commitment_doc() {
        // The 2022 fixture verifies against the AWS root but predates QOS's
        // manifest-commitment PCRs; even with its own key and user_data it
        // is not a valid QOS live attestation.
        let doc = verify_document(NITRO_DOC).unwrap().doc;
        let inputs = Inputs {
            ephemeral_public_key: doc.public_key.clone().unwrap().into_vec(),
            manifest_hash: doc.user_data.clone().unwrap().into_vec(),
            pcrs: [0, 1, 2, 3].map(|i| doc.pcrs.get(&i).unwrap().to_vec()),
        };
        assert!(matches!(
            check_binding(&doc, &inputs),
            Err(AttestationError::Binding(_))
        ));
    }

    #[tokio::test]
    async fn real_doc_not_attesting_our_inputs_is_rejected() {
        let nsm = FakeNsm::new(Reply::Raw(TVC_LIVE_DOC.to_vec()));
        let (load, _) = loader(inputs(1, 1));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::Binding(_)), "{err}");
        assert!(!cache.healthy());
    }

    #[tokio::test]
    async fn real_doc_attesting_our_inputs_is_accepted() {
        let nsm = FakeNsm::new(Reply::Raw(TVC_LIVE_DOC.to_vec()));
        let (load, _) = loader(tvc_inputs());
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        let (a, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::Initial));
        assert_eq!(a.cert.not_after_unix, 1_790_603_442);
        assert!(cache.healthy());
    }

    #[tokio::test]
    async fn doc_attesting_another_key_is_rejected() {
        let nsm = FakeNsm::new(Reply::WrongKey);
        let (load, _) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load);
        let err = cache.get().await.unwrap_err();
        assert_eq!(
            err,
            AttestationError::Binding("public_key is not the ephemeral key".to_string())
        );
        assert!(!cache.healthy());
    }

    #[tokio::test]
    async fn caches_until_inputs_change() {
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load);
        assert!(!cache.healthy(), "not healthy before first attestation");

        let (first, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::Initial));
        assert_eq!(first.cert.remaining, Duration::from_secs(THREE_HOURS));
        let left = first.valid_for();
        assert!(left <= first.cert.remaining && left > Duration::from_secs(THREE_HOURS - 60));
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

        assert_eq!(
            cache.stats(),
            CacheStats {
                initial: 1,
                ephemeral_key_changed: 1,
                ..CacheStats::default()
            },
            "cache hits aren't counted"
        );
    }

    #[tokio::test]
    async fn concurrent_gets_single_flight_one_nsm_call() {
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        let cache = Arc::new(fake_cache(&nsm, load));
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
        let nsm = FakeNsm::new(Reply::Raw(b"not a cose sign1".to_vec()));
        let (load, _) = loader(inputs(1, 1));
        let cache = AttestationCache::new(Arc::clone(&nsm), load);
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::Certificate(_)), "{err}");
        assert!(!cache.healthy());
        // Nothing cached: the next call attests again.
        cache.get().await.unwrap_err();
        assert_eq!(nsm.calls(), 2);

        let stats = cache.stats();
        assert_eq!(stats.failures, 2);
        assert_eq!(stats.initial, 0);
        assert!(
            stats
                .last_error
                .unwrap()
                .contains("attestation certificate")
        );
    }

    #[tokio::test]
    async fn latest_follows_get() {
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load);
        assert!(
            cache.latest().is_none(),
            "nothing before the first attestation"
        );

        let (first, _) = cache.get().await.unwrap();
        assert!(Arc::ptr_eq(&cache.latest().unwrap(), &first));

        nsm.set(Reply::Raw(b"broken".to_vec()));
        *current.lock().unwrap() = inputs(2, 1);
        cache.get().await.unwrap_err();
        assert!(
            cache.latest().is_none(),
            "a doc for the old key is never served"
        );
    }

    // The request path reads `latest` while the watcher refreshes: it must
    // keep serving the valid doc rather than wait behind the cache lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn latest_does_not_wait_behind_a_refresh() {
        let release = Arc::new(std::sync::Barrier::new(2));
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = Arc::new(fake_cache(&nsm, load).with_call_timeout(Duration::from_secs(30)));
        let (first, _) = cache.get().await.unwrap();

        nsm.set(Reply::Hang(Arc::clone(&release)));
        *current.lock().unwrap() = inputs(2, 1);
        let refreshing = tokio::spawn({
            let cache = Arc::clone(&cache);
            async move { cache.get().await.map(|_| ()) }
        });
        wait_for(|| cache.nsm_in_flight.load(Ordering::Acquire)).await;

        let served = cache.latest().expect("valid doc still served");
        assert!(Arc::ptr_eq(&served, &first));

        nsm.set(Reply::Echo {
            not_after: T0 + THREE_HOURS,
        });
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .unwrap();
        refreshing.await.unwrap().unwrap();
        assert!(
            !Arc::ptr_eq(&cache.latest().unwrap(), &first),
            "the refreshed doc replaces it"
        );
    }

    #[tokio::test]
    async fn failed_refresh_after_rotation_drops_stale_doc() {
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load);
        cache.get().await.unwrap();
        assert!(cache.healthy());

        nsm.set(Reply::Raw(b"broken".to_vec()));
        *current.lock().unwrap() = inputs(2, 1);
        cache.get().await.unwrap_err();
        assert!(
            !cache.healthy(),
            "doc for the old key must not keep us healthy"
        );
    }

    #[tokio::test]
    async fn failed_near_expiry_refresh_serves_valid_doc_and_is_counted() {
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        // A margin longer than the 3h lifetime makes every get a NearExpiry refresh.
        let cache = fake_cache(&nsm, load).with_refresh_margin(Duration::from_secs(4 * 60 * 60));
        let (first, _) = cache.get().await.unwrap();

        nsm.set(Reply::Raw(b"broken".to_vec()));
        let (served, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None);
        assert!(
            Arc::ptr_eq(&first, &served),
            "still-valid doc served on failed refresh"
        );
        assert!(cache.healthy());

        // Backed off: no retry storm while NSM is failing.
        cache.get().await.unwrap();
        assert_eq!(nsm.calls(), 2);

        let stats = cache.stats();
        assert_eq!(stats.refresh_failures, 1, "visible before hard expiry");
        assert_eq!(stats.failures, 0, "every get was served");
        assert!(stats.last_error.unwrap().contains("not a fake doc"));
    }

    #[tokio::test]
    async fn near_expiry_same_leaf_backs_off_instead_of_looping() {
        // NSM keeps handing back the same leaf until it rotates; a margin
        // longer than its 3h life makes every get due.
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load)
            .with_refresh_margin(Duration::from_secs(4 * 60 * 60))
            .with_retry_backoff(Duration::from_secs(60 * 60));
        let (first, _) = cache.get().await.unwrap();

        let (served, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None, "same leaf is not a refresh");
        assert!(Arc::ptr_eq(&first, &served), "cached doc kept");
        assert_eq!(nsm.calls(), 2, "one near-expiry attempt");

        for _ in 0..10 {
            cache.get().await.unwrap();
        }
        assert_eq!(nsm.calls(), 2, "backed off: no NSM call per get");
        assert!(cache.healthy());
        assert_eq!(
            cache.stats(),
            CacheStats {
                initial: 1,
                near_expiry_same_cert: 1,
                ..CacheStats::default()
            }
        );
    }

    #[tokio::test]
    async fn near_expiry_newer_leaf_is_stored() {
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load)
            .with_refresh_margin(Duration::from_secs(4 * 60 * 60))
            .with_retry_backoff(Duration::ZERO);
        let (first, _) = cache.get().await.unwrap();

        // NSM rotates: the new leaf outlives the margin.
        nsm.set(Reply::Echo {
            not_after: T0 + 2 * THREE_HOURS,
        });
        let (rotated, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::NearExpiry));
        assert!(!Arc::ptr_eq(&first, &rotated));
        assert_eq!(rotated.cert.not_after_unix, T0 + 2 * THREE_HOURS);

        let (again, reason) = cache.get().await.unwrap();
        assert_eq!(reason, None, "new leaf is outside the margin");
        assert!(Arc::ptr_eq(&rotated, &again));
        assert_eq!(nsm.calls(), 2);
        assert_eq!(cache.stats().near_expiry, 1);
    }

    #[tokio::test]
    async fn zero_backoff_retries_same_leaf_every_get() {
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load)
            .with_refresh_margin(Duration::from_secs(4 * 60 * 60))
            .with_retry_backoff(Duration::ZERO);
        for _ in 0..4 {
            cache.get().await.unwrap();
        }
        assert_eq!(nsm.calls(), 4);
        assert_eq!(cache.stats().near_expiry_same_cert, 3);
    }

    #[tokio::test]
    async fn input_change_ignores_near_expiry_backoff() {
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load)
            .with_refresh_margin(Duration::from_secs(4 * 60 * 60))
            .with_retry_backoff(Duration::from_secs(60 * 60));
        cache.get().await.unwrap();
        cache.get().await.unwrap(); // same leaf -> backoff armed
        *current.lock().unwrap() = inputs(2, 1);
        let (rotated, reason) = cache.get().await.unwrap();
        assert_eq!(reason, Some(RefreshReason::EphemeralKeyChanged));
        assert_eq!(rotated.inputs, inputs(2, 1));
        assert_eq!(nsm.calls(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hung_nsm_call_is_not_joined_by_more() {
        let release = Arc::new(std::sync::Barrier::new(2));
        let nsm = FakeNsm::new(Reply::Hang(Arc::clone(&release)));
        let (load, _) = loader(inputs(1, 1));
        let cache = fake_cache(&nsm, load).with_call_timeout(Duration::from_millis(20));

        let err = cache.get().await.unwrap_err();
        assert!(err.to_string().contains("exceeded"), "{err}");
        // The first call is still stuck on its blocking thread.
        for _ in 0..5 {
            let err = cache.get().await.unwrap_err();
            assert!(err.to_string().contains("still in flight"), "{err}");
        }
        assert_eq!(nsm.calls(), 1, "no new blocking thread per attempt");

        // Once NSM returns, attempts resume.
        nsm.set(Reply::Echo {
            not_after: T0 + THREE_HOURS,
        });
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .unwrap();
        wait_for(|| !cache.nsm_in_flight.load(Ordering::Acquire)).await;
        cache.get().await.unwrap();
        assert_eq!(nsm.calls(), 2);
    }

    #[tokio::test]
    async fn loader_failure_is_an_error_not_a_hang() {
        let nsm = FakeNsm::echo();
        let load: InputLoader =
            Arc::new(|| Err(AttestationError::EphemeralKey("missing".to_string())));
        let cache = fake_cache(&nsm, load);
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::EphemeralKey(_)));
        assert_eq!(nsm.calls(), 0);
    }

    #[tokio::test]
    async fn slow_loader_times_out() {
        let nsm = FakeNsm::echo();
        let load: InputLoader = Arc::new(|| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(inputs(1, 1))
        });
        let cache = fake_cache(&nsm, load).with_call_timeout(Duration::from_millis(20));
        let err = cache.get().await.unwrap_err();
        assert!(matches!(err, AttestationError::Task(_)), "{err}");
    }

    #[tokio::test]
    async fn watcher_refreshes_on_rotation_without_requests() {
        let nsm = FakeNsm::echo();
        let (load, current) = loader(inputs(1, 1));
        let cache = Arc::new(fake_cache(&nsm, load));
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
        let nsm = FakeNsm::echo();
        let (load, _) = loader(inputs(1, 1));
        let cache = Arc::new(fake_cache(&nsm, load));
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
