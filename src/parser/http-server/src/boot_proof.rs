//! Where a response's `bootProof` comes from.
//!
//! PR 3 ships [`StaticBootProof`] (real ephemeral key + real manifest bytes,
//! empty attestation doc); a later PR adds an NSM-backed implementation that
//! fills the attestation doc in.

use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use host_primitives::turnkey::TurnkeyBootProof;
use qos_core::protocol::services::boot::ManifestEnvelope;
use qos_p256::P256Pair;

/// Maximum allowed size for the QOS manifest file (10 MB), matching the
/// bounded-reader convention in `parser/cli-core/src/mapping_parser.rs`.
const MAX_MANIFEST_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Errors surfaced while assembling a boot proof. `Nsm` is declared here
/// (unused in this PR) so a later NSM-backed `BootProofSource` only has to
/// touch its own file, not this shared enum.
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
    ) -> Result<Self, BootProofError> {
        let (qos_manifest_b64, qos_manifest_envelope_b64) = read_manifest_borsh_b64()?;
        Ok(Self {
            ephemeral_public_key_hex: qos_hex::encode(&ephemeral.public_key().to_bytes()),
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            enclave_app,
            deployment_label,
        })
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
        Ok(Self {
            ephemeral_public_key_hex: qos_hex::encode(&ephemeral.public_key().to_bytes()),
            qos_manifest_b64,
            qos_manifest_envelope_b64,
            enclave_app,
            deployment_label,
        })
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
    let envelope = read_manifest_envelope()?;
    Ok((
        encode_borsh_b64(&envelope.manifest)?,
        encode_borsh_b64(&envelope)?,
    ))
}

#[cfg(test)]
fn read_manifest_borsh_b64_at(path: &Path) -> Result<(String, String), BootProofError> {
    let envelope = read_manifest_envelope_at(path)?;
    Ok((
        encode_borsh_b64(&envelope.manifest)?,
        encode_borsh_b64(&envelope)?,
    ))
}

fn encode_borsh_b64(v: &impl borsh::BorshSerialize) -> Result<String, BootProofError> {
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = borsh::to_vec(v).map_err(|e| BootProofError::Encode(format!("{e}")))?;
    Ok(engine.encode(bytes))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use qos_core::protocol::services::boot::{
        Manifest, ManifestSet, Namespace, NitroConfig, PatchSet, PivotConfig, RestartPolicy,
        ShareSet,
    };

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
}
