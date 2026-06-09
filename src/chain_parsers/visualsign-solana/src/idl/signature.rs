//! IDL signature validation
//!
//! Mirrors `visualsign-ethereum::abi_metadata` for Solana IDL mappings. The
//! proto carries an optional `SignatureMetadata` on every `Idl` entry; when
//! present, we validate it as an ed25519 signature over a domain-separated
//! prehash that binds the program id to the IDL JSON bytes before accepting the
//! entry into the registry. The prehash is the shared v1 domain-separated,
//! length-prefixed construction defined in [`visualsign::signing`]; that module
//! documents the authoritative byte layout. The 32-byte prehash digest is the
//! message signed: signers reproduce it via
//! [`visualsign::signing::solana_metadata_prehash`] and ed25519-sign the
//! resulting 32 bytes (verified here with `verify_strict`), not the raw JSON
//! bytes directly.
//!
//! # Trust model: curator key, NOT the program's on-chain authority
//!
//! The signer is an off-chain, VisualSign-trusted *metadata curator* key. It is
//! deliberately **not** the Solana program's on-chain upgrade authority, and the
//! signature provides no binding to that authority. What a valid signature
//! attests is narrow and display-scoped: "a key VisualSign trusts has vouched
//! for this IDL as the correct decoder for this program id." It does not attest
//! that the program owner signed anything. The parser runs with no chain access,
//! so it cannot and does not check the IDL against the real on-chain authority;
//! trust comes solely from the curator allowlist below. ed25519 is used because
//! it is Solana's native curve (least surprise for anyone inspecting these
//! signatures), but the curve choice carries no curator-vs-authority meaning on
//! its own; that distinction lives in this documentation and the signer
//! custody, not in the algorithm.
//!
//! Behaviour parity with the Ethereum ABI path (which uses secp256k1 for its own
//! curator key, since EVM contracts have no native curve to match):
//! - Unsigned IDLs are accepted (graceful degradation). Callers that require
//!   mandatory signatures must enforce that at the API boundary.
//! - Algorithm must be `ed25519`. The proto is algorithm-agnostic, but we only
//!   accept ed25519 for the Solana IDL path.
//! - Signatures bind the program id. The prehash commits to the program id the
//!   IDL describes, so a signature minted for an IDL at one program id no longer
//!   verifies when replayed under a different program. Existing signatures must
//!   be re-issued.
//! - Signers are checked against an authorized allowlist. A verified signature
//!   only proves the IDL was signed by *some* ed25519 key, not an authorized
//!   one. When an IDL carries a signature it must verify AND the signer must
//!   appear in the allowlist (see [`authorized_idl_signers`]); both checks must
//!   pass. An EMPTY allowlist rejects every signed IDL (fail-closed). Unsigned
//!   IDLs remain accepted (graceful degradation), since the trusted-program and
//!   reserved-name guards in the extraction path already constrain them.
//! - Unlike the Ethereum ABI path, Solana has no exported dev signing key, so
//!   the allowlist has no compile-time dev entry: it is built solely from the
//!   env-configured production list.

use std::sync::OnceLock;

use ed25519_dalek::{Signature, VerifyingKey};
use visualsign::signing::SignerAllowlist;

/// The only supported signature algorithm.
const SUPPORTED_ALGORITHM: &str = "ed25519";

/// Length of an ed25519 public key in bytes.
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Length of an ed25519 signature in bytes.
const ED25519_SIGNATURE_LEN: usize = 64;

/// Error type for IDL signature validation.
#[derive(Debug, thiserror::Error)]
pub enum IdlSignatureError {
    #[error("IDL signature validation failed: {0}")]
    Validation(String),
}

/// IDL signature metadata for validation.
///
/// Mirrors the protobuf `SignatureMetadata` structure in a local type.
#[derive(Debug, Clone)]
pub struct SignatureMetadata {
    /// Signature value (hex-encoded, 64-byte raw ed25519 signature).
    pub value: String,
    /// Algorithm used (e.g., "ed25519").
    pub algorithm: Option<String>,
    /// Public key for signature verification (hex-encoded, 32-byte ed25519 key).
    pub public_key: Option<String>,
    /// Issuer of the signature (mirrors proto field; not used in validation).
    #[allow(dead_code)]
    pub issuer: Option<String>,
    /// Timestamp of signature (mirrors proto field; not used in validation).
    #[allow(dead_code)]
    pub timestamp: Option<String>,
}

/// Convert protobuf `SignatureMetadata` (key-value pairs) into the local
/// strongly-typed `SignatureMetadata`.
pub fn convert_proto_signature(proto: &generated::parser::SignatureMetadata) -> SignatureMetadata {
    let get = |key: &str| -> Option<String> {
        proto
            .metadata
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value.clone())
    };

    SignatureMetadata {
        value: proto.value.clone(),
        algorithm: get("algorithm"),
        public_key: get("public_key"),
        issuer: get("issuer"),
        timestamp: get("timestamp"),
    }
}

