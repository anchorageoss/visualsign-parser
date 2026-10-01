//! Where a response's `bootProof` comes from.
//!
//! [`StaticBootProof`] carries a real ephemeral key and real manifest bytes
//! but an empty attestation doc; [`NsmBootProof`] fills the doc in from a
//! cached, verified `/dev/nsm` attestation (`tvc_attestation::cache`).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use host_primitives::turnkey::TurnkeyBootProof;
use qos_p256::P256Pair;
use tokio::sync::Notify;
use tvc_attestation::AttestationError;
use tvc_attestation::cache::{AttestationCache, InputLoader, Inputs, load_inputs};
use tvc_attestation::manifest::{BootProofManifest, boot_proof_manifest, read_manifest_envelope};
use tvc_attestation::paths;

/// Errors surfaced while assembling a boot proof. The `String` payloads are
/// read only through the derived `Debug` (see call sites' `{e:?}`
/// formatting), which rustc's dead-code analysis doesn't count as a read.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BootProofError {
    Manifest(String),
    Encode(String),
    /// Anything else from `tvc_attestation` (key, NSM, certificate, binding).
    Attestation(AttestationError),
}

impl From<AttestationError> for BootProofError {
    fn from(e: AttestationError) -> Self {
        match e {
            AttestationError::Manifest(e) => Self::Manifest(e),
            AttestationError::Encode(e) => Self::Encode(e),
            other => Self::Attestation(other),
        }
    }
}

pub trait BootProofSource {
    /// The proof for a successful response. `Err` means no attested proof is
    /// available right now; callers must fail the request rather than fall
    /// back to an unattested one.
    fn boot_proof(&self) -> Result<TurnkeyBootProof, BootProofError>;

    /// Readiness for `GET /health`.
    fn healthy(&self) -> bool {
        true
    }
}

pub struct StaticBootProof {
    ephemeral_public_key_hex: String,
    qos_manifest_b64: String,
    qos_manifest_envelope_b64: String,
    enclave_app: String,
    deployment_label: String,
}

impl StaticBootProof {
    pub fn from_enclave_files(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        let BootProofManifest {
            manifest_b64: qos_manifest_b64,
            envelope_b64: qos_manifest_envelope_b64,
        } = read_boot_proof_manifest(Path::new(paths::MANIFEST_FILE))?;
        Ok(Self::new(
            ephemeral,
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            enclave_app,
            deployment_label,
        ))
    }

    /// Test-only variant of [`Self::from_enclave_files`] that reads the
    /// manifest from an arbitrary path instead of the production
    /// `paths::MANIFEST_FILE` (the real, absolute `/qos.manifest` under the
    /// `vsock` feature). Lets tests point at a throwaway fixture instead of
    /// touching a real host path.
    #[cfg(test)]
    pub(crate) fn from_enclave_files_at(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
        manifest_path: &Path,
    ) -> Result<Self, BootProofError> {
        let BootProofManifest {
            manifest_b64: qos_manifest_b64,
            envelope_b64: qos_manifest_envelope_b64,
        } = read_boot_proof_manifest(manifest_path)?;
        Ok(Self::new(
            ephemeral,
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            enclave_app,
            deployment_label,
        ))
    }

    fn new(
        ephemeral: &P256Pair,
        qos_manifest_b64: String,
        qos_manifest_envelope_b64: String,
        enclave_app: String,
        deployment_label: String,
    ) -> Self {
        Self {
            ephemeral_public_key_hex: qos_hex::encode(&ephemeral.public_key().to_bytes()),
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            enclave_app,
            deployment_label,
        }
    }
}

impl BootProofSource for StaticBootProof {
    fn boot_proof(&self) -> Result<TurnkeyBootProof, BootProofError> {
        Ok(TurnkeyBootProof {
            // Empty off-enclave; never faked, so a strict verifier rejects
            // an unattested response outright. `NsmBootProof` fills it in.
            aws_attestation_doc_b64: String::new(),
            qos_manifest_b64: self.qos_manifest_b64.clone(),
            qos_manifest_envelope_b64: self.qos_manifest_envelope_b64.clone(),
            ephemeral_public_key_hex: self.ephemeral_public_key_hex.clone(),
            enclave_app: self.enclave_app.clone(),
            deployment_label: self.deployment_label.clone(),
        })
    }
}

