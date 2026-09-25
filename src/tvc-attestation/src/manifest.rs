//! Reading `/qos.manifest` and encoding it for the boot proof.
//!
//! QOS 0.12.1 writes the manifest as JSON with string-encoded numerics and may
//! write any of three schemas (v0/v1/v2); decoding goes through
//! [`VersionedManifestEnvelope::try_from_slice_compat`], the same reader
//! qos_core uses.

use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use qos_core::protocol::services::boot::VersionedManifestEnvelope;

use crate::AttestationError;

/// Largest manifest file we'll read; bounds memory against a corrupted or
/// hostile file.
pub const MAX_MANIFEST_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Read and decode the manifest envelope at `path`, reading at most
/// [`MAX_MANIFEST_FILE_SIZE`] bytes.
///
/// # Errors
///
/// [`AttestationError::Manifest`] if the file can't be read, exceeds the size
/// bound, or doesn't decode as any QOS manifest schema.
pub fn read_manifest_envelope(path: &Path) -> Result<VersionedManifestEnvelope, AttestationError> {
    let file = std::fs::File::open(path)
        .map_err(|e| AttestationError::Manifest(format!("{}: {e}", path.display())))?;
    // `take` bounds the read even if the file grows between open and read.
    let mut contents = Vec::new();
    file.take(MAX_MANIFEST_FILE_SIZE + 1)
        .read_to_end(&mut contents)
        .map_err(|e| AttestationError::Manifest(format!("{}: {e}", path.display())))?;
    if contents.len() as u64 > MAX_MANIFEST_FILE_SIZE {
        return Err(AttestationError::Manifest(format!(
            "{} exceeds maximum size (> {MAX_MANIFEST_FILE_SIZE} bytes)",
            path.display()
        )));
    }
    decode_manifest_envelope(&contents)
}

/// Decode manifest envelope bytes in any QOS storage encoding.
///
/// # Errors
///
/// [`AttestationError::Manifest`] if the bytes aren't a v0/v1/v2 envelope.
pub fn decode_manifest_envelope(
    bytes: &[u8],
) -> Result<VersionedManifestEnvelope, AttestationError> {
    VersionedManifestEnvelope::try_from_slice_compat(bytes)
        .map_err(|e| AttestationError::Manifest(format!("undecodable: {e}")))
}

/// `"v0"` / `"v1"` / `"v2"`, for logs and diagnostics.
#[must_use]
pub fn version_label(envelope: &VersionedManifestEnvelope) -> &'static str {
    match envelope {
        VersionedManifestEnvelope::V2(_) => "v2",
        VersionedManifestEnvelope::V1(_) => "v1",
        VersionedManifestEnvelope::V0(_) => "v0",
    }
}

/// The manifest and its envelope, base64-encoded for the boot proof's
/// `qos_manifest_b64` / `qos_manifest_envelope_b64` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootProofManifest {
    pub manifest_b64: String,
    pub envelope_b64: String,
}