/// Decode an optionally `0x`/`0X`-prefixed hex string into a fixed-size byte
/// array, failing if the hex is malformed or the wrong length.
fn decode_hex_fixed<const N: usize>(value: &str, what: &str) -> Result<[u8; N], IdlSignatureError> {
    let trimmed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let bytes = hex::decode(trimmed)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid {what} hex: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        IdlSignatureError::Validation(format!(
            "Invalid {what} length: expected {N} bytes, got {}",
            v.len()
        ))
    })
}

/// Validate an IDL JSON string against an ed25519 signature, enforcing an
/// authorized-signer allowlist.
///
/// The signature must have been produced over the shared domain-separated
/// prehash that binds `program_id` to `idl_json` (see
/// [`visualsign::signing::solana_metadata_prehash`]). This function recomputes
/// that 32-byte prehash and verifies the signature against it as the signed
/// message via `verify_strict` (which rejects malleable / non-canonical
/// signatures and small-order keys). A signature is therefore valid only for the
/// exact program id it was produced for.
///
/// Both checks must pass: the signature must verify over the prehash AND the
/// signer's public key must appear in `allowlist`. An empty allowlist rejects
/// every signed IDL (fail-closed); see [`authorized_idl_signers`].
///
/// # Arguments
/// * `idl_json` - The IDL JSON string that was signed.
/// * `program_id` - The 32-byte program id the IDL is bound to.
/// * `signature` - Signature and metadata for validation.
/// * `allowlist` - Authorized signer public keys (32-byte ed25519 keys).
///
/// # Returns
/// * `Ok(())` if the signature verifies against the program-id-bound prehash and
///   the signer is authorized.
/// * `Err(IdlSignatureError)` if signature validation fails or the signer is not
///   in the allowlist.
pub fn validate_idl_signature(
    idl_json: &str,
    program_id: &[u8; 32],
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<(), IdlSignatureError> {
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| IdlSignatureError::Validation("Missing algorithm".to_string()))?;

    if algorithm != SUPPORTED_ALGORITHM {
        return Err(IdlSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {SUPPORTED_ALGORITHM} is supported."
        )));
    }

    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| IdlSignatureError::Validation("Missing public_key".to_string()))?;

    let hash = visualsign::signing::solana_metadata_prehash(program_id, idl_json.as_bytes());

    let sig_bytes = decode_hex_fixed::<ED25519_SIGNATURE_LEN>(&signature.value, "signature")?;
    let sig = Signature::from_bytes(&sig_bytes);

    let pubkey_bytes = decode_hex_fixed::<ED25519_PUBLIC_KEY_LEN>(public_key_hex, "public key")?;
    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid public key: {e}")))?;

    verifying_key.verify_strict(&hash, &sig).map_err(|e| {
        IdlSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    // Enforce the authorized-signer allowlist. A verified signature only proves
    // the IDL was signed by some ed25519 key; it must also be an authorized one.
    // ed25519 public keys are a canonical fixed 32 bytes (no compressed/
    // uncompressed variants), so the verified key's bytes are compared directly
    // against the allowlist. An empty allowlist contains nothing, so this rejects
    // every signed IDL (fail-closed).
    if !allowlist.contains(&verifying_key.to_bytes()) {
        return Err(IdlSignatureError::Validation(
            "signer not in allowlist".to_string(),
        ));
    }

    Ok(())
}

/// Build the authorized IDL-signer allowlist from the env-configured production
/// list.
///
/// The env var `VISUALSIGN_SOL_IDL_SIGNERS` (comma-separated hex ed25519 public
/// keys, 32 bytes each) populates the allowlist for configured deployments. Each
/// entry is validated as a real ed25519 point via [`canonical_pubkey_from_hex`]
/// before insertion. Invalid entries are logged and skipped.
///
/// Unlike the Ethereum ABI path, Solana has no exported dev signing key, so
/// there is intentionally NO compile-time dev entry: the allowlist is built
/// solely from the env var. When the env var is unset (or holds no valid keys)
/// the allowlist is empty, which rejects all signed IDLs (fail-closed). This is
/// the secure default for the untrusted, display-only caller-IDL path; unsigned
/// IDLs are unaffected and still accepted by the extraction path.
///
/// The allowlist is built once per process and cached: `VISUALSIGN_SOL_IDL_SIGNERS`
/// is read from the environment on first call (deployments set it before launch),
/// so the env read, hex decode, and ed25519 point validation happen a single time
/// rather than on every parse request. Returns a shared reference to the cached
/// allowlist.
#[must_use]
pub fn authorized_idl_signers() -> &'static SignerAllowlist {
    static SIGNERS: OnceLock<SignerAllowlist> = OnceLock::new();
    SIGNERS.get_or_init(|| {
        let mut allow = SignerAllowlist::new();

        if let Ok(list) = std::env::var("VISUALSIGN_SOL_IDL_SIGNERS") {
            for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                match canonical_pubkey_from_hex(entry) {
                    Some(bytes) => allow.insert(bytes),
                    None => tracing::warn!("Ignoring invalid pubkey in VISUALSIGN_SOL_IDL_SIGNERS"),
                }
            }
        }

        allow
    })
}