/// How often the attestation watcher re-checks the inputs and expiry; what
/// the 72h `nsm_probe` soak ran with.
pub const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// NSM-backed boot proof: the manifest and ephemeral key from startup, plus
/// a verified attestation doc from [`AttestationCache`], refreshed by its
/// watcher before the NSM certificate chain expires.
///
/// The doc commits to the manifest hash (`user_data`) and the ephemeral
/// public key (`public_key`), with no nonce, so it isn't request-bound and
/// one cached doc serves every response. Every doc is checked against the
/// manifest (PCR0-3, PCR17) and our key before it's cached.
///
/// Both inputs are pinned to what this process loaded at startup: responses
/// are signed with the startup key and carry the startup manifest bytes, so
/// a doc attesting anything else would contradict the rest of the response.
/// If either file changes underneath us, the process can't follow: it would
/// need the new key to sign with. So the first mismatch marks the source
/// unhealthy and fails every request at once, and [`Self::input_drift`]
/// resolves so `main` can exit and the replica restarts on the new inputs.
pub struct NsmBootProof {
    base: StaticBootProof,
    cache: Arc<AttestationCache<qos_nsm::Nsm>>,
    drift: Arc<InputDrift>,
}

impl NsmBootProof {
    pub fn from_enclave_files(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        let envelope = read_manifest_envelope(Path::new(paths::MANIFEST_FILE))?;
        let BootProofManifest {
            manifest_b64,
            envelope_b64,
        } = boot_proof_manifest(&envelope)?;
        let pinned = PinnedInputs {
            ephemeral_public_key: ephemeral.public_key().to_bytes(),
            manifest_hash: envelope.manifest_hash().to_vec(),
        };
        let drift = Arc::new(InputDrift::default());
        let load = pinned_loader(pinned, Arc::clone(&drift), || {
            load_inputs(paths::EPHEMERAL_KEY_FILE, Path::new(paths::MANIFEST_FILE))
        });
        Ok(Self {
            base: StaticBootProof::new(
                ephemeral,
                manifest_b64,
                envelope_b64,
                enclave_app,
                deployment_label,
            ),
            cache: Arc::new(AttestationCache::new(Arc::new(qos_nsm::Nsm), load)),
            drift,
        })
    }

    /// For the startup attestation and `spawn_watcher`.
    pub fn cache(&self) -> &Arc<AttestationCache<qos_nsm::Nsm>> {
        &self.cache
    }

    /// Resolves once the enclave's key or manifest no longer matches what
    /// this process loaded at startup. Never resolves otherwise.
    pub async fn input_drift(&self) {
        self.drift.wait().await;
    }
}

impl BootProofSource for NsmBootProof {
    /// Serves the cache's latest verified doc without touching the enclave
    /// files or waiting behind a refresh: the watcher keeps it fresh, so a
    /// slow read or an in-flight NSM call never delays or fails a request.
    fn boot_proof(&self) -> Result<TurnkeyBootProof, BootProofError> {
        if self.drift.is_set() {
            return Err(AttestationError::Task(INPUT_DRIFT.to_string()).into());
        }
        let attestation = self.cache.latest().ok_or_else(|| {
            AttestationError::Task("no verified, unexpired attestation cached".to_string())
        })?;
        Ok(TurnkeyBootProof {
            aws_attestation_doc_b64: base64::engine::general_purpose::STANDARD
                .encode(&attestation.document),
            ..self.base.boot_proof()?
        })
    }

    fn healthy(&self) -> bool {
        !self.drift.is_set() && self.cache.healthy()
    }
}

pub const INPUT_DRIFT: &str = "enclave ephemeral key or manifest changed since startup";

