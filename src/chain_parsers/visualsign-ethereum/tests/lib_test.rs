#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use alloy_consensus::TxEip1559;
use alloy_primitives::U256;
use alloy_rlp::Encodable;
use alloy_sol_types::{SolCall, sol};
use generated::parser::{
    Abi, ChainMetadata, EthereumMetadata, Metadata as ProtoMetadata, SignatureMetadata,
    chain_metadata::Metadata,
};
use k256::ecdsa::SigningKey;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignError, VisualSignOptions};
use visualsign_ethereum::EthereumVisualSignConverter;
use visualsign_ethereum::transaction_string_to_visual_sign;

/// Build a valid proto `SignatureMetadata` for `abi_json` using a deterministic
/// test key. Unsigned entries are rejected by the parser, so tests
/// that exercise the metadata-ABI path must attach a real signature.
fn sign_abi_for_test(abi_json: &str) -> SignatureMetadata {
    let seed: [u8; 32] = [0x42u8; 32];
    let signing_key = SigningKey::from_bytes(&seed).expect("valid signing key");
    let verifying_key = k256::ecdsa::VerifyingKey::from(&signing_key);

    let mut hasher = Sha256::new();
    hasher.update(abi_json.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    let signature: k256::ecdsa::Signature =
        signing_key.sign_prehash(&hash).expect("sign succeeded");
    let signature_hex = hex::encode(signature.to_der().as_bytes());
    let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

    SignatureMetadata {
        value: signature_hex,
        metadata: vec![
            ProtoMetadata {
                key: "algorithm".to_string(),
                value: "secp256k1".to_string(),
            },
            ProtoMetadata {
                key: "public_key".to_string(),
                value: public_key_hex,
            },
        ],
    }
}

// Helper function to get fixture path
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

static FIXTURES: [&str; 6] = [
    "1559",
    "legacy",
    "uniswap-v2swap",
    "uniswap-v3swap",
    "json-eip1559",
    "json-legacy",
];

#[test]
fn test_with_fixtures() {
    // Get paths for all test cases
    let fixtures_dir = fixture_path("");

    for test_name in FIXTURES {
        let input_path = fixtures_dir.join(format!("{test_name}.input"));

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {input_path:?}"));

        // Parse the input to extract transaction data
        let transaction_input = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
        };

        let result = transaction_string_to_visual_sign(transaction_input, options);

        let actual_output = match result {
            Ok(payload) => payload.to_json().unwrap(),
            Err(error) => format!("Error: {error:?}"),
        };

        // Construct expected output path
        let expected_path = fixtures_dir.join(format!("{test_name}.expected"));

        // Read expected output
        let expected_output = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {expected_path:?}"));

        assert_eq!(
            actual_output.trim(),
            expected_output.trim(),
            "Test case '{test_name}' failed",
        );
    }
}

#[test]
fn test_ethereum_charset_validation() {
    // Test that Ethereum parser produces ASCII-only output
    let fixtures_dir = fixture_path("");

    for test_name in FIXTURES {
        let input_path = fixtures_dir.join(format!("{test_name}.input"));

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {input_path:?}"));

        // Parse the input to extract transaction data
        let transaction_input = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
        };

        let result = transaction_string_to_visual_sign(transaction_input, options);

        match result {
            Ok(payload) => {
                // Test charset validation
                let validation_result = payload.validate_charset();
                assert!(
                    validation_result.is_ok(),
                    "Ethereum parser should produce ASCII-only output for test case '{}', got validation error: {:?}",
                    test_name,
                    validation_result.err()
                );

                // Test that to_validated_json works
                let json_result = payload.to_validated_json();
                assert!(
                    json_result.is_ok(),
                    "Ethereum parser output should serialize with charset validation for test case '{}', got error: {:?}",
                    test_name,
                    json_result.err()
                );

                let json_string = json_result.unwrap();

                // Verify specific unicode escapes are not present
                let unicode_escapes = vec!["\\u003e", "\\u003c", "\\u0026", "\\u0027", "\\u002b"];
                for escape in unicode_escapes {
                    assert!(
                        !json_string.contains(escape),
                        "Ethereum parser JSON should not contain unicode escape {escape} for test case '{test_name}', but found in: {}",
                        json_string.chars().take(200).collect::<String>()
                    );
                }

                // Verify the JSON is valid ASCII
                assert!(
                    json_string.is_ascii(),
                    "Ethereum parser JSON output should be ASCII only for test case '{test_name}'",
                );
            }
            Err(error) => {
                // If parsing fails, that's okay for this test - we're only testing
                // that successful parses produce valid charsets
                eprintln!(
                    "Skipping charset validation for test case '{test_name}' due to parse error: {error:?}",
                );
            }
        }
    }
}

