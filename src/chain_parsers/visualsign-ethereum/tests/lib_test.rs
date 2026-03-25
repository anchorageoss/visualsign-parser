use generated::parser::{Abi, ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignOptions};
use visualsign_ethereum::EthereumVisualSignConverter;
use visualsign_ethereum::transaction_string_to_visual_sign;

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
        let transaction_hex = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
            abi_registry: None,
        };

        let result = transaction_string_to_visual_sign(transaction_hex, options);

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
        let transaction_hex = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
            abi_registry: None,
        };

        let result = transaction_string_to_visual_sign(transaction_hex, options);

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
        abi_registry: None,
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
    use alloy_primitives::U256;
    use alloy_sol_types::{SolCall, sol};

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

    use alloy_consensus::TxEip1559;
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
    use alloy_rlp::Encodable;
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

    let mut abi_mappings = HashMap::new();
    abi_mappings.insert(
        unknown_contract.to_string(),
        Abi {
            value: abi_json.to_string(),
            signature: None,
        },
    );

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings,
            })),
        }),
        developer_config: None,
        abi_registry: None,
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
