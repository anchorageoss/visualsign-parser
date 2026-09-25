//! Where a response's `bootProof` comes from.
//!
//! [`StaticBootProof`] carries a real ephemeral key and real manifest bytes
//! but an empty attestation doc; [`NsmBootProof`] fills the doc in from a
//! real `/dev/nsm` call.

use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use host_primitives::turnkey::TurnkeyBootProof;
use qos_core::protocol::services::boot::ManifestEnvelope;
use qos_p256::P256Pair;

/// Maximum allowed size for the QOS manifest file (10 MB), matching the
/// bounded-reader convention in `parser/cli-core/src/mapping_parser.rs`.
const MAX_MANIFEST_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Errors surfaced while assembling a boot proof. The `String` payloads are
/// read only through the derived `Debug` (see call sites' `{e:?}`
/// formatting), which rustc's dead-code analysis doesn't count as a read.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BootProofError {
    Manifest(String),
    Encode(String),
    Nsm(String),
}

pub trait BootProofSource {
    fn boot_proof(&self) -> TurnkeyBootProof;
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
        let (qos_manifest_b64, qos_manifest_envelope_b64) = read_manifest_borsh_b64()?;
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
    /// `qos_core::MANIFEST_FILE` (the real, absolute `/qos.manifest` under
    /// the `vsock`/`vm` feature). Lets tests point at a throwaway fixture
    /// instead of touching a real host path.
    #[cfg(test)]
    pub(crate) fn from_enclave_files_at(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
        manifest_path: &Path,
    ) -> Result<Self, BootProofError> {
        let (qos_manifest_b64, qos_manifest_envelope_b64) =
            read_manifest_borsh_b64_at(manifest_path)?;
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
    fn boot_proof(&self) -> TurnkeyBootProof {
        TurnkeyBootProof {
            // Empty until an NSM-backed source lands; never faked, so a
            // strict verifier rejects an unattested response outright.
            aws_attestation_doc_b64: String::new(),
            qos_manifest_b64: self.qos_manifest_b64.clone(),
            qos_manifest_envelope_b64: self.qos_manifest_envelope_b64.clone(),
            ephemeral_public_key_hex: self.ephemeral_public_key_hex.clone(),
            enclave_app: self.enclave_app.clone(),
            deployment_label: self.deployment_label.clone(),
        }
    }
}

/// Same six keys as a real proof, every value empty. `qosManifestB64` carries
/// `pivotArgs` (including the X-Stamp allowlist), so every error response gets
/// this instead and only a successful parse discloses the real proof.
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

/// `/qos.manifest` holds JSON at qos rev 365ba7ed, but the wallet contract's
/// `qosManifestB64` / `qosManifestEnvelopeB64` are *borsh* bytes: the Go
/// verifier borsh-deserializes both (visualsign-turnkeyclient
/// manifest/parser.go), and the attestation doc's `user_data` is
/// sha256(borsh(manifest)). Base64-ing the file bytes directly would produce
/// fields no verifier can read. So: read JSON, re-encode with borsh.
///
/// Shared by `StaticBootProof` and `NsmBootProof`, which also needs the
/// envelope for `manifest.qos_hash()`.
pub fn read_manifest_envelope() -> Result<ManifestEnvelope, BootProofError> {
    read_manifest_envelope_at(Path::new(qos_core::MANIFEST_FILE))
}

fn read_manifest_envelope_at(path: &Path) -> Result<ManifestEnvelope, BootProofError> {
    let file = std::fs::File::open(path)
        .map_err(|e| BootProofError::Manifest(format!("{}: {e}", path.display())))?;

    // Bounded reader: never read more than MAX_MANIFEST_FILE_SIZE, even if the
    // file grows between the open and the read.
    let mut bounded = file.take(MAX_MANIFEST_FILE_SIZE + 1);
    let mut contents = Vec::new();
    bounded
        .read_to_end(&mut contents)
        .map_err(|e| BootProofError::Manifest(format!("{}: {e}", path.display())))?;

    if contents.len() as u64 > MAX_MANIFEST_FILE_SIZE {
        return Err(BootProofError::Manifest(format!(
            "{} exceeds maximum size (> {MAX_MANIFEST_FILE_SIZE} bytes)",
            path.display()
        )));
    }

    serde_json::from_slice(&contents)
        .map_err(|e| BootProofError::Manifest(format!("manifest json: {e}")))
}

fn read_manifest_borsh_b64() -> Result<(String, String), BootProofError> {
    encode_manifest_borsh_b64(&read_manifest_envelope()?)
}

#[cfg(test)]
fn read_manifest_borsh_b64_at(path: &Path) -> Result<(String, String), BootProofError> {
    encode_manifest_borsh_b64(&read_manifest_envelope_at(path)?)
}

fn encode_manifest_borsh_b64(
    envelope: &ManifestEnvelope,
) -> Result<(String, String), BootProofError> {
    Ok((
        encode_borsh_b64(&envelope.manifest)?,
        encode_borsh_b64(envelope)?,
    ))
}

fn encode_borsh_b64(v: &impl borsh::BorshSerialize) -> Result<String, BootProofError> {
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = borsh::to_vec(v).map_err(|e| BootProofError::Encode(format!("{e}")))?;
    Ok(engine.encode(bytes))
}

/// NSM-backed boot proof: the real AWS Nitro attestation document, generated
/// once at construction and reused for every response.
///
/// Reproduces qos_core's post-boot attestation call
/// (`protocol/services/attestation.rs::get_post_boot_attestation_doc`): the
/// manifest hash goes in `user_data`, the ephemeral pubkey in `public_key`,
/// and `nonce` stays `None`. That makes the document not request-bound, so
/// generating it once at startup (rather than per request) is correct, not
/// just cheap: our reference verifier (`visualsign-turnkeyclient
/// cmd/verify.go`) sets `SkipTimestampCheck: true`, so there is no freshness
/// window to satisfy.
pub struct NsmBootProof {
    base: StaticBootProof,
    aws_attestation_doc_b64: String,
}

impl NsmBootProof {
    /// Production constructor: calls the real `/dev/nsm` device.
    pub fn new(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        Self::from_envelope(
            qos_nsm::Nsm,
            &read_manifest_envelope()?,
            ephemeral,
            enclave_app,
            deployment_label,
        )
    }