/// Set, permanently, the first time the pin check fails. A failed pin check
/// alone wouldn't do it: `AttestationCache::get` returns a loader error before
/// it drops the cached doc, so `healthy()` would stay true until that doc's
/// certificate expired, up to ~3h of a replica that's healthy but 503s every
/// parse.
#[derive(Default)]
struct InputDrift {
    set: AtomicBool,
    notify: Notify,
}

impl InputDrift {
    fn mark(&self) {
        if !self.set.swap(true, Ordering::AcqRel) {
            // `notify_one` keeps a permit if nobody is waiting yet, so a
            // `wait` that starts later still returns.
            self.notify.notify_one();
        }
    }

    fn is_set(&self) -> bool {
        self.set.load(Ordering::Acquire)
    }

    async fn wait(&self) {
        while !self.is_set() {
            self.notify.notified().await;
        }
    }
}

/// `load`, then the pin check. Only a pin mismatch marks drift: a load that
/// fails outright (a transient read error) is the cache's to retry.
fn pinned_loader(
    pinned: PinnedInputs,
    drift: Arc<InputDrift>,
    load: impl Fn() -> Result<Inputs, AttestationError> + Send + Sync + 'static,
) -> InputLoader {
    Arc::new(move || {
        let checked = pinned.check(load()?);
        if checked.is_err() {
            drift.mark();
        }
        checked
    })
}

/// What this process signs with and serves; see [`NsmBootProof`].
struct PinnedInputs {
    ephemeral_public_key: Vec<u8>,
    manifest_hash: Vec<u8>,
}

impl PinnedInputs {
    fn check(&self, inputs: Inputs) -> Result<Inputs, AttestationError> {
        if inputs.ephemeral_public_key != self.ephemeral_public_key {
            return Err(AttestationError::EphemeralKey(
                "changed since startup; this process still signs with the startup key".to_string(),
            ));
        }
        if inputs.manifest_hash != self.manifest_hash {
            return Err(AttestationError::Manifest(
                "changed since startup; this process still serves the startup manifest".to_string(),
            ));
        }
        Ok(inputs)
    }
}

/// Same six keys as a real proof, every value empty. `qosManifestB64` carries
/// `pivotArgs` (including the X-Stamp allowlist) and, for v2 manifests,
/// `pivot.env`; every error response gets this instead, and only a successful
/// parse discloses the real proof. Don't put anything in pivot args or env
/// that must stay private: a successful response publishes it.
pub fn redacted_boot_proof() -> TurnkeyBootProof {
    TurnkeyBootProof {
        aws_attestation_doc_b64: String::new(),
        qos_manifest_b64: String::new(),
        qos_manifest_envelope_b64: String::new(),
        ephemeral_public_key_hex: String::new(),
        enclave_app: String::new(),
        deployment_label: String::new(),
    }
}

