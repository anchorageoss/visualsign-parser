//! Where a response's `bootProof` comes from.
//!
//! PR 3 ships [`StaticBootProof`] (real ephemeral key + real manifest bytes,
//! empty attestation doc); a later PR adds an NSM-backed implementation that
//! fills the attestation doc in.

use host_primitives::turnkey::TurnkeyBootProof;
use qos_core::protocol::services::boot::ManifestEnvelope;
use qos_p256::P256Pair;

/// Errors surfaced while assembling a boot proof. `Encode` and `Nsm` are
/// declared here (unused in this PR) so a later NSM-backed `BootProofSource`
/// only has to touch its own file, not this shared enum.
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