/// Parse a hex ed25519 public key (optionally `0x`- or `0X`-prefixed, exactly 32
/// bytes) and return its canonical bytes, or `None` if the input is not a valid
/// ed25519 point. ed25519 keys have a single canonical encoding, so validation
/// here just confirms the bytes decompress to a real point; the returned bytes
/// are the same 32 bytes that [`validate_idl_signature`] compares against.
fn canonical_pubkey_from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let bytes = decode_hex_fixed::<ED25519_PUBLIC_KEY_LEN>(hex_str, "public key").ok()?;
    let verifying_key = VerifyingKey::from_bytes(&bytes).ok()?;
    Some(verifying_key.to_bytes().to_vec())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use generated::parser::Metadata;

    const SAMPLE_IDL: &str = r#"{"metadata":{"name":"Real Program"},"instructions":[]}"#;

    /// Fixed 32-byte test program id used by signing-path tests.
    const TEST_PROGRAM_ID: [u8; 32] = [7u8; 32];

    /// Helper to create a valid signature for testing.
    ///
    /// The signature is over the shared domain-separated prehash binding
    /// `program_id` to `idl_json`, matching what [`validate_idl_signature`]
    /// verifies. Returns `(signature_hex, public_key_hex)`.
    fn create_test_signature(idl_json: &str, program_id: &[u8; 32]) -> (String, String) {
        let signing_key = SigningKey::from_bytes(&[0x42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let hash = visualsign::signing::solana_metadata_prehash(program_id, idl_json.as_bytes());

        let signature = signing_key.sign(&hash);
        let signature_hex = hex::encode(signature.to_bytes());
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        (signature_hex, public_key_hex)
    }

    /// Canonical public-key bytes derived from a 32-byte ed25519 seed.
    fn pubkey_bytes_from_seed(seed: &[u8; 32]) -> Vec<u8> {
        SigningKey::from_bytes(seed)
            .verifying_key()
            .to_bytes()
            .to_vec()
    }

    /// Allowlist authorizing the deterministic test signer (`create_test_signature`
    /// signs with seed `[0x42u8; 32]`). Built explicitly so the tests never depend
    /// on the env var. Non-empty so verified signatures from the test signer reach
    /// the verify step and pass.
    fn test_idl_signer_allowlist() -> SignerAllowlist {
        let mut allow = SignerAllowlist::new();
        allow.insert(pubkey_bytes_from_seed(&[0x42u8; 32]));
        allow
    }

    #[test]
    fn valid_signature_verifies() {
        let (sig_hex, pk_hex) = create_test_signature(SAMPLE_IDL, &TEST_PROGRAM_ID);
        let sig = SignatureMetadata {
            value: sig_hex,
            algorithm: Some("ed25519".to_string()),
            public_key: Some(pk_hex),
            issuer: None,
            timestamp: None,
        };
        assert!(
            validate_idl_signature(
                SAMPLE_IDL,
                &TEST_PROGRAM_ID,
                &sig,
                &test_idl_signer_allowlist()
            )
            .is_ok()
        );
    }

    /// A signature that verifies but whose signer is NOT in the allowlist is
    /// rejected.
    #[test]
    fn validate_idl_signature_rejects_unlisted_signer() {
        let (sig_hex, pk_hex) = create_test_signature(SAMPLE_IDL, &TEST_PROGRAM_ID);
        let sig = SignatureMetadata {
            value: sig_hex,
            algorithm: Some("ed25519".to_string()),
            public_key: Some(pk_hex),
            issuer: None,
            timestamp: None,
        };

        // Allowlist holds a DIFFERENT key (seed 0x43), so the seed-0x42 signer is
        // absent even though its signature verifies.
        let mut allow = SignerAllowlist::new();
        allow.insert(pubkey_bytes_from_seed(&[0x43u8; 32]));
        let result = validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig, &allow);
        assert!(
            result.is_err(),
            "a verified signature from an unlisted signer must be rejected"
        );
        assert!(
            result.unwrap_err().to_string().contains("not in allowlist"),
            "rejection must cite the allowlist check"
        );
    }

    /// An empty allowlist rejects every signed IDL, even a perfectly valid one
    /// (fail-closed default).
    #[test]
    fn validate_idl_signature_fails_closed_on_empty_allowlist() {
        let (sig_hex, pk_hex) = create_test_signature(SAMPLE_IDL, &TEST_PROGRAM_ID);
        let sig = SignatureMetadata {
            value: sig_hex,
            algorithm: Some("ed25519".to_string()),
            public_key: Some(pk_hex),
            issuer: None,
            timestamp: None,
        };

        let empty = SignerAllowlist::new();
        assert!(empty.is_empty(), "precondition: allowlist is empty");
        let result = validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig, &empty);
        assert!(
            result.is_err(),
            "an empty allowlist must reject all signed IDLs (fail-closed)"
        );
    }

    /// Core regression for this change: a signature is valid only for the exact
    /// program id it was produced for. Signing SAMPLE_IDL for program id A must
    /// verify under A but fail under a different program id B.
    #[test]
    fn validate_idl_signature_bound_to_program_id_rejects_replay() {
        let program_id_a: [u8; 32] = [7u8; 32];
        let program_id_b: [u8; 32] = [8u8; 32];

        let (sig_hex, pk_hex) = create_test_signature(SAMPLE_IDL, &program_id_a);
        let sig = SignatureMetadata {
            value: sig_hex,
            algorithm: Some("ed25519".to_string()),
            public_key: Some(pk_hex),
            issuer: None,
            timestamp: None,
        };

        let allow = test_idl_signer_allowlist();
        // Valid for the exact program id it was produced for.
        assert!(
            validate_idl_signature(SAMPLE_IDL, &program_id_a, &sig, &allow).is_ok(),
            "signature must verify for the bound program id"
        );
        // Different program id: rejected.
        assert!(
            validate_idl_signature(SAMPLE_IDL, &program_id_b, &sig, &allow).is_err(),
            "signature must not verify when replayed under a different program id"
        );
    }

    #[test]
    fn tampered_idl_rejected() {
        let (sig_hex, pk_hex) = create_test_signature(SAMPLE_IDL, &TEST_PROGRAM_ID);
        let sig = SignatureMetadata {
            value: sig_hex,
            algorithm: Some("ed25519".to_string()),
            public_key: Some(pk_hex),
            issuer: None,
            timestamp: None,
        };
        let tampered = r#"{"metadata":{"name":"Phantom Wallet"},"instructions":[]}"#;
        assert!(
            validate_idl_signature(
                tampered,
                &TEST_PROGRAM_ID,
                &sig,
                &test_idl_signer_allowlist()
            )
            .is_err()
        );
    }

    #[test]
    fn missing_algorithm_rejected() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: None,
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };
        let err = validate_idl_signature(
            SAMPLE_IDL,
            &TEST_PROGRAM_ID,
            &sig,
            &test_idl_signer_allowlist(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Missing algorithm"));
    }

    #[test]
    fn missing_public_key_rejected() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("ed25519".to_string()),
            public_key: None,
            issuer: None,
            timestamp: None,
        };
        let err = validate_idl_signature(
            SAMPLE_IDL,
            &TEST_PROGRAM_ID,
            &sig,
            &test_idl_signer_allowlist(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Missing public_key"));
    }

    #[test]
    fn unsupported_algorithm_rejected() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };
        let err = validate_idl_signature(
            SAMPLE_IDL,
            &TEST_PROGRAM_ID,
            &sig,
            &test_idl_signer_allowlist(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Unsupported algorithm"));
    }

    #[test]
    fn convert_proto_signature_maps_all_fields() {
        let proto = generated::parser::SignatureMetadata {
            value: "sig".to_string(),
            metadata: vec![
                Metadata {
                    key: "algorithm".to_string(),
                    value: "ed25519".to_string(),
                },
                Metadata {
                    key: "public_key".to_string(),
                    value: "abcd".to_string(),
                },
                Metadata {
                    key: "issuer".to_string(),
                    value: "test".to_string(),
                },
                Metadata {
                    key: "timestamp".to_string(),
                    value: "2026-01-01T00:00:00Z".to_string(),
                },
            ],
        };
        let local = convert_proto_signature(&proto);
        assert_eq!(local.value, "sig");
        assert_eq!(local.algorithm, Some("ed25519".to_string()));
        assert_eq!(local.public_key, Some("abcd".to_string()));
        assert_eq!(local.issuer, Some("test".to_string()));
        assert_eq!(local.timestamp, Some("2026-01-01T00:00:00Z".to_string()));
    }
}