/// Encode `envelope` the way a verifier expects it for its schema:
/// - v2: QOS storage JSON. v2 is JSON-only (qos_core refuses to borsh it) and
///   its hash is the canonical-JSON hash; verifiers sniff JSON and take their
///   v2 path.
/// - v0/v1: borsh, byte-identical to what the boot proof has always sent.
///   (Their QOS storage encoding is JSON too, but verifiers' JSON path is
///   v2-only, so these must stay borsh.)
///
/// # Errors
///
/// [`AttestationError::Encode`] if serialization fails.
pub fn boot_proof_manifest(
    envelope: &VersionedManifestEnvelope,
) -> Result<BootProofManifest, AttestationError> {
    let encode_err = |e: std::io::Error| AttestationError::Encode(e.to_string());
    let manifest = envelope.clone().manifest();
    let (manifest_bytes, envelope_bytes) = match envelope {
        VersionedManifestEnvelope::V2(_) => (
            manifest.to_storage_vec().map_err(encode_err)?,
            envelope.to_storage_vec().map_err(encode_err)?,
        ),
        VersionedManifestEnvelope::V1(_) | VersionedManifestEnvelope::V0(_) => (
            borsh::to_vec(&manifest).map_err(encode_err)?,
            borsh::to_vec(envelope).map_err(encode_err)?,
        ),
    };
    let b64 = base64::engine::general_purpose::STANDARD;
    Ok(BootProofManifest {
        manifest_b64: b64.encode(manifest_bytes),
        envelope_b64: b64.encode(envelope_bytes),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use qos_core::protocol::services::boot::{
        Manifest, ManifestEnvelope, ManifestEnvelopeV2, ManifestSet, ManifestV2, ManifestVersion,
        Namespace, NitroConfig, PatchSet, PivotConfig, PivotConfigV2, RestartPolicy, ShareSet,
    };

    /// A v1 envelope with non-default numerics, so string-vs-number encoding
    /// matters (a threshold of `0` would hide it).
    pub(crate) fn sample_v1() -> ManifestEnvelope {
        ManifestEnvelope {
            manifest: Manifest {
                namespace: Namespace {
                    name: "visualsign-parser".to_string(),
                    nonce: 7,
                    quorum_key: vec![0x04; 65],
                },
                pivot: PivotConfig {
                    hash: [0x11; 32],
                    restart: RestartPolicy::Never,
                    bridge_config: Vec::new(),
                    debug_mode: false,
                    args: vec!["--port".to_string(), "3000".to_string()],
                },
                manifest_set: sample_set(),
                share_set: ShareSet {
                    threshold: 1,
                    members: Vec::new(),
                },
                enclave: sample_nitro(),
                patch_set: PatchSet {
                    threshold: 0,
                    members: Vec::new(),
                },
            },
            manifest_set_approvals: Vec::new(),
            share_set_approvals: Vec::new(),
        }
    }

    fn sample_set() -> ManifestSet {
        ManifestSet {
            threshold: 1,
            members: Vec::new(),
        }
    }

    fn sample_nitro() -> NitroConfig {
        NitroConfig {
            pcr0: vec![0xA0; 48],
            pcr1: vec![0xA1; 48],
            pcr2: vec![0xA2; 48],
            pcr3: vec![0xA3; 48],
            aws_root_certificate: vec![0x30; 8],
            qos_commit: "82d8e18b8875362cdb639f962b6a7f78bd42d320".to_string(),
        }
    }

    pub(crate) fn sample_v2() -> ManifestEnvelopeV2 {
        let v1 = sample_v1().manifest;
        ManifestEnvelopeV2 {
            manifest: ManifestV2 {
                version: ManifestVersion::V2,
                namespace: v1.namespace,
                pivot: PivotConfigV2 {
                    hash: v1.pivot.hash,
                    restart: v1.pivot.restart,
                    bridge_config: v1.pivot.bridge_config,
                    debug_mode: v1.pivot.debug_mode,
                    args: v1.pivot.args,
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

    fn b64_decode(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(s).unwrap()
    }

    #[test]
    fn decodes_v2_storage_json_with_string_numerics() {
        let bytes = VersionedManifestEnvelope::V2(sample_v2())
            .to_storage_vec()
            .unwrap();
        // The shape the old workspace qos pin choked on in production:
        // numerics as JSON strings.
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#""threshold":"1""#), "{text}");

        let decoded = decode_manifest_envelope(&bytes).unwrap();
        assert_eq!(version_label(&decoded), "v2");
        assert_eq!(decoded, VersionedManifestEnvelope::V2(sample_v2()));
    }

    #[test]
    fn decodes_v1_json_and_borsh() {
        let v1 = VersionedManifestEnvelope::V1(sample_v1());
        for bytes in [
            v1.to_storage_vec().unwrap(),
            borsh::to_vec(&sample_v1()).unwrap(),
        ] {
            let decoded = decode_manifest_envelope(&bytes).unwrap();
            assert_eq!(version_label(&decoded), "v1");
            assert_eq!(decoded.manifest_hash(), v1.manifest_hash());
        }
    }

    #[test]
    fn rejects_garbage_and_empty() {
        for bytes in [&b"not a manifest"[..], &[][..]] {
            assert!(matches!(
                decode_manifest_envelope(bytes),
                Err(AttestationError::Manifest(_))
            ));
        }
    }

    #[test]
    fn v1_boot_proof_encoding_is_unchanged_borsh() {
        // Must stay byte-identical to what the boot proof sent before QOS
        // 0.12.1, so existing v1 deployments verify exactly as they did.
        let envelope = sample_v1();
        let encoded =
            boot_proof_manifest(&VersionedManifestEnvelope::V1(envelope.clone())).unwrap();
        assert_eq!(
            b64_decode(&encoded.manifest_b64),
            borsh::to_vec(&envelope.manifest).unwrap()
        );
        assert_eq!(
            b64_decode(&encoded.envelope_b64),
            borsh::to_vec(&envelope).unwrap()
        );
    }

    #[test]
    fn v2_boot_proof_encoding_is_storage_json_that_round_trips() {
        let envelope = VersionedManifestEnvelope::V2(sample_v2());
        let encoded = boot_proof_manifest(&envelope).unwrap();

        let envelope_bytes = b64_decode(&encoded.envelope_b64);
        assert_eq!(
            envelope_bytes.first(),
            Some(&b'{'),
            "v2 envelope must be JSON"
        );
        assert_eq!(decode_manifest_envelope(&envelope_bytes).unwrap(), envelope);

        let manifest_bytes = b64_decode(&encoded.manifest_b64);
        assert_eq!(
            manifest_bytes.first(),
            Some(&b'{'),
            "v2 manifest must be JSON"
        );
    }

    #[test]
    fn read_enforces_size_bound() {
        let dir = std::env::temp_dir().join(format!("attestation-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let ok = dir.join("ok.manifest");
        std::fs::write(
            &ok,
            VersionedManifestEnvelope::V2(sample_v2())
                .to_storage_vec()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(version_label(&read_manifest_envelope(&ok).unwrap()), "v2");

        let big = dir.join("big.manifest");
        std::fs::write(&big, vec![b' '; (MAX_MANIFEST_FILE_SIZE + 1) as usize]).unwrap();
        let err = read_manifest_envelope(&big).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum size"), "{err}");

        let missing = read_manifest_envelope(&dir.join("missing")).unwrap_err();
        assert!(matches!(missing, AttestationError::Manifest(_)));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
