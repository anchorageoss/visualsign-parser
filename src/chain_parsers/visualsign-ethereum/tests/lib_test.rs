use alloy_primitives::hex;
use std::fs;
use std::path::PathBuf;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::partial_transaction::Eip7702TransactionWrapper;
use visualsign_ethereum::transaction_string_to_visual_sign;

// Helper function to get fixture path
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

static FIXTURES: [&str; 2] = ["1559", "legacy"];

#[test]
fn test_with_fixtures() {
    // Get paths for all test cases
    let fixtures_dir = fixture_path("");

    for test_name in FIXTURES {
        let input_path = fixtures_dir.join(format!("{}.input", test_name));

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {:?}", input_path));

        // Parse the input to extract transaction data
        let transaction_hex = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: true,
        };

        let result = transaction_string_to_visual_sign(transaction_hex, options);

        let actual_output = match result {
            Ok(payload) => {
                // Format the payload as a debug string or custom format
                format!("{:#?}", payload)
            }
            Err(error) => {
                format!("Error: {:?}", error)
            }
        };

        // Construct expected output path
        let expected_path = fixtures_dir.join(format!("{}.expected", test_name));

        // Read expected output
        let expected_output = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {:?}", expected_path));

        assert_eq!(
            actual_output.trim(),
            expected_output.trim(),
            "Test case '{}' failed",
            test_name
        );
    }
}

#[test]
fn test_ethereum_charset_validation() {
    // Test that Ethereum parser produces ASCII-only output
    let fixtures_dir = fixture_path("");

    for test_name in FIXTURES {
        let input_path = fixtures_dir.join(format!("{}.input", test_name));

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {:?}", input_path));

        // Parse the input to extract transaction data
        let transaction_hex = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: false,
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
                        "Ethereum parser JSON should not contain unicode escape {} for test case '{}', but found in: {}",
                        escape,
                        test_name,
                        json_string.chars().take(200).collect::<String>()
                    );
                }

                // Verify the JSON is valid ASCII
                assert!(
                    json_string.is_ascii(),
                    "Ethereum parser JSON output should be ASCII only for test case '{}'",
                    test_name
                );
            }
            Err(error) => {
                // If parsing fails, that's okay for this test - we're only testing
                // that successful parses produce valid charsets
                eprintln!(
                    "Skipping charset validation for test case '{}' due to parse error: {:?}",
                    test_name, error
                );
            }
        }
    }
}

#[test]
fn test_empty_data_errors() {
    // Test that empty data returns an error instead of made-up values
    let empty_result = Eip7702TransactionWrapper::decode_partial(&[]);
    assert!(
        empty_result.is_err(),
        "Empty data should return error, not made-up values"
    );

    // Test that minimal invalid data returns error instead of made-up values
    let minimal_data = vec![0x01, 0x02];
    let minimal_result = Eip7702TransactionWrapper::decode_partial(&minimal_data);
    assert!(
        minimal_result.is_err(),
        "Minimal invalid data should return error, not made-up values"
    );

    // Test that completely invalid data returns error
    let invalid_data = vec![0xff; 10]; // All 0xff bytes
    let invalid_result = Eip7702TransactionWrapper::decode_partial(&invalid_data);
    assert!(
        invalid_result.is_err(),
        "Invalid data should return error, not made-up values"
    );

    println!("✅ All empty data tests passed - decoder properly errors on invalid input");
}

#[test]
fn test_eip7702_transaction_roundtrip() {
    // Test that we can create, encode, and decode an EIP-7702 transaction
    use visualsign_ethereum::partial_transaction::Eip7702TransactionWrapper;
    use alloy_consensus::TxEip7702;
    use alloy_primitives::{ChainId, Address, U256, Bytes};

    // Create a valid EIP-7702 transaction
    let original_tx = Eip7702TransactionWrapper::from_inner(TxEip7702 {
        chain_id: ChainId::from(17u64),
        nonce: 0,
        max_fee_per_gas: 23_000_000_000u128,
        max_priority_fee_per_gas: 20_000_000_000u128,
        gas_limit: 21000,
        to: Address::from_slice(&hex::decode("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap()),
        value: U256::from(1u64),
        input: Bytes::new(),
        access_list: Default::default(),
        authorization_list: Default::default(),
    });

    // Encode it
    let encoded = original_tx.encode_partial();
    let hex_data = format!("0x{}", hex::encode(&encoded));
    
    println!("Testing EIP-7702 transaction: {}", hex_data);
    println!("Encoded length: {}", encoded.len());

    // Decode it back
    let result = Eip7702TransactionWrapper::decode_partial(&encoded);
    println!("Decode result: {:?}", result);
    assert!(result.is_ok(), "EIP-7702 transaction should decode successfully");

    // Also test the hex function directly
    use visualsign_ethereum::partial_transaction::decode_eip7702_transaction_from_hex;
    let hex_result = decode_eip7702_transaction_from_hex(&hex_data);
    println!("Hex decode result: {:?}", hex_result);
    assert!(hex_result.is_ok(), "Hex decode should also work");
    
    // Verify they're equal
    assert_eq!(original_tx, result.unwrap());
    assert_eq!(original_tx, hex_result.unwrap());
}
