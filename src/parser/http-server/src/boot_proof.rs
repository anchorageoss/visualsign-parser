//! Where a response's `bootProof` comes from.
//!
//! [`StaticBootProof`] carries a real ephemeral key and real manifest bytes
//! but an empty attestation doc; [`NsmBootProof`] fills the doc in from a
//! real `/dev/nsm` call.

use host_primitives::turnkey::TurnkeyBootProof;
use qos_core::protocol::services::boot::ManifestEnvelope;
use qos_p256::P256Pair;

/// Errors surfaced while assembling a boot proof. Every variant carries a
/// message that is only ever surfaced through the derived `Debug` impl (a log
/// line or an `eprintln!`), which the dead-code checker does not count as a
/// read, hence the blanket allow.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BootProofError {
    Manifest(String),
    Encode(String),
    Nsm(String),
}

/// Where a response's `bootProof` comes from.
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
    ) -> Self {
        let (qos_manifest_b64, qos_manifest_envelope_b64) = read_manifest_borsh_b64();
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
            // Filled by the NSM-backed source in a later PR. Empty, never
            // faked: a strict verifier must reject an unattested response
            // outright.
            aws_attestation_doc_b64: String::new(),
            qos_manifest_b64: self.qos_manifest_b64.clone(),
            qos_manifest_envelope_b64: self.qos_manifest_envelope_b64.clone(),
            ephemeral_public_key_hex: self.ephemeral_public_key_hex.clone(),
            enclave_app: self.enclave_app.clone(),
            deployment_label: self.deployment_label.clone(),
        }
    }
}

/// `/qos.manifest` holds JSON at qos rev 365ba7ed, but the wallet contract's
/// `qosManifestB64` / `qosManifestEnvelopeB64` are *borsh* bytes: the Go
/// verifier borsh-deserializes both (visualsign-turnkeyclient
/// manifest/parser.go), and the attestation doc's `user_data` is
/// sha256(borsh(manifest)). Base64-ing the file bytes directly would produce
/// fields no verifier can read. So: read JSON, re-encode with borsh.
///
/// Shared by `StaticBootProof` and (in a later PR) an NSM-backed source,
/// which also needs the envelope for `manifest.qos_hash()`.
pub fn read_manifest_envelope() -> Result<ManifestEnvelope, BootProofError> {
    let contents = std::fs::read(qos_core::MANIFEST_FILE)
        .map_err(|e| BootProofError::Manifest(format!("{}: {e}", qos_core::MANIFEST_FILE)))?;
    serde_json::from_slice(&contents)
        .map_err(|e| BootProofError::Manifest(format!("manifest json: {e}")))
}

/// Returns empty strings when the manifest is unreadable (local dev outside an
/// enclave), which keeps the six keys present and obviously unverifiable.
fn read_manifest_borsh_b64() -> (String, String) {
    use base64::Engine as _;
    let Ok(envelope) = read_manifest_envelope() else {
        eprintln!(
            "boot proof: {} unreadable, manifest fields empty",
            qos_core::MANIFEST_FILE
        );
        return (String::new(), String::new());
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let manifest_b64 = borsh::to_vec(&envelope.manifest)
        .map(|b| engine.encode(b))
        .unwrap_or_default();
    let envelope_b64 = borsh::to_vec(&envelope)
        .map(|b| engine.encode(b))
        .unwrap_or_default();
    (manifest_b64, envelope_b64)
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
    aws_attestation_doc_b64: String,
    qos_manifest_b64: String,
    qos_manifest_envelope_b64: String,
    ephemeral_public_key_hex: String,
    enclave_app: String,
    deployment_label: String,
}

impl NsmBootProof {
    /// Production constructor: calls the real `/dev/nsm` device.
    pub fn new(
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        Self::with_attestor(qos_nsm::Nsm, ephemeral, enclave_app, deployment_label)
    }

    /// Test seam: takes any `NsmProvider` so unit tests can assert the
    /// attestor is called exactly once without touching `/dev/nsm`.
    pub fn with_attestor<A: qos_nsm::NsmProvider>(
        attestor: A,
        ephemeral: &P256Pair,
        enclave_app: String,
        deployment_label: String,
    ) -> Result<Self, BootProofError> {
        use base64::Engine as _;
        use qos_core::protocol::QosHash;
        use qos_nsm::types::{NsmRequest, NsmResponse};

        let envelope = read_manifest_envelope()?;
        let manifest_hash = envelope.manifest.qos_hash().to_vec();
        let ephemeral_public_key = ephemeral.public_key().to_bytes();

        let response = attestor.nsm_process_request(NsmRequest::Attestation {
            user_data: Some(manifest_hash),
            nonce: None,
            public_key: Some(ephemeral_public_key.clone()),
        });
        let document = match response {
            NsmResponse::Attestation { document } => document,
            other => return Err(BootProofError::Nsm(format!("{other:?}"))),
        };

        let engine = base64::engine::general_purpose::STANDARD;
        let qos_manifest_b64 = engine.encode(
            borsh::to_vec(&envelope.manifest)
                .map_err(|e| BootProofError::Encode(format!("manifest borsh: {e}")))?,
        );
        let qos_manifest_envelope_b64 = engine.encode(
            borsh::to_vec(&envelope)
                .map_err(|e| BootProofError::Encode(format!("envelope borsh: {e}")))?,
        );

        Ok(Self {
            aws_attestation_doc_b64: engine.encode(document),
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            ephemeral_public_key_hex: qos_hex::encode(&ephemeral_public_key),
            enclave_app,
            deployment_label,
        })
    }
}

impl BootProofSource for NsmBootProof {
    fn boot_proof(&self) -> TurnkeyBootProof {
        TurnkeyBootProof {
            aws_attestation_doc_b64: self.aws_attestation_doc_b64.clone(),
            qos_manifest_b64: self.qos_manifest_b64.clone(),
            qos_manifest_envelope_b64: self.qos_manifest_envelope_b64.clone(),
            ephemeral_public_key_hex: self.ephemeral_public_key_hex.clone(),
            enclave_app: self.enclave_app.clone(),
            deployment_label: self.deployment_label.clone(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use qos_nsm::NsmProvider;
    use qos_nsm::nitro::AttestError;
    use qos_nsm::types::{NsmRequest, NsmResponse};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// `read_manifest_envelope` reads a real file path (`qos_core::MANIFEST_FILE`),
    /// so the test writes a throwaway manifest there and removes it on drop.
    /// `ManifestEnvelope::default()` only exists under qos_core's "mock"
    /// feature (see this crate's `[dev-dependencies]`), which is exactly the
    /// scope this fixture needs it in.
    struct ManifestFixture;

    impl ManifestFixture {
        fn write() -> Self {
            let path = std::path::Path::new(qos_core::MANIFEST_FILE);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).unwrap();
            }
            let envelope = qos_core::protocol::services::boot::ManifestEnvelope::default();
            std::fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
            Self
        }
    }

    impl Drop for ManifestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(qos_core::MANIFEST_FILE);
        }
    }

    struct CountingAttestor {
        calls: Arc<AtomicUsize>,
        document: Vec<u8>,
    }

    impl NsmProvider for CountingAttestor {
        fn nsm_process_request(&self, _request: NsmRequest) -> NsmResponse {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
        let _fixture = ManifestFixture::write();
        let calls = Arc::new(AtomicUsize::new(0));
        let source = NsmBootProof::with_attestor(
            CountingAttestor {
                calls: calls.clone(),
                document: vec![0xAA; 64],
            },
            &qos_p256::P256Pair::generate().unwrap(),
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
    }
}
