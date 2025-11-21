//! gRPC ABI metadata extraction and validation
//!
//! This module handles extracting ABIs from gRPC metadata payloads and validating them
//! using optional secp256k1 signatures.

use crate::abi_registry::AbiRegistry;
use crate::embedded_abis::{AbiEmbeddingError, register_embedded_abi};

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
    /// Signature value (hex-encoded)
    pub value: String,
    /// Algorithm used (e.g., "secp256k1-sha256")
    pub algorithm: Option<String>,
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
///
/// # Note
/// This is a placeholder for the actual signature validation logic.
/// The implementation would use:
/// - SHA256 hash of abi_json
/// - secp256k1::verify() with provided signature
/// - Recovery of public key from signature
fn validate_abi_signature(
    abi_json: &str,
    _signature: &SignatureMetadata,
) -> Result<(), GrpcAbiError> {
    // TODO: Implement actual secp256k1 signature validation
    // For now, just verify the ABI can be parsed
    serde_json::from_str::<serde_json::Value>(abi_json)
        .map_err(|e| GrpcAbiError::SignatureValidation(format!("Invalid ABI JSON: {e}")))?;

    // TODO: When secp256k1 validation is added:
    // 1. Hash the ABI JSON with SHA256
    // 2. Recover public key from signature
    // 3. Verify signature matches
    // 4. Log issuer and timestamp if present

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_extract_abi_from_metadata_valid() {
        let result = extract_abi_from_metadata(VALID_ABI, None);
        assert!(result.is_ok());

        let registry = result.unwrap();
        // Verify ABI was registered
        assert!(registry.list_abis().contains(&"wallet_provided"));
    }

    #[test]
    fn test_extract_abi_from_metadata_invalid_json() {
        let result = extract_abi_from_metadata("not valid json", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_abi_from_metadata_with_signature() {
        let sig = SignatureMetadata {
            value: "0x123456789abcdef".to_string(),
            algorithm: Some("secp256k1-sha256".to_string()),
            issuer: Some("wallet.example.com".to_string()),
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let result = extract_abi_from_metadata(VALID_ABI, Some(&sig));
        // Should succeed - signature validation is placeholder
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_abi_from_metadata_signature_with_invalid_abi() {
        let sig = SignatureMetadata {
            value: "0x123456789abcdef".to_string(),
            algorithm: None,
            issuer: None,
            timestamp: None,
        };

        let result = extract_abi_from_metadata("invalid json", Some(&sig));
        assert!(result.is_err());
    }

    #[test]
    fn test_signature_metadata_struct() {
        let sig = SignatureMetadata {
            value: "0xabc123".to_string(),
            algorithm: Some("secp256k1-sha256".to_string()),
            issuer: Some("issuer.example.com".to_string()),
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        assert_eq!(sig.value, "0xabc123");
        assert_eq!(sig.algorithm, Some("secp256k1-sha256".to_string()));
        assert_eq!(sig.issuer, Some("issuer.example.com".to_string()));
    }
}
