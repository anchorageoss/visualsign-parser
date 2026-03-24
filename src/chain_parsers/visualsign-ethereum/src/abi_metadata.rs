//! ABI metadata extraction and signature validation
//!
//! Converts `ChainMetadata` ABI mappings into an `AbiRegistry` and optionally
//! validates secp256k1 signatures attached to individual ABI entries.

use crate::abi_registry::AbiRegistry;
use crate::embedded_abis::register_embedded_abi;
use generated::parser::{ChainMetadata, chain_metadata};
use k256::EncodedPoint;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// Error type for ABI signature validation.
#[derive(Debug, thiserror::Error)]
enum AbiSignatureError {
    #[error("ABI signature validation failed: {0}")]
    Validation(String),
}

/// Extract and validate ABIs from `ChainMetadata`, if present.
///
/// Navigates `ChainMetadata -> Ethereum -> abi_mappings` and registers each ABI
/// with its contract address. Returns `Ok(None)` if the metadata doesn't contain
/// any Ethereum ABI mappings.
///
/// The `chain_id` is needed to register address-to-ABI mappings in the registry.
///
/// # Security notes
///
/// - **Unsigned ABIs are accepted.** If no signature is present in the mapping,
///   the ABI is registered without validation. This is by design for graceful
///   degradation — callers that require mandatory signatures should enforce this
///   at the API boundary before calling this function.
/// - **Any public key is accepted.** Signature validation proves the ABI was not
///   tampered with after signing, but does not verify the signer's identity.
///   To establish trust, callers should verify the public key against a known
///   allowlist before passing metadata to this function.
pub fn try_extract_from_chain_metadata(
    chain_metadata: Option<&ChainMetadata>,
    chain_id: u64,
) -> Option<AbiRegistry> {
    let chain_metadata = chain_metadata?;
    let chain_metadata::Metadata::Ethereum(ethereum) = chain_metadata.metadata.as_ref()? else {
        return None;
    };
    if ethereum.abi_mappings.is_empty() {
        // Fallback to legacy `abi` field for backwards compatibility.
        // Note: the legacy field has no contract address, so the ABI is registered
        // as "wallet_provided" without an address mapping. The decoder's
        // `get_abi_for_address` won't find it — callers that need address-based
        // lookup should migrate to `abi_mappings`. This fallback exists so the ABI
        // is at least available via `list_abis()` for tooling that iterates all ABIs.
        let legacy_abi = ethereum.abi.as_ref()?;
        let mut registry = AbiRegistry::new();
        if let Some(proto_sig) = legacy_abi.signature.as_ref() {
            let signature = convert_proto_signature(proto_sig);
            if let Err(e) = validate_abi_signature(&legacy_abi.value, &signature) {
                log::warn!("Legacy ABI signature validation failed: {e}");
                return None;
            }
        }
        match register_embedded_abi(&mut registry, "wallet_provided", &legacy_abi.value) {
            Ok(()) => return Some(registry),
            Err(e) => {
                log::warn!("Failed to register legacy ABI: {e}");
                return None;
            }
        }
    }

    let mut registry = AbiRegistry::new();
    for (address, abi) in &ethereum.abi_mappings {
        // Validate address first (cheap) before expensive signature/ABI operations
        let parsed_address = match address.parse::<alloy_primitives::Address>() {
            Ok(addr) => addr,
            Err(e) => {
                log::warn!("Skipping ABI mapping with invalid address '{address}': {e}");
                continue;
            }
        };

        // Validate signature if present
        if let Some(proto_sig) = abi.signature.as_ref() {
            let signature = convert_proto_signature(proto_sig);
            if let Err(e) = validate_abi_signature(&abi.value, &signature) {
                log::warn!(
                    "Skipping ABI mapping for '{address}': signature validation failed: {e}"
                );
                continue;
            }
        }

        match register_embedded_abi(&mut registry, address, &abi.value) {
            Ok(()) => {
                registry.map_address(chain_id, parsed_address, address);
            }
            Err(e) => {
                log::warn!("Skipping ABI mapping for '{address}': {e}");
            }
        }
    }
    if registry.list_abis().is_empty() {
        return None;
    }
    Some(registry)
}