#[test]
fn test_trait_path_without_abi_metadata() {
    // Verify the trait path with no ABI metadata works correctly
    let fixtures_dir = fixture_path("");
    let input_path = fixtures_dir.join("1559.input");
    let transaction_hex = fs::read_to_string(&input_path).unwrap();
    let transaction_hex = transaction_hex.trim();

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: None,
        developer_config: None,
    };

    let result = transaction_string_to_visual_sign(transaction_hex, options).unwrap();
    assert_eq!(result.title, "Ethereum Transaction");
}

#[test]
fn test_abi_from_metadata_decodes_function() {
    // Verify that ABIs provided via metadata.abi_mappings are used for decoding.
    //
    // We use an ERC-20 transfer calldata (selector 0xa9059cbb) sent to a contract
    // address that is NOT in the built-in visualizer registry, so the dynamic ABI
    // path via metadata is the only way the function name can appear in the output.
    sol! {
        function transfer(address to, uint256 amount) external returns (bool);
    }

    let recipient: alloy_primitives::Address = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    let calldata = transferCall {
        to: recipient,
        amount: U256::from(1_000_000u64),
    }
    .abi_encode();

    // Build a minimal EIP-1559 transaction to an unknown contract address
    let unknown_contract: alloy_primitives::Address = "0x1111111111111111111111111111111111111111"
        .parse()
        .unwrap();

    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 100_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: alloy_primitives::TxKind::Call(unknown_contract),
        input: calldata.into(),
        ..Default::default()
    };

    // RLP-encode as unsigned EIP-1559 (type 0x02)
    let mut buf = Vec::new();
    buf.push(0x02); // EIP-1559 type byte
    tx.encode(&mut buf);
    let tx_hex = format!("0x{}", hex::encode(&buf));

    // Provide ABI via metadata abi_mappings -- the standard path
    let abi_json = r#"[{
        "type": "function",
        "name": "transfer",
        "inputs": [
            {"name": "to", "type": "address"},
            {"name": "amount", "type": "uint256"}
        ],
        "outputs": [{"name": "", "type": "bool"}],
        "stateMutability": "nonpayable"
    }]"#;

    let signature = sign_abi_for_test(abi_json);
    let mut abi_mappings = BTreeMap::new();
    abi_mappings.insert(
        unknown_contract.to_string(),
        Abi {
            value: abi_json.to_string(),
            signature: Some(signature),
            ..Default::default()
        },
    );

    // Boundary conversion: proto `EthereumMetadata.abi_mappings` is still
    // `HashMap` in generated code. The crate-wide rule (clippy.toml) keeps us
    // on `BTreeMap` internally and we collect at the FFI point.
    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: abi_mappings.into_iter().collect(),
            })),
        }),
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let result = converter
        .to_visual_sign_payload_from_string(&tx_hex, options)
        .unwrap();

    // The ABI from metadata should decode the function name.
    // Without abi_mappings, this address is unknown and would show raw hex.
    let json = result.to_json().unwrap();
    assert!(
        json.contains("transfer"),
        "Payload should contain decoded function name 'transfer' from metadata ABI, got: {json}"
    );
    assert!(
        !json.contains("a9059cbb"),
        "Raw selector should not appear when ABI is decoded, got: {json}"
    );
}