    /// Test seam: takes any `NsmProvider` and an already-read envelope so
    /// unit tests can assert the attestor is called exactly once without
    /// touching `/dev/nsm` or `qos_core::MANIFEST_FILE`.
    fn from_envelope<A: qos_nsm::NsmProvider>(
        attestor: A,
        envelope: &ManifestEnvelope,
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        use qos_core::protocol::QosHash;
        use qos_nsm::types::{NsmRequest, NsmResponse};

        let manifest_hash = envelope.manifest.qos_hash().to_vec();
        let ephemeral_public_key = ephemeral.public_key().to_bytes();

        let response = attestor.nsm_process_request(NsmRequest::Attestation {
            user_data: Some(manifest_hash),
            nonce: None,
            public_key: Some(ephemeral_public_key),
        });
        let document = match response {
            NsmResponse::Attestation { document } => document,
            other => return Err(BootProofError::Nsm(format!("{other:?}"))),
        };

        let (qos_manifest_b64, qos_manifest_envelope_b64) = encode_manifest_borsh_b64(envelope)?;

        Ok(Self {
            base: StaticBootProof::new(
                ephemeral,
                qos_manifest_b64,
                qos_manifest_envelope_b64,
                enclave_app,
                deployment_label,
            ),
            aws_attestation_doc_b64: base64::engine::general_purpose::STANDARD.encode(document),
        })
    }
}

impl BootProofSource for NsmBootProof {
    fn boot_proof(&self) -> TurnkeyBootProof {
        TurnkeyBootProof {
            aws_attestation_doc_b64: self.aws_attestation_doc_b64.clone(),
            ..self.base.boot_proof()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use qos_core::protocol::QosHash;
    use qos_core::protocol::services::boot::{
        Manifest, ManifestSet, Namespace, NitroConfig, PatchSet, PivotConfig, RestartPolicy,
        ShareSet,
    };
    use qos_nsm::NsmProvider;
    use qos_nsm::nitro::AttestError;
    use qos_nsm::types::{NsmRequest, NsmResponse};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Built field-by-field rather than via `ManifestEnvelope::default()`:
    // that impl only exists behind qos_core's `mock` feature, which cannot
    // be unified in the same build graph as this crate's `vsock` feature
    // (qos_core's own `compile_error!` forbids `vm` + `mock` together). Every
    // field here is a plain public value, so no derive is needed at all.
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
    // `qos_core::MANIFEST_FILE` (the real, absolute `/qos.manifest` under the
    // `vsock`/`vm` feature): tests must not fail for an unprivileged
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
            let bytes =
                serde_json::to_vec(&sample_manifest_envelope()).expect("failed to encode fixture");
            std::fs::write(&path, bytes).expect("failed to write manifest fixture");
            path
        })
        .clone()
    }

