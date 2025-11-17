/// Test to validate SignatureMetadata concept with ABI and IDL
///
/// This test demonstrates how SignatureMetadata works:
/// 1. Content (ABI/IDL JSON) is stored separately from signature
/// 2. Signature is computed over the content using a specified algorithm
/// 3. SignatureMetadata contains algorithm, issuer, timestamp - but NOT included in signature computation
/// 4. Verification computes hash of content using algorithm from metadata and compares to signature
#[cfg(test)]
mod signature_metadata_validation {
    use sha2::{Digest, Sha256};

    /// Simulates the SignatureMetadata structure from proto
    #[derive(Debug, Clone)]
    struct Metadata {
        key: String,
        value: String,
    }

    #[derive(Debug, Clone)]
    struct SignatureMetadata {
        value: String,           // Signature of content hash
        metadata: Vec<Metadata>, // Algorithm, issuer, timestamp, etc.
    }

    /// Get metadata value by key
    fn get_metadata(sig_metadata: &SignatureMetadata, key: &str) -> Option<String> {
        sig_metadata
            .metadata
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value.clone())
    }

    /// Compute signature of content using specified algorithm
    fn compute_signature(content: &str, algorithm: &str) -> String {
        match algorithm {
            "SHA-256" => {
                let mut hasher = Sha256::new();
                hasher.update(content.as_bytes());
                format!("{:x}", hasher.finalize())
            }
            _ => panic!("Unknown algorithm: {algorithm}"),
        }
    }

    /// Verify signature metadata
    fn verify_signature(content: &str, sig_metadata: &SignatureMetadata) -> bool {
        let algorithm = match get_metadata(sig_metadata, "algorithm") {
            Some(algo) => algo,
            None => return false,
        };

        let computed_sig = compute_signature(content, &algorithm);
        computed_sig == sig_metadata.value
    }

    #[test]
    fn test_ethereum_abi_signature() {
        // Simulates Ethereum ABI content
        let abi_content = r#"[{"type":"function","name":"transfer","inputs":[{"name":"to","type":"address"},{"name":"amount","type":"uint256"}]}]"#;

        // Create signature metadata
        let sig_metadata = SignatureMetadata {
            value: compute_signature(abi_content, "SHA-256"),
            metadata: vec![
                Metadata {
                    key: "algorithm".to_string(),
                    value: "SHA-256".to_string(),
                },
                Metadata {
                    key: "issuer".to_string(),
                    value: "0x1234567890abcdef".to_string(),
                },
                Metadata {
                    key: "timestamp".to_string(),
                    value: "1699779600".to_string(),
                },
            ],
        };

        // Verify the signature
        assert!(
            verify_signature(abi_content, &sig_metadata),
            "ABI signature verification failed"
        );

        // Verify tampering is detected
        let tampered_content = r#"[{"type":"function","name":"approve","inputs":[]}]"#;
        assert!(
            !verify_signature(tampered_content, &sig_metadata),
            "Tampering not detected!"
        );
    }

    #[test]
    fn test_solana_idl_signature() {
        // Simulates Solana IDL content
        let idl_content = r#"{"version":"0.1.0","name":"example_program","instructions":[{"name":"initialize","accounts":[],"args":[]}]}"#;

        // Create signature metadata with ed25519 algorithm
        let sig_metadata = SignatureMetadata {
            value: compute_signature(idl_content, "SHA-256"),
            metadata: vec![
                Metadata {
                    key: "algorithm".to_string(),
                    value: "SHA-256".to_string(),
                },
                Metadata {
                    key: "issuer".to_string(),
                    value: "ExampleProgramAuthority111111111111".to_string(),
                },
                Metadata {
                    key: "timestamp".to_string(),
                    value: "1699779600".to_string(),
                },
            ],
        };

        // Verify the signature
        assert!(
            verify_signature(idl_content, &sig_metadata),
            "IDL signature verification failed"
        );

        // Verify metadata is NOT part of signature (can change without invalidating signature)
        // This demonstrates that metadata can be modified independently
        let mut modified_sig_metadata = sig_metadata.clone();
        modified_sig_metadata.metadata.push(Metadata {
            key: "additional_info".to_string(),
            value: "new_value".to_string(),
        });
        assert!(
            verify_signature(idl_content, &modified_sig_metadata),
            "Adding metadata should not invalidate signature"
        );
    }

    #[test]
    fn test_signature_metadata_generic_reusability() {
        // This test validates that SignatureMetadata is truly generic and works
        // the same way for any content

        let test_cases = vec![
            ("ethereum_abi", r#"[{"name":"transfer"}]"#),
            ("solana_idl", r#"{"name":"program"}"#),
            ("arbitrary_json", r#"{"key":"value"}"#),
        ];

        for (content_type, content) in test_cases {
            let sig = compute_signature(content, "SHA-256");
            let sig_metadata = SignatureMetadata {
                value: sig.clone(),
                metadata: vec![
                    Metadata {
                        key: "algorithm".to_string(),
                        value: "SHA-256".to_string(),
                    },
                    Metadata {
                        key: "content_type".to_string(),
                        value: content_type.to_string(),
                    },
                ],
            };

            assert!(
                verify_signature(content, &sig_metadata),
                "Failed for content type: {content_type}"
            );
            println!("✓ {content_type} signature verified");
        }
    }

    #[test]
    fn test_metadata_immutability() {
        // Demonstrates that changing the content invalidates the signature
        // even though the metadata remains the same

        let original_content = r#"{"program":"example"}"#;
        let modified_content = r#"{"program":"malicious"}"#;

        let sig_metadata = SignatureMetadata {
            value: compute_signature(original_content, "SHA-256"),
            metadata: vec![Metadata {
                key: "algorithm".to_string(),
                value: "SHA-256".to_string(),
            }],
        };

        assert!(verify_signature(original_content, &sig_metadata));
        assert!(
            !verify_signature(modified_content, &sig_metadata),
            "Should detect content modification"
        );
    }
}
