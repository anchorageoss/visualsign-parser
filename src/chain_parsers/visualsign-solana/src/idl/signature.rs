//! IDL signature validation
//!
//! Mirrors `visualsign-ethereum::abi_metadata` for Solana IDL mappings. The
//! proto carries an optional `SignatureMetadata` on every `Idl` entry; when
//! present, we validate it as a secp256k1 ECDSA signature over a
//! domain-separated prehash that binds the program id to the IDL JSON bytes
//! (prehashed verification via `PrehashVerifier::verify_prehash`) before
//! accepting the entry into the registry. The prehash is the shared v1
//! domain-separated, length-prefixed construction defined in
//! [`visualsign::signing`]; that module documents the authoritative byte
//! layout. Signers must reproduce it via
//! [`visualsign::signing::solana_metadata_prehash`] and sign the resulting
//! 32-byte digest, not the raw JSON bytes directly.
//!
//! Behaviour parity with the Ethereum ABI path:
//! - Unsigned IDLs are accepted (graceful degradation). Callers that require
//!   mandatory signatures must enforce that at the API boundary.
//! - Algorithm must be `secp256k1`. The proto is algorithm-agnostic, but we
//!   only accept secp256k1 today so that wallets can rotate to a single trust
//!   anchor shared with the Ethereum ABI path.
//! - Signatures bind the program id. The prehash commits to the program id the
//!   IDL describes, so a signature minted for an IDL at one program id no
//!   longer verifies when replayed under a different program. Existing
//!   signatures must be re-issued.
//! - Signers are checked against an authorized allowlist. A verified signature
//!   only proves the IDL was signed by *some* secp256k1 key, not an authorized
//!   one. When an IDL carries a signature it must verify AND the signer must
//!   appear in the allowlist (see [`authorized_idl_signers`]); both checks must
//!   pass. An EMPTY allowlist rejects every signed IDL (fail-closed). Unsigned
//!   IDLs remain accepted (graceful degradation), since the trusted-program and
//!   reserved-name guards in the extraction path already constrain them.
//! - Unlike the Ethereum ABI path, Solana has no exported dev signing key, so
//!   the allowlist has no compile-time dev entry: it is built solely from the
//!   env-configured production list.

use k256::EncodedPoint;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};
use visualsign::signing::SignerAllowlist;

/// The only supported signature algorithm.
const SUPPORTED_ALGORITHM: &str = "secp256k1";

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
    /// Signature value (hex-encoded, DER format for secp256k1).
    pub value: String,
    /// Algorithm used (e.g., "secp256k1").
    pub algorithm: Option<String>,
    /// Public key for signature verification (hex-encoded).
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