    // The Go verifier borsh-deserializes both `qosManifestB64` and
    // `qosManifestEnvelopeB64` and hashes the borsh bytes into the
    // attestation doc's `user_data` (see the module doc on
    // `read_manifest_envelope`). Prove the encode side actually round-trips
    // through borsh, so a future refactor that swaps the encoding (e.g. to
    // JSON, or introduces a HashMap-backed field) fails a test instead of
    // silently breaking verification.
    #[test]
    fn manifest_and_envelope_borsh_b64_round_trip() {
        let envelope = sample_manifest_envelope();
        let engine = base64::engine::general_purpose::STANDARD;

        let manifest_b64 = encode_borsh_b64(&envelope.manifest).unwrap();
        let decoded_manifest: qos_core::protocol::services::boot::Manifest =
            borsh::from_slice(&engine.decode(manifest_b64).unwrap()).unwrap();
        assert_eq!(decoded_manifest, envelope.manifest);

        let envelope_b64 = encode_borsh_b64(&envelope).unwrap();
        let decoded_envelope: ManifestEnvelope =
            borsh::from_slice(&engine.decode(envelope_b64).unwrap()).unwrap();
        assert_eq!(decoded_envelope, envelope);
    }

    struct CountingAttestor {
        calls: Arc<AtomicUsize>,
        // Captures the last request so tests can assert on the
        // security-critical fields (`user_data`, `public_key`, `nonce`),
        // not just that the attestor was called.
        last_request: Arc<std::sync::Mutex<Option<NsmRequest>>>,
        document: Vec<u8>,
    }

    impl NsmProvider for CountingAttestor {
        fn nsm_process_request(&self, request: NsmRequest) -> NsmResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_request.lock().unwrap() = Some(request);
            NsmResponse::Attestation {
                document: self.document.clone(),
            }
        }

        fn timestamp_ms(&self) -> Result<u64, AttestError> {
            Ok(0)
        }
    }

    #[test]
    fn nsm_boot_proof_generates_once_and_reuses_the_document() {
        // The production route passes nonce: None, so the doc is not
        // request-bound and can be generated at startup. Our own verifier
        // does not check the timestamp either (visualsign-turnkeyclient
        // cmd/verify.go sets SkipTimestampCheck: true), so caching is safe
        // rather than merely cheap.
        let calls = Arc::new(AtomicUsize::new(0));
        let last_request = Arc::new(std::sync::Mutex::new(None));
        let envelope = sample_manifest_envelope();
        let ephemeral = qos_p256::P256Pair::generate().unwrap();
        let source = NsmBootProof::from_envelope(
            CountingAttestor {
                calls: calls.clone(),
                last_request: last_request.clone(),
                document: vec![0xAA; 64],
            },
            &envelope,
            &ephemeral,
            "visualsign-parser".to_string(),
            "test".to_string(),
        )
        .unwrap();

        let first = source.boot_proof();
        let second = source.boot_proof();
        assert_eq!(
            first.aws_attestation_doc_b64,
            second.aws_attestation_doc_b64
        );
        assert!(!first.aws_attestation_doc_b64.is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "doc must be generated once"
        );

        // The NSM input contract: user_data is the manifest's qos_hash,
        // public_key is the ephemeral key bytes, and nonce is None (the
        // doc must not be request-bound, see the module doc above).
        let expected_request = NsmRequest::Attestation {
            user_data: Some(envelope.manifest.qos_hash().to_vec()),
            nonce: None,
            public_key: Some(ephemeral.public_key().to_bytes()),
        };
        assert_eq!(
            *last_request.lock().unwrap(),
            Some(expected_request),
            "NSM request must carry the manifest hash, ephemeral public key, and no nonce"
        );
    }
}
