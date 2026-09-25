//! Where a response's `bootProof` comes from.
//!
//! [`StaticBootProof`] provides a real ephemeral key and real manifest bytes
//! with an empty attestation doc; a later NSM-backed implementation fills
//! the attestation doc in.

use std::path::Path;

use host_primitives::turnkey::TurnkeyBootProof;
use qos_p256::P256Pair;
use tvc_attestation::AttestationError;
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
}

impl From<AttestationError> for BootProofError {
    fn from(e: AttestationError) -> Self {
        match e {
            AttestationError::Encode(e) => Self::Encode(e),
            other => Self::Manifest(other.to_string()),
        }
    }
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
    use base64::Engine as _;
    use qos_core::protocol::services::boot::{
        Manifest, ManifestEnvelope, ManifestSet, Namespace, NitroConfig, PatchSet, PivotConfig,
        RestartPolicy, ShareSet,
    };

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
}
