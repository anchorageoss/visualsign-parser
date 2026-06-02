//! IDL signature validation
//!
//! Mirrors `visualsign-ethereum::abi_metadata` for Solana IDL mappings. The
//! proto carries an optional `SignatureMetadata` on every `Idl` entry; when
//! present, we validate it as a secp256k1 ECDSA signature over a
//! domain-separated prehash that binds the program id to the IDL JSON bytes
//! (prehashed verification via `PrehashVerifier::verify_prehash`) before
//! accepting the entry into the registry. The prehash is the shared v1
//! construction in [`visualsign::signing`]:
//! `SHA-256(DOMAIN \0 "solana" \0 program_id \0 idl_json)`. Signers must
//! therefore reproduce that exact construction via
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
//! - Any public key is accepted; signatures prove the IDL was not tampered
//!   with after signing and is bound to this program id, not who the signer is.
//!   Identity must be established via an allowlist outside this module.

use k256::EncodedPoint;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};

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

/// Validate an IDL JSON string against a secp256k1 ECDSA signature.
///
/// The signature must have been produced over the shared domain-separated
/// prehash that binds `program_id` to `idl_json` (see
/// [`visualsign::signing::solana_metadata_prehash`]). This function recomputes
/// that prehash and verifies the signature against the resulting 32-byte digest
/// via `PrehashVerifier::verify_prehash`. A signature is therefore valid only
/// for the exact program id it was produced for.
///
/// # Arguments
/// * `idl_json` - The IDL JSON string that was signed.
/// * `program_id` - The 32-byte program id the IDL is bound to.
/// * `signature` - Signature and metadata for validation.
///
/// # Returns
/// * `Ok(())` if the signature verifies against the program-id-bound prehash.
/// * `Err(IdlSignatureError)` if any step of validation fails.
pub fn validate_idl_signature(
    idl_json: &str,
    program_id: &[u8; 32],
    signature: &SignatureMetadata,
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
        .unwrap_or(&signature.value);
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid signature hex: {e}")))?;

    let sig = Signature::from_der(&sig_bytes)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid DER signature: {e}")))?;

    let pubkey_hex = public_key_hex.strip_prefix("0x").unwrap_or(public_key_hex);
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid public key hex: {e}")))?;

    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid public key point: {e}")))?;

    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point)
        .map_err(|e| IdlSignatureError::Validation(format!("Invalid verifying key: {e}")))?;

    verifying_key.verify_prehash(&hash, &sig).map_err(|e| {
        IdlSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    Ok(())
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
        assert!(validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig).is_ok());
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

        // Valid for the exact program id it was produced for.
        assert!(
            validate_idl_signature(SAMPLE_IDL, &program_id_a, &sig).is_ok(),
            "signature must verify for the bound program id"
        );
        // Different program id: rejected.
        assert!(
            validate_idl_signature(SAMPLE_IDL, &program_id_b, &sig).is_err(),
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
        assert!(validate_idl_signature(tampered, &TEST_PROGRAM_ID, &sig).is_err());
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
        let err = validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig).unwrap_err();
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
        let err = validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig).unwrap_err();
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
        let err = validate_idl_signature(SAMPLE_IDL, &TEST_PROGRAM_ID, &sig).unwrap_err();
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