#[test]
fn test_proxy_decodes_via_implementation_abi() {
    // A transaction to a proxy address should be decoded against the linked
    // implementation's ABI, surface a "Proxy" badge on the To field, and show the
    // resolved implementation address.
    sol! {
        function transfer(address to, uint256 amount) external returns (bool);
    }

    let recipient: alloy_primitives::Address = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    let calldata = transferCall {
        to: recipient,
        amount: U256::from(1_000_000u64),
    }
    .abi_encode();

    let proxy: alloy_primitives::Address = "0x1111111111111111111111111111111111111111"
        .parse()
        .unwrap();
    let implementation: alloy_primitives::Address = "0x2222222222222222222222222222222222222222"
        .parse()
        .unwrap();

    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 100_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: alloy_primitives::TxKind::Call(proxy),
        input: calldata.into(),
        ..Default::default()
    };
    let mut buf = Vec::new();
    buf.push(0x02);
    tx.encode(&mut buf);
    let tx_hex = format!("0x{}", hex::encode(&buf));

    let impl_abi_json = r#"[{
        "type": "function",
        "name": "transfer",
        "inputs": [
            {"name": "to", "type": "address"},
            {"name": "amount", "type": "uint256"}
        ],
        "outputs": [{"name": "", "type": "bool"}],
        "stateMutability": "nonpayable"
    }]"#;

    // Proxy entry carries an empty ABI plus the implementation link; the
    // implementation entry carries the real decoding ABI.
    let mut abi_mappings = BTreeMap::new();
    abi_mappings.insert(
        proxy.to_string(),
        Abi {
            value: "[]".to_string(),
            signature: Some(sign_abi_for_test("[]")),
            abi_type: Some(generated::parser::AbiType::Proxy as i32),
            implementation_address: Some(implementation.to_string()),
        },
    );
    abi_mappings.insert(
        implementation.to_string(),
        Abi {
            value: impl_abi_json.to_string(),
            signature: Some(sign_abi_for_test(impl_abi_json)),
            ..Default::default()
        },
    );

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: abi_mappings.into_iter().collect(),
            })),
        }),
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let json = converter
        .to_visual_sign_payload_from_string(&tx_hex, options)
        .unwrap()
        .to_json()
        .unwrap();

    assert!(
        json.contains("transfer"),
        "Proxy call should decode the implementation function 'transfer', got: {json}"
    );
    assert!(
        json.contains("Proxy"),
        "Proxy destination should carry a 'Proxy' badge, got: {json}"
    );
    assert!(
        json.contains(&implementation.to_string()),
        "Output should show the resolved implementation address, got: {json}"
    );
    assert!(
        !json.contains("a9059cbb"),
        "Raw selector should not appear when the implementation ABI decodes, got: {json}"
    );
}

#[test]
fn test_proxy_entry_cannot_override_canonical_token() {
    // A caller-supplied "proxy" entry mapped onto a canonical token address (USDC)
    // must NOT redirect decoding: the known-token short-circuit runs first and wins.
    sol! {
        function evil(address spender, uint256 amount) external;
    }

    let usdc: alloy_primitives::Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        .parse()
        .unwrap();
    let attacker_impl: alloy_primitives::Address = "0x3333333333333333333333333333333333333333"
        .parse()
        .unwrap();

    // The protection keys off the destination ADDRESS (USDC is in the global
    // canonical-token registry), not the selector: `evilCall` has its own selector
    // the built-in ERC20 visualizer does not recognize, so the known-token
    // short-circuit emits a raw-hex fallback and locks out the caller proxy ABI.
    let calldata = evilCall {
        spender: attacker_impl,
        amount: U256::from(1u64),
    }
    .abi_encode();

    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 100_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: alloy_primitives::TxKind::Call(usdc),
        input: calldata.into(),
        ..Default::default()
    };
    let mut buf = Vec::new();
    buf.push(0x02);
    tx.encode(&mut buf);
    let tx_hex = format!("0x{}", hex::encode(&buf));

    let evil_abi = r#"[{"type":"function","name":"evil","inputs":[{"name":"spender","type":"address"},{"name":"amount","type":"uint256"}],"outputs":[],"stateMutability":"nonpayable"}]"#;
    let mut abi_mappings = BTreeMap::new();
    abi_mappings.insert(
        usdc.to_string(),
        Abi {
            value: evil_abi.to_string(),
            // Signed so the entry survives extraction: the test must prove the
            // known-token short-circuit beats a *valid* proxy entry, not that an
            // unsigned entry is dropped.
            signature: Some(sign_abi_for_test(evil_abi)),
            abi_type: Some(generated::parser::AbiType::Proxy as i32),
            implementation_address: Some(attacker_impl.to_string()),
        },
    );

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: abi_mappings.into_iter().collect(),
            })),
        }),
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let json = converter
        .to_visual_sign_payload_from_string(&tx_hex, options)
        .unwrap()
        .to_json()
        .unwrap();

    assert!(
        !json.contains("evil"),
        "Canonical token must not be decoded with a caller-supplied proxy ABI, got: {json}"
    );
}