/// Validate an IDL JSON string against a secp256k1 ECDSA signature, enforcing an
/// authorized-signer allowlist.
///
/// The signature must have been produced over the shared domain-separated
/// prehash that binds `program_id` to `idl_json` (see
/// [`visualsign::signing::solana_metadata_prehash`]). This function recomputes
/// that prehash and verifies the signature against the resulting 32-byte digest
/// via `PrehashVerifier::verify_prehash`. A signature is therefore valid only
/// for the exact program id it was produced for.
///
/// Both checks must pass: the signature must verify over the prehash AND the
/// recovered signer must appear in `allowlist`. An empty allowlist rejects every
/// signed IDL (fail-closed); see [`authorized_idl_signers`].
///
/// # Arguments
/// * `idl_json` - The IDL JSON string that was signed.
/// * `program_id` - The 32-byte program id the IDL is bound to.
/// * `signature` - Signature and metadata for validation.
/// * `allowlist` - Authorized signer public keys (canonical uncompressed bytes).
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

    let sig_hex = signature
        .value
        .strip_prefix("0x")
        .or_else(|| signature.value.strip_prefix("0X"))
        .unwrap_or(&signature.value);
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid signature hex: {e}")))?;

    let sig = Signature::from_der(&sig_bytes)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid DER signature: {e}")))?;

    let pubkey_hex = public_key_hex
        .strip_prefix("0x")
        .or_else(|| public_key_hex.strip_prefix("0X"))
        .unwrap_or(public_key_hex);
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid public key hex: {e}")))?;

    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid public key point: {e}")))?;

    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid verifying key: {e}")))?;

    verifying_key.verify_prehash(&hash, &sig).map_err(|e| {
        IdlSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    // Enforce the authorized-signer allowlist. A verified signature only proves
    // the IDL was signed by some secp256k1 key; it must also be an authorized
    // one. Compare on the canonical uncompressed SEC1 encoding so the lookup
    // matches how keys are stored in the allowlist. An empty allowlist contains
    // nothing, so this rejects every signed IDL (fail-closed).
    let pk = verifying_key.to_encoded_point(false).as_bytes().to_vec();
    if !allowlist.contains(&pk) {
        return Err(IdlSignatureError::Validation(
            "signer not in allowlist".to_string(),
        ));
    }

    Ok(())
}

/// Build the authorized IDL-signer allowlist from the env-configured production
/// list.
///
/// The env var `VISUALSIGN_SOL_IDL_SIGNERS` (comma-separated hex secp256k1
/// public keys, any SEC1 encoding) populates the allowlist for configured
/// deployments. Each entry is canonicalized to its uncompressed encoding via
/// [`canonical_pubkey_from_hex`], so compressed and uncompressed inputs for the
/// same key match. Invalid entries are logged and skipped.
///
/// Unlike the Ethereum ABI path, Solana has no exported dev signing key, so
/// there is intentionally NO compile-time dev entry: the allowlist is built
/// solely from the env var. When the env var is unset (or holds no valid keys)
/// the allowlist is empty, which rejects all signed IDLs (fail-closed). This is
/// the secure default for the untrusted, display-only caller-IDL path; unsigned
/// IDLs are unaffected and still accepted by the extraction path.
#[must_use]
pub fn authorized_idl_signers() -> SignerAllowlist {
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
}

/// Parse a hex secp256k1 public key (optionally `0x`- or `0X`-prefixed, any
/// SEC1 encoding) and return its canonical UNCOMPRESSED encoded-point bytes, or
/// `None` if the input is not a valid point. Canonicalizing here means a
/// compressed input and an uncompressed input for the same key both reduce to
/// identical allowlist bytes.
fn canonical_pubkey_from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let trimmed = hex_str
        .strip_prefix("0x")
        .or_else(|| hex_str.strip_prefix("0X"))
        .unwrap_or(hex_str);
    let bytes = hex::decode(trimmed).ok()?;
    let encoded_point = EncodedPoint::from_bytes(&bytes).ok()?;
    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point).ok()?;
    Some(verifying_key.to_encoded_point(false).as_bytes().to_vec())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::Metadata;
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    const SAMPLE_IDL: &str = r#"{"metadata":{"name":"Real Program"},"instructions":[]}"#;

    /// Fixed 32-byte test program id used by signing-path tests.
    const TEST_PROGRAM_ID: [u8; 32] = [7u8; 32];

    /// Helper to create a valid signature for testing.
    ///
    /// The signature is over the shared domain-separated prehash binding
    /// `program_id` to `idl_json`, matching what [`validate_idl_signature`]
    /// verifies.
    fn create_test_signature(idl_json: &str, program_id: &[u8; 32]) -> (String, String) {
        let seed: [u8; 32] = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed).expect("valid key");
        let verifying_key = VerifyingKey::from(&signing_key);

        let hash = visualsign::signing::solana_metadata_prehash(program_id, idl_json.as_bytes());

        let signature: Signature = signing_key.sign_prehash(&hash).expect("signing failed");
        let signature_hex = hex::encode(signature.to_der().as_bytes());
        let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

        (signature_hex, public_key_hex)
    }

    /// Canonical uncompressed public-key bytes derived from a 32-byte seed.
    fn pubkey_bytes_from_seed(seed: &[u8; 32]) -> Vec<u8> {
        let signing_key = SigningKey::from_bytes(seed).expect("valid key");
        let verifying_key = VerifyingKey::from(&signing_key);
        verifying_key.to_encoded_point(false).as_bytes().to_vec()
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("secp256k1".to_string()),
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
            algorithm: Some("ed25519".to_string()),
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
                    value: "secp256k1".to_string(),
                },
                Metadata {
                    key: "public_key".to_string(),
                    value: "04abcd".to_string(),
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
        assert_eq!(local.algorithm, Some("secp256k1".to_string()));
        assert_eq!(local.public_key, Some("04abcd".to_string()));
        assert_eq!(local.issuer, Some("test".to_string()));
        assert_eq!(local.timestamp, Some("2026-01-01T00:00:00Z".to_string()));
    }
}
