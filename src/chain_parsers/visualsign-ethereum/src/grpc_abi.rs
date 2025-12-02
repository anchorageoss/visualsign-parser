//! gRPC ABI metadata extraction and validation
//!
//! This module handles extracting ABIs from gRPC metadata payloads and validating them
//! using optional secp256k1 signatures.

use crate::abi_registry::AbiRegistry;
use crate::embedded_abis::{AbiEmbeddingError, register_embedded_abi};
use k256::EncodedPoint;
use k256::ecdsa::signature::Verifier;
use k256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Error type for gRPC ABI operations
#[derive(Debug, thiserror::Error)]
pub enum GrpcAbiError {
    /// Failed to parse ABI JSON
    #[error("Failed to parse ABI: {0}")]
    InvalidAbi(#[from] AbiEmbeddingError),

    /// Signature validation failed
    #[error("ABI signature validation failed: {0}")]
    SignatureValidation(String),

    /// Missing required metadata
    #[error("Missing ABI metadata")]
    MissingMetadata,
}

/// Extract and validate ABI from gRPC EthereumMetadata
///
/// # Arguments
/// * `abi_value` - JSON ABI string from Abi.value
/// * `signature` - Optional secp256k1 signature for validation
///
/// # Returns
/// * `Ok(AbiRegistry)` with the ABI registered as "wallet_provided"
/// * `Err(GrpcAbiError)` if ABI is invalid or signature validation fails
///
/// # Example
/// ```ignore
/// let metadata = ParseRequest { chain_metadata: Some(ChainMetadata { ... }) };
/// if let Some(chain) = &metadata.chain_metadata {
///     if let Some(ethereum) = &chain.ethereum {
///         if let Some(abi) = &ethereum.abi {
///             let registry = extract_abi_from_metadata(&abi.value, abi.signature.as_ref())?;
///             // Use registry in visualizer context
///         }
///     }
/// }
/// ```
pub fn extract_abi_from_metadata(
    abi_value: &str,
    signature: Option<&SignatureMetadata>,
) -> Result<AbiRegistry, GrpcAbiError> {
    // Validate signature if present
    if let Some(sig) = signature {
        validate_abi_signature(abi_value, sig)?;
    }

    // Create registry and register ABI
    let mut registry = AbiRegistry::new();
    register_embedded_abi(&mut registry, "wallet_provided", abi_value)?;

    Ok(registry)
}

/// Represents ABI signature metadata from gRPC
///
/// This mirrors the protobuf structure but is chain-agnostic
#[derive(Debug, Clone)]
pub struct SignatureMetadata {
    /// Signature value (hex-encoded, DER format for secp256k1)
    pub value: String,
    /// Algorithm used (e.g., "secp256k1")
    pub algorithm: Option<String>,
    /// Public key for signature verification (hex-encoded)
    pub public_key: Option<String>,
    /// Issuer of the signature
    pub issuer: Option<String>,
    /// Timestamp of signature
    pub timestamp: Option<String>,
}

/// Validate ABI using secp256k1 signature
///
/// # Arguments
/// * `abi_json` - The ABI JSON string that was signed
/// * `signature_metadata` - Signature and metadata for validation
///
/// # Returns
/// * `Ok(())` if signature is valid
/// * `Err(GrpcAbiError)` if signature validation fails
fn validate_abi_signature(
    abi_json: &str,
    signature: &SignatureMetadata,
) -> Result<(), GrpcAbiError> {
    // 1. Get algorithm - must be secp256k1
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| GrpcAbiError::SignatureValidation("Missing algorithm".to_string()))?;

    if algorithm != "secp256k1" {
        return Err(GrpcAbiError::SignatureValidation(format!(
            "Unsupported algorithm: {algorithm}. Only secp256k1 is supported."
        )));
    }

    // 2. Get public key
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| GrpcAbiError::SignatureValidation("Missing public_key".to_string()))?;

    // 3. Hash ABI JSON with SHA256
    let mut hasher = Sha256::new();
    hasher.update(abi_json.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();

    // 4. Decode signature (DER format) from hex
    let sig_bytes = hex::decode(&signature.value)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid signature hex: {e}")))?;

    let sig = Signature::from_der(&sig_bytes)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid DER signature: {e}")))?;

    // 5. Decode public key from hex
    let pubkey_bytes = hex::decode(public_key_hex)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid public key hex: {e}")))?;

    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid public key point: {e}")))?;

    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid verifying key: {e}")))?;

    // 6. Verify signature
    verifying_key.verify(&hash, &sig).map_err(|e| {
        GrpcAbiError::SignatureValidation(format!("Signature verification failed: {e}"))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::Signer;

    const VALID_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        }
    ]"#;

    /// Helper to create a valid signature for testing
    fn create_test_signature(content: &str) -> (String, String) {
        // Use a deterministic test seed
        let seed: [u8; 32] = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed).expect("valid key");
        let verifying_key = VerifyingKey::from(&signing_key);

        // Hash the content
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash: [u8; 32] = hasher.finalize().into();

        // Sign the hash
        let signature: Signature = signing_key.sign(&hash);
        let signature_der = signature.to_der();
        let signature_hex = hex::encode(signature_der.as_bytes());

        // Get public key (uncompressed format)
        let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

        (signature_hex, public_key_hex)
    }

    #[test]
    fn test_extract_abi_from_metadata_valid() {
        let result = extract_abi_from_metadata(VALID_ABI, None);
        assert!(result.is_ok());

        let registry = result.unwrap();
        assert!(registry.list_abis().contains(&"wallet_provided"));
    }

    #[test]
    fn test_extract_abi_from_metadata_invalid_json() {
        let result = extract_abi_from_metadata("not valid json", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_signature_verification() {
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI);

        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: Some("test".to_string()),
            timestamp: None,
        };

        let result = extract_abi_from_metadata(VALID_ABI, Some(&sig));
        assert!(
            result.is_ok(),
            "Valid signature should verify: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_tampering_detection() {
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI);

        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        // Try to verify with tampered ABI
        let tampered_abi = r#"[{"type":"function","name":"approve"}]"#;
        let result = extract_abi_from_metadata(tampered_abi, Some(&sig));
        assert!(result.is_err(), "Tampered content should fail verification");
    }

    #[test]
    fn test_missing_algorithm_error() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: None,
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &sig);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing algorithm"), "Error: {err}");
    }

    #[test]
    fn test_missing_public_key_error() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: None,
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &sig);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing public_key"), "Error: {err}");
    }

    #[test]
    fn test_unsupported_algorithm_error() {
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("ed25519".to_string()),
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &sig);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unsupported algorithm"), "Error: {err}");
    }

    #[test]
    fn test_invalid_signature_hex() {
        let sig = SignatureMetadata {
            value: "not_hex".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &sig);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid signature hex"), "Error: {err}");
    }

    #[test]
    fn test_signature_metadata_struct() {
        let sig = SignatureMetadata {
            value: "0xabc123".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: Some("04abcd1234".to_string()),
            issuer: Some("issuer.example.com".to_string()),
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        assert_eq!(sig.value, "0xabc123");
        assert_eq!(sig.algorithm, Some("secp256k1".to_string()));
        assert_eq!(sig.public_key, Some("04abcd1234".to_string()));
        assert_eq!(sig.issuer, Some("issuer.example.com".to_string()));
    }
}