/// Convert protobuf `SignatureMetadata` (key-value pairs) to local `SignatureMetadata`.
fn convert_proto_signature(proto: &generated::parser::SignatureMetadata) -> SignatureMetadata {
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

/// The only supported signature algorithm.
const SUPPORTED_ALGORITHM: &str = "secp256k1";

/// ABI signature metadata for validation
///
/// Mirrors the protobuf `SignatureMetadata` structure in a local type
#[derive(Debug, Clone)]
struct SignatureMetadata {
    /// Signature value (hex-encoded, DER format for secp256k1)
    value: String,
    /// Algorithm used (e.g., "secp256k1")
    algorithm: Option<String>,
    /// Public key for signature verification (hex-encoded)
    public_key: Option<String>,
    /// Issuer of the signature (mirrors proto field; not used in validation)
    #[allow(dead_code)]
    issuer: Option<String>,
    /// Timestamp of signature (mirrors proto field; not used in validation)
    #[allow(dead_code)]
    timestamp: Option<String>,
}

/// Validate ABI using secp256k1 signature
///
/// # Arguments
/// * `abi_json` - The ABI JSON string that was signed
/// * `signature_metadata` - Signature and metadata for validation
///
/// # Returns
/// * `Ok(())` if signature is valid
/// * `Err(AbiSignatureError)` if signature validation fails
fn validate_abi_signature(
    abi_json: &str,
    signature: &SignatureMetadata,
) -> Result<(), AbiSignatureError> {
    // 1. Get algorithm - must be secp256k1
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| AbiSignatureError::Validation("Missing algorithm".to_string()))?;

    if algorithm != SUPPORTED_ALGORITHM {
        return Err(AbiSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {SUPPORTED_ALGORITHM} is supported."
        )));
    }

    // 2. Get public key
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| AbiSignatureError::Validation("Missing public_key".to_string()))?;

    // 3. Hash ABI JSON with SHA256
    let mut hasher = Sha256::new();
    hasher.update(abi_json.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();

    // 4. Decode signature (DER format) from hex
    let sig_hex = signature
        .value
        .strip_prefix("0x")
        .unwrap_or(&signature.value);
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid signature hex: {e}")))?;

    let sig = Signature::from_der(&sig_bytes)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid DER signature: {e}")))?;

    // 5. Decode public key from hex
    let pubkey_hex = public_key_hex.strip_prefix("0x").unwrap_or(public_key_hex);
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid public key hex: {e}")))?;

    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid public key point: {e}")))?;

    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid verifying key: {e}")))?;

    // 6. Verify pre-hashed signature (hash was computed in step 3)
    verifying_key.verify_prehash(&hash, &sig).map_err(|e| {
        AbiSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use generated::parser::{Abi, EthereumMetadata, SolanaMetadata};
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

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

        // Sign the pre-hashed content
        let signature: Signature = signing_key.sign_prehash(&hash).expect("signing failed");
        let signature_der = signature.to_der();
        let signature_hex = hex::encode(signature_der.as_bytes());

        // Get public key (uncompressed format)
        let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

        (signature_hex, public_key_hex)
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

        let result = validate_abi_signature(VALID_ABI, &sig);
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
        let result = validate_abi_signature(tampered_abi, &sig);
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

    // --- try_extract_from_chain_metadata tests ---

    const TEST_ADDRESS: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";

    fn make_abi_mappings(entries: Vec<(&str, Abi)>) -> std::collections::BTreeMap<String, Abi> {
        entries
            .into_iter()
            .map(|(addr, abi)| (addr.to_string(), abi))
            .collect()
    }

    #[test]
    fn test_try_extract_no_metadata() {
        assert!(try_extract_from_chain_metadata(None, 1).is_none());
    }

    #[test]
    fn test_try_extract_non_ethereum_metadata() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                network_id: None,
                idl: None,
                idl_mappings: Default::default(),
            })),
        };
        assert!(try_extract_from_chain_metadata(Some(&metadata), 1).is_none());
    }

    #[test]
    fn test_try_extract_ethereum_without_abi() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: Default::default(),
            })),
        };
        assert!(try_extract_from_chain_metadata(Some(&metadata), 1).is_none());
    }

    #[test]
    fn test_try_extract_valid_abi() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: None,
                    },
                )]),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1).expect("should contain ABI");
        assert!(registry.list_abis().contains(&TEST_ADDRESS));
    }

    #[test]
    fn test_try_extract_valid_abi_with_signature() {
        use generated::parser::Metadata;

        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI);

        let proto_sig = generated::parser::SignatureMetadata {
            value: signature_hex.clone(),
            metadata: vec![
                Metadata {
                    key: "algorithm".to_string(),
                    value: "secp256k1".to_string(),
                },
                Metadata {
                    key: "public_key".to_string(),
                    value: public_key_hex.clone(),
                },
                Metadata {
                    key: "issuer".to_string(),
                    value: "test-issuer".to_string(),
                },
                Metadata {
                    key: "timestamp".to_string(),
                    value: "2024-01-01T00:00:00Z".to_string(),
                },
            ],
        };

        // Verify convert_proto_signature maps fields correctly
        let local_sig = convert_proto_signature(&proto_sig);
        assert_eq!(local_sig.value, signature_hex);
        assert_eq!(local_sig.algorithm, Some("secp256k1".to_string()));
        assert_eq!(local_sig.public_key, Some(public_key_hex));
        assert_eq!(local_sig.issuer, Some("test-issuer".to_string()));
        assert_eq!(
            local_sig.timestamp,
            Some("2024-01-01T00:00:00Z".to_string())
        );

        // Verify end-to-end: ABI with valid signature extracts successfully
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: Some(proto_sig),
                    },
                )]),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1).expect("should contain ABI");
        assert!(registry.list_abis().contains(&TEST_ADDRESS));
    }

    #[test]
    fn test_convert_proto_signature_missing_keys() {
        let proto_sig = generated::parser::SignatureMetadata {
            value: "sig_value".to_string(),
            metadata: vec![],
        };

        let local_sig = convert_proto_signature(&proto_sig);
        assert_eq!(local_sig.value, "sig_value");
        assert!(local_sig.algorithm.is_none());
        assert!(local_sig.public_key.is_none());
        assert!(local_sig.issuer.is_none());
        assert!(local_sig.timestamp.is_none());
    }

    #[test]
    fn test_try_extract_invalid_address_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: make_abi_mappings(vec![(
                    "not_an_address",
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: None,
                    },
                )]),
            })),
        };
        // Invalid entries are skipped; with no valid entries left, result is None
        assert!(try_extract_from_chain_metadata(Some(&metadata), 1).is_none());
    }

    #[test]
    fn test_try_extract_invalid_abi_json_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    Abi {
                        value: "not valid json".to_string(),
                        signature: None,
                    },
                )]),
            })),
        };
        // Invalid ABI JSON is skipped; with no valid entries left, result is None
        assert!(try_extract_from_chain_metadata(Some(&metadata), 1).is_none());
    }

    #[test]
    fn test_try_extract_mixed_valid_and_invalid() {
        let valid_address = TEST_ADDRESS;
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: make_abi_mappings(vec![
                    (
                        "not_an_address",
                        Abi {
                            value: VALID_ABI.to_string(),
                            signature: None,
                        },
                    ),
                    (
                        valid_address,
                        Abi {
                            value: VALID_ABI.to_string(),
                            signature: None,
                        },
                    ),
                ]),
            })),
        };
        // The valid entry should be registered; the invalid one skipped
        let registry = try_extract_from_chain_metadata(Some(&metadata), 1)
            .expect("should contain the valid ABI");
        assert!(registry.list_abis().contains(&valid_address));
    }
}