/// Regression test: a wallet-supplied `chain_metadata.network_id` must not
/// override the chain_id encoded in the transaction bytes. If the two disagree,
/// the parser refuses to produce a payload. Otherwise an attacker could trick a
/// wallet into displaying "Polygon, 1 POL" while the transaction bytes actually
/// transfer 1 ETH on Ethereum mainnet.
#[test]
fn test_chain_id_mismatch_rejected() {
    // Transaction bytes declare chain_id = 1 (Ethereum mainnet).
    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(
            "0x000000000000000000000000000000000000dEaD"
                .parse()
                .unwrap(),
        ),
        value: U256::from(1_000_000_000_000_000_000u64), // 1 ETH
        ..Default::default()
    };
    let mut buf = Vec::new();
    buf.push(0x02); // EIP-1559 type byte
    tx.encode(&mut buf);
    let tx_hex = format!("0x{}", hex::encode(&buf));

    // Metadata claims POLYGON_MAINNET (chain_id = 137), which disagrees with
    // the tx-declared chain_id in the transaction bytes. Parser must refuse.
    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("POLYGON_MAINNET".to_string()),
                abi_mappings: Default::default(),
            })),
        }),
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let err = converter
        .to_visual_sign_payload_from_string(&tx_hex, options)
        .expect_err("mismatched network_id vs tx-declared chain_id must be rejected");

    let msg = err.to_string();
    assert!(
        msg.contains("chain_id mismatch"),
        "error should mention chain_id mismatch, got: {msg}"
    );
    // Assert on explicit "chain_id N" substrings to avoid the substring trap
    // where "137" already contains '1'. "chain_id 1 " (trailing space) uniquely
    // identifies the tx-declared id, "chain_id 137" the metadata-derived id.
    assert!(
        msg.contains("chain_id 1 "),
        "error should reference tx-declared chain_id 1, got: {msg}"
    );
    assert!(
        msg.contains("chain_id 137"),
        "error should reference metadata chain_id 137, got: {msg}"
    );
}

/// Sibling to `test_chain_id_mismatch_rejected`: when metadata agrees with the
/// chain_id declared in the transaction bytes, parsing succeeds. Guards against
/// an over-eager rejection regression.
#[test]
fn test_chain_id_matching_metadata_succeeds() {
    let tx = TxEip1559 {
        chain_id: 137,
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(
            "0x000000000000000000000000000000000000dEaD"
                .parse()
                .unwrap(),
        ),
        value: U256::from(1_000_000_000_000_000_000u64),
        ..Default::default()
    };
    let mut buf = Vec::new();
    buf.push(0x02);
    tx.encode(&mut buf);
    let tx_hex = format!("0x{}", hex::encode(&buf));

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("POLYGON_MAINNET".to_string()),
                abi_mappings: Default::default(),
            })),
        }),
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let payload = converter
        .to_visual_sign_payload_from_string(&tx_hex, options)
        .expect("matching network_id and chain_id should parse cleanly");
    let json = payload.to_json().unwrap();
    assert!(
        json.contains("Polygon Mainnet"),
        "Polygon network label should be rendered when both inputs agree, got: {json}"
    );
}

#[test]
fn test_non_ascii_payload_is_rejected_by_converter() {
    // Regression for PRS-224: the Ethereum override of
    // `to_visual_sign_payload_from_string` previously called the
    // non-validated converter, so any non-ASCII text that reached the
    // rendered payload was emitted into the signed JSON. Every other chain
    // converter uses the default impl that runs charset validation; the
    // Ethereum override must enforce the same invariant.
    //
    // The ticket's stated PoC uses wallet-supplied ABI `function.name` /
    // parameter `input.name`. Today upstream `alloy_json_abi` validates
    // those as Solidity identifiers and rejects U+202E before our code sees
    // it, so we exercise the invariant via `VisualSignOptions::transaction_name`,
    // which lands directly in `payload.title` with no upstream filtering.
    // Either way the converter must reject non-ASCII content, since the
    // invariant must not depend on which input field carries it.
    let fixtures_dir = fixture_path("");
    let input_path = fixtures_dir.join("1559.input");
    let transaction_hex = fs::read_to_string(&input_path)
        .expect("Failed to read 1559.input fixture")
        .trim()
        .to_string();

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: Some("Send \u{202E}evil".to_string()),
        metadata: None,
        developer_config: None,
    };

    let converter = EthereumVisualSignConverter::new();
    let result = converter.to_visual_sign_payload_from_string(&transaction_hex, options);

    match result {
        Ok(payload) => panic!(
            "Expected non-ASCII title to be rejected by charset validation, got payload: {}",
            payload.to_json().unwrap_or_default()
        ),
        Err(VisualSignError::ValidationError(_)) => {
            // Expected: charset validation rejected the non-ASCII payload.
        }
        Err(other) => panic!("Expected VisualSignError::ValidationError, got: {other:?}"),
    }
}