/// Read `/qos.manifest` (any QOS schema) and encode it for the wallet
/// contract's `qosManifestB64` / `qosManifestEnvelopeB64`: borsh for v0/v1
/// (byte-identical to before QOS 0.12.1), QOS storage JSON for v2, which is
/// JSON-only. Verifiers (visualsign-turnkeyclient `manifest/parser.go`) sniff
/// JSON and take their v2 path, else borsh-decode. See
/// `tvc_attestation::manifest::boot_proof_manifest`.
fn read_boot_proof_manifest(path: &Path) -> Result<BootProofManifest, BootProofError> {
    Ok(boot_proof_manifest(&read_manifest_envelope(path)?)?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use qos_core::protocol::services::boot::{
        Manifest, ManifestEnvelope, ManifestEnvelopeV2, ManifestSet, ManifestV2, ManifestVersion,
        Namespace, NitroConfig, PatchSet, PivotConfig, PivotConfigV2, RestartPolicy, ShareSet,
        VersionedManifestEnvelope,
    };
    use tvc_attestation::manifest::decode_manifest_envelope;

    // Built field-by-field rather than via `ManifestEnvelope::default()`:
    // that impl only exists behind qos_core's `mock` feature, which cannot
    // be unified into this crate's build graph. Every field here is a plain
    // public value, so no derive is needed at all.
    pub(crate) fn sample_manifest_envelope() -> ManifestEnvelope {
        ManifestEnvelope {
            manifest: Manifest {
                namespace: Namespace {
                    name: String::new(),
                    nonce: 0,
                    quorum_key: Vec::new(),
                },
                pivot: PivotConfig {
                    hash: [0u8; 32],
                    restart: RestartPolicy::Never,
                    bridge_config: Vec::new(),
                    debug_mode: false,
                    args: Vec::new(),
                },
                manifest_set: ManifestSet {
                    threshold: 0,
                    members: Vec::new(),
                },
                share_set: ShareSet {
                    threshold: 0,
                    members: Vec::new(),
                },
                enclave: NitroConfig {
                    pcr0: Vec::new(),
                    pcr1: Vec::new(),
                    pcr2: Vec::new(),
                    pcr3: Vec::new(),
                    aws_root_certificate: Vec::new(),
                    qos_commit: String::new(),
                },
                patch_set: PatchSet {
                    threshold: 0,
                    members: Vec::new(),
                },
            },
            manifest_set_approvals: Vec::new(),
            share_set_approvals: Vec::new(),
        }
    }

    // Writes to a unique path under the OS temp dir, never to
    // `paths::MANIFEST_FILE` (the real, absolute `/qos.manifest` under the
    // `vsock` feature): tests must not fail for an unprivileged
    // developer, or corrupt a real host manifest, just by running. Callers
    // read the manifest via `StaticBootProof::from_enclave_files_at` with
    // the returned path instead of the production `from_enclave_files`.
    // `OnceLock` guarantees the write happens exactly once per test-process
    // run, even under parallel test execution: a racy `path.exists()` check
    // could let one test observe the file mid-write (empty/partial) or reuse
    // a stale fixture left over from an earlier run.
    pub(crate) fn write_test_manifest_fixture() -> std::path::PathBuf {
        static INIT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        INIT.get_or_init(|| {
            let path = std::env::temp_dir().join(format!(
                "parser-http-server-test-manifest-{}.json",
                std::process::id()
            ));
            // QOS storage encoding (JSON with string-encoded numerics), as
            // qos_core 0.12.1 writes `/qos.manifest`.
            let bytes = qos_core::protocol::services::boot::VersionedManifestEnvelope::V1(
                sample_manifest_envelope(),
            )
            .to_storage_vec()
            .expect("failed to encode fixture");
            std::fs::write(&path, bytes).expect("failed to write manifest fixture");
            path
        })
        .clone()
    }

    // A v1 manifest (what `write_test_manifest_fixture` writes, as QOS
    // storage JSON) must still reach the boot proof as borsh, byte-identical
    // to before QOS 0.12.1: the Go verifier borsh-deserializes v1 and hashes
    // the borsh bytes into the attestation doc's `user_data`.
    #[test]
    fn v1_manifest_file_encodes_to_borsh() {
        let engine = base64::engine::general_purpose::STANDARD;
        let envelope = sample_manifest_envelope();
        let encoded = read_boot_proof_manifest(&write_test_manifest_fixture()).unwrap();

        let decoded_manifest: qos_core::protocol::services::boot::Manifest =
            borsh::from_slice(&engine.decode(encoded.manifest_b64).unwrap()).unwrap();
        assert_eq!(decoded_manifest, envelope.manifest);

        let decoded_envelope: ManifestEnvelope =
            borsh::from_slice(&engine.decode(encoded.envelope_b64).unwrap()).unwrap();
        assert_eq!(decoded_envelope, envelope);
    }

    fn sample_v2_envelope() -> ManifestEnvelopeV2 {
        let v1 = sample_manifest_envelope().manifest;
        ManifestEnvelopeV2 {
            manifest: ManifestV2 {
                version: ManifestVersion::V2,
                namespace: v1.namespace,
                pivot: PivotConfigV2 {
                    hash: v1.pivot.hash,
                    restart: v1.pivot.restart,
                    bridge_config: v1.pivot.bridge_config,
                    debug_mode: v1.pivot.debug_mode,
                    args: vec!["--host-port".to_string(), "3000".to_string()],
                    env: Default::default(),
                },
                manifest_set: v1.manifest_set,
                share_set: v1.share_set,
                enclave: v1.enclave,
                dns: None,
            },
            manifest_set_approvals: Vec::new(),
            share_set_approvals: Vec::new(),
        }
    }

    // A v2 `/qos.manifest` (what TVC writes for new apps) through this
    // crate's own seam: the boot proof must carry QOS storage JSON that
    // decodes back to the same envelope, not borsh (v2 is JSON-only).
    #[test]
    fn v2_manifest_file_reaches_boot_proof_as_storage_json() {
        let engine = base64::engine::general_purpose::STANDARD;
        let envelope = VersionedManifestEnvelope::V2(sample_v2_envelope());
        let path = std::env::temp_dir().join(format!(
            "parser-http-server-test-manifest-v2-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, envelope.to_storage_vec().unwrap()).unwrap();

        let pair = P256Pair::generate().unwrap();
        let proof = StaticBootProof::from_enclave_files_at(
            &pair,
            "app".to_string(),
            "l".to_string(),
            &path,
        )
        .unwrap()
        .boot_proof()
        .unwrap();
        std::fs::remove_file(&path).unwrap();

        let envelope_bytes = engine.decode(proof.qos_manifest_envelope_b64).unwrap();
        assert_eq!(envelope_bytes.first(), Some(&b'{'), "v2 envelope is JSON");
        assert_eq!(decode_manifest_envelope(&envelope_bytes).unwrap(), envelope);

        let manifest: serde_json::Value =
            serde_json::from_slice(&engine.decode(proof.qos_manifest_b64).unwrap()).unwrap();
        assert_eq!(manifest["version"], "v2");
        assert_eq!(
            manifest["pivot"]["args"],
            serde_json::json!(["--host-port", "3000"])
        );
    }

    fn inputs(key: &[u8], hash: &[u8]) -> Inputs {
        Inputs {
            ephemeral_public_key: key.to_vec(),
            manifest_hash: hash.to_vec(),
            pcrs: Default::default(),
        }
    }

    // The doc must attest the key responses are signed with and the manifest
    // they carry, so the loader refuses anything else rather than letting the
    // cache attest it.
    #[test]
    fn pinned_inputs_accept_only_the_startup_key_and_manifest() {
        let pinned = PinnedInputs {
            ephemeral_public_key: vec![1; 4],
            manifest_hash: vec![2; 4],
        };
        assert!(pinned.check(inputs(&[1; 4], &[2; 4])).is_ok());
        assert!(matches!(
            pinned.check(inputs(&[9; 4], &[2; 4])),
            Err(AttestationError::EphemeralKey(_))
        ));
        assert!(matches!(
            pinned.check(inputs(&[1; 4], &[9; 4])),
            Err(AttestationError::Manifest(_))
        ));
    }

    #[tokio::test]
    async fn pin_mismatch_marks_drift_but_a_failed_read_does_not() {
        let pinned = || PinnedInputs {
            ephemeral_public_key: vec![1; 4],
            manifest_hash: vec![2; 4],
        };

        let drift = Arc::new(InputDrift::default());
        let read_error = pinned_loader(pinned(), Arc::clone(&drift), || {
            Err(AttestationError::EphemeralKey("transient".to_string()))
        });
        assert!(read_error().is_err());
        assert!(!drift.is_set(), "a failed read is retried, not drift");

        let rotated = pinned_loader(pinned(), Arc::clone(&drift), || {
            Ok(inputs(&[9; 4], &[2; 4]))
        });
        assert!(matches!(rotated(), Err(AttestationError::EphemeralKey(_))));
        assert!(drift.is_set());
        // Marked before anyone waited: `wait` must still return.
        tokio::time::timeout(std::time::Duration::from_secs(1), drift.wait())
            .await
            .expect("wait returns once drift is marked");
    }
}
