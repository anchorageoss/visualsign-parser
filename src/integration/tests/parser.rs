use generated::health::{AppHealthRequest, AppHealthResponse};
use generated::parser::{Chain, ParseRequest};
use integration::TestArgs;
use tonic::Code;

/// Recursively validates that all fields in expected are present in actual
/// This catches missing fields but allows extra fields in actual implementation.
/// Instead of complicating this further, I'm focusing to ensure that the expected field texts are correct first
fn validate_json_structure(actual: &serde_json::Value, expected: &serde_json::Value, path: &str) {
    match (actual, expected) {
        (serde_json::Value::Object(actual_map), serde_json::Value::Object(expected_map)) => {
            for (key, expected_value) in expected_map {
                let current_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };

                let actual_value = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("Missing field '{current_path}' in actual JSON"));

                validate_json_structure(actual_value, expected_value, &current_path);
            }
        }
        (serde_json::Value::Array(actual_arr), serde_json::Value::Array(expected_arr)) => {
            assert_eq!(
                actual_arr.len(),
                expected_arr.len(),
                "Array length mismatch at '{}': expected {}, got {}",
                path,
                expected_arr.len(),
                actual_arr.len()
            );

            for (i, (actual_item, expected_item)) in
                actual_arr.iter().zip(expected_arr.iter()).enumerate()
            {
                let current_path = format!("{path}[{i}]");
                validate_json_structure(actual_item, expected_item, &current_path);
            }
        }
        _ => {
            assert_eq!(
                actual, expected,
                "Value mismatch at '{path}': expected {expected:?}, got {actual:?}",
            );
        }
    }
}

/// Validates that actual contains at least all fields from expected (strict subset check)
fn validate_required_fields_present(actual: &serde_json::Value, expected: &serde_json::Value) {
    validate_json_structure(actual, expected, "");
}

/// Validates that the JSON string only contains safe ASCII characters to prevent unicode confusion
fn validate_safe_charset(json_str: &str) {
    // Check for unicode escapes
    assert!(
        !json_str.contains("\\u"),
        "JSON output contains unicode escape sequences: {json_str}",
    );

    // Use Rust's built-in ASCII validation - much simpler and more reliable
    assert!(
        json_str.is_ascii(),
        "JSON output contains non-ASCII characters: {json_str}",
    );

    // Additional validation for printable characters (optional - can be more restrictive)
    for (i, ch) in json_str.char_indices() {
        if !ch.is_ascii_graphic() && !ch.is_ascii_whitespace() {
            panic!(
                "JSON output contains non-printable character '{}' (U+{:02X}) at position {}: {}",
                ch.escape_default(),
                ch as u32,
                i,
                &json_str[i.saturating_sub(20)..std::cmp::min(i + 20, json_str.len())]
            );
        }
    }
}

// XXX: if you're iterating on these tests and the underlying code, make sure you run `cargo build --all`.
// Otherwise, Rust will not recompile the app binaries used here.
// You can also use `make test`, which takes care of recompiling the binaries before running the tests.

#[tokio::test]
async fn parser_e2e() {
    async fn test(test_args: TestArgs) {
        let parse_request = ParseRequest {
            unsigned_payload: "unsignedpayload".to_string(),
            chain: Chain::Unspecified as i32,
            chain_metadata: None,
        };

        let parse_response = test_args
            .parser_client
            .unwrap()
            .parse(tonic::Request::new(parse_request))
            .await
            .unwrap()
            .into_inner();

        let parsed_transaction = parse_response.parsed_transaction.unwrap().payload.unwrap();
        assert_eq!(
            parsed_transaction.signable_payload,
            "{\"Fields\":[{\"FallbackText\":\"Unspecified Chain\",\"Label\":\"Network\",\"TextV2\":{\"Text\":\"Unspecified Chain\"},\"Type\":\"text_v2\"},{\"FallbackText\":\"Raw Data\",\"Label\":\"Raw Data\",\"TextV2\":{\"Text\":\"unsignedpayload\"},\"Type\":\"text_v2\"}],\"PayloadType\":\"fill in parsed signable payload\",\"Title\":\"Unspecified Transaction\",\"Version\":\"0\"}"
        );
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn propagates_grpc_errors() {
    async fn test(test_args: TestArgs) {
        let parse_request = ParseRequest {
            unsigned_payload: "no-no-that-is-not-valid-hex".to_string(),
            chain: Chain::Ethereum as i32,
            chain_metadata: None,
        };

        let parse_error = test_args
            .parser_client
            .unwrap()
            .parse(tonic::Request::new(parse_request))
            .await
            .unwrap_err();

        assert_eq!(parse_error.code(), Code::InvalidArgument);
        assert_eq!(parse_error.message(), "Failed to parse transaction");
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn parser_health_check() {
    async fn test(test_args: TestArgs) {
        let request = tonic::Request::new(AppHealthRequest {});
        let response = test_args
            .health_check_client
            .unwrap()
            .app_health(request)
            .await;
        assert_eq!(
            response.unwrap().into_inner(),
            AppHealthResponse { code: 200 }
        );
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn parser_k8_health() {
    async fn test(test_args: TestArgs) {
        integration::k8_health(test_args).await;
    }

    integration::Builder::new().execute(test).await
}

// This is deliberately using a more "high level test" that only handles the native transfer - any chain specific logic is handled by the tests in chain_parsers
// This allows us to focus on the parser's ability to handle different chain types without getting bogged down in chain-specific libraries
#[tokio::test]
#[tracing_test::traced_test]
async fn parser_solana_native_transfer_e2e() {
    async fn test(test_args: TestArgs) {
        // Base64 encoded Solana transfer transaction
        // This was generated using the Solana CLI using solana transfer --sign-only which only prints message, that needs to be wrapped into a transaction
        let solana_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";

        // If the function is in a different module, update the import path accordingly.
        // For example, if it's in visualsign_solana::utils:
        let solana_tx = visualsign_solana::utils::create_transaction_with_empty_signatures(
            solana_transfer_message,
        );
        tracing::debug!("Solana transaction: {}", solana_tx);
        let parse_request = ParseRequest {
            unsigned_payload: solana_tx,
            chain: Chain::Solana as i32,
            chain_metadata: None,
        };

        let parse_response = test_args
            .parser_client
            .unwrap()
            .parse(tonic::Request::new(parse_request))
            .await
            .unwrap()
            .into_inner();

        let parsed_transaction = parse_response.parsed_transaction.unwrap().payload.unwrap();

        // this is currently optimized around just being able to copy the json output from parser as-is and pass the eye-test
        let expected_sp = serde_json::json!({
            "Fields": [
                {
                    "FallbackText": "Solana",
                    "Label": "Network",
                    "TextV2": {
                        "Text": "Solana"
                    },
                    "Type": "text_v2"
                },
                {
                    "FallbackText": "Transfer 1: From HdD2N8HDzNEM6vwAq5mBLiUbgy1P9wyJfbASt93ndDsD To 8jSCrV9xWkmMRSyf6xH3phL7SretagdqP3LRqkUYUp73 For 1000000000",
                    "Label": "Transfer 1",
                    "TextV2": {
                        "Text": "From: HdD2N8HDzNEM6vwAq5mBLiUbgy1P9wyJfbASt93ndDsD\nTo: 8jSCrV9xWkmMRSyf6xH3phL7SretagdqP3LRqkUYUp73\nAmount: 1000000000"
                    },
                    "Type": "text_v2"
                },
                {
                    "FallbackText": "Program ID: 11111111111111111111111111111111\nData: 0200000000ca9a3b00000000",
                    "Label": "Instruction 1",
                    "PreviewLayout": {
                        "Condensed": {
                            "Fields": [
                                {
                                    "FallbackText": "Transfer: 1000000000 lamports",
                                    "Label": "Instruction",
                                    "TextV2": {
                                        "Text": "Transfer: 1000000000 lamports"
                                    },
                                    "Type": "text_v2"
                                }
                            ]
                        },
                        "Expanded": {
                            "Fields": [
                                {
                                    "FallbackText": "11111111111111111111111111111111",
                                    "Label": "Program ID",
                                    "TextV2": {
                                        "Text": "11111111111111111111111111111111"
                                    },
                                    "Type": "text_v2"
                                },
                                {
                                    "AmountV2": {
                                        "Abbreviation": "lamports",
                                        "Amount": "1000000000"
                                    },
                                    "FallbackText": "1 SOL",
                                    "Label": "Transfer Amount",
                                    "Type": "amount_v2"
                                },
                                {
                                    "FallbackText": "0200000000ca9a3b00000000",
                                    "Label": "Raw Data",
                                    "TextV2": {
                                        "Text": "0200000000ca9a3b00000000"
                                    },
                                    "Type": "text_v2"
                                }
                            ]
                        },
                        "Subtitle": {
                            "Text": ""
                        },
                        "Title": {
                            "Text": "Transfer: 1000000000 lamports"
                        }
                    },
                    "Type": "preview_layout"
                },
                {
                    "FallbackText": "8jSCrV9xWkmMRSyf6xH3phL7SretagdqP3LRqkUYUp73[SW], HdD2N8HDzNEM6vwAq5mBLiUbgy1P9wyJfbASt93ndDsD[SW], 11111111111111111111111111111111[R]",
                    "Label": "Accounts",
                    "PreviewLayout": {
                        "Condensed": {
                            "Fields": [
                                {
                                    "FallbackText": "2 Signers",
                                    "Label": "Signers",
                                    "TextV2": {
                                        "Text": "2 Signers"
                                    },
                                    "Type": "text_v2"
                                },
                                {
                                    "FallbackText": "1 Read Only",
                                    "Label": "Read Only",
                                    "TextV2": {
                                        "Text": "1 Read Only"
                                    },
                                    "Type": "text_v2"
                                }
                            ]
                        },
                        "Expanded": {
                            "Fields": [
                                {
                                    "FallbackText": "8jSCrV9xWkmMRSyf6xH3phL7SretagdqP3LRqkUYUp73, Signer, Writable",
                                    "Label": "Account",
                                    "TextV2": {
                                        "Text": "8jSCrV9xWkmMRSyf6xH3phL7SretagdqP3LRqkUYUp73, Signer, Writable"
                                    },
                                    "Type": "text_v2"
                                },
                                {
                                    "FallbackText": "HdD2N8HDzNEM6vwAq5mBLiUbgy1P9wyJfbASt93ndDsD, Signer, Writable",
                                    "Label": "Account",
                                    "TextV2": {
                                        "Text": "HdD2N8HDzNEM6vwAq5mBLiUbgy1P9wyJfbASt93ndDsD, Signer, Writable"
                                    },
                                    "Type": "text_v2"
                                },
                                {
                                    "FallbackText": "11111111111111111111111111111111",
                                    "Label": "Account",
                                    "TextV2": {
                                        "Text": "11111111111111111111111111111111"
                                    },
                                    "Type": "text_v2"
                                }
                            ]
                        },
                        "Subtitle": {
                            "Text": "3 accounts"
                        },
                        "Title": {
                            "Text": "Accounts"
                        }
                    },
                    "Type": "preview_layout"
                }
            ],
            "PayloadType": "SolanaTx",
            "Title": "Solana Transaction",
            "Version": "0"
        });

        // Verify the transaction contains Solana-specific fields
        let signable_payload: serde_json::Value =
            serde_json::from_str(&parsed_transaction.signable_payload).unwrap();

        // Validate charset safety - no unicode escapes or non-ASCII characters
        let json_str = &parsed_transaction.signable_payload;
        validate_safe_charset(json_str);

        tracing::debug!("📄 Emitted JSON for visual inspection:");
        tracing::debug!("{}", json_str);

        // Validate that the parsed transaction contains all expected fields
        validate_required_fields_present(&signable_payload, &expected_sp);
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn parser_ethereum_native_transfer_e2e() {
    async fn test(test_args: TestArgs) {
        // Base64 encoded Ethereum legacy transaction
        // This is a sample Ethereum transaction that transfers 1 ETH
        let ethereum_tx_hex = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

        let parse_request = ParseRequest {
            unsigned_payload: ethereum_tx_hex.to_string(),
            chain: Chain::Ethereum as i32,
            chain_metadata: None,
        };

        let parse_response = test_args
            .parser_client
            .unwrap()
            .parse(tonic::Request::new(parse_request))
            .await
            .unwrap()
            .into_inner();

        let parsed_transaction = parse_response.parsed_transaction.unwrap().payload.unwrap();

        // Expected structure for Ethereum transaction
        let expected_sp = serde_json::json!({
          "Fields": [
          {
            "FallbackText": "Xpla Mainnet",
            "Label": "Network",
            "TextV2": {
            "Text": "Xpla Mainnet"
            },
            "Type": "text_v2"
          },
          {
            "FallbackText": "0x3535353535353535353535353535353535353535",
            "Label": "To",
            "TextV2": {
            "Text": "0x3535353535353535353535353535353535353535"
            },
            "Type": "text_v2"
          },
          {
            "FallbackText": "1 ETH",
            "Label": "Value",
            "TextV2": {
            "Text": "1 ETH"
            },
            "Type": "text_v2"
          },
          {
            "FallbackText": "21000",
            "Label": "Gas Limit",
            "TextV2": {
            "Text": "21000"
            },
            "Type": "text_v2"
          },
          {
            "FallbackText": "20 gwei",
            "Label": "Gas Price",
            "TextV2": {
            "Text": "20 gwei"
            },
            "Type": "text_v2"
          },
          {
            "FallbackText": "0",
            "Label": "Nonce",
            "TextV2": {
            "Text": "0"
            },
            "Type": "text_v2"
          }
          ],
          "PayloadType": "EthereumTx",
          "Title": "Ethereum Transaction",
          "Version": "0"
        });

        // Verify the transaction contains Ethereum-specific fields
        let signable_payload: serde_json::Value =
            serde_json::from_str(&parsed_transaction.signable_payload).unwrap();

        // Validate that the parsed transaction contains all expected fields
        validate_required_fields_present(&signable_payload, &expected_sp);
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn parser_charset_validation_all_chains() {
    async fn test(test_args: TestArgs) {
        // Test transactions for each supported chain
        // These should all pass charset validation

        // Solana transaction with Jupiter swap (previously had Unicode arrow issue)
        let solana_jupiter_tx = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAkSTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ664CMz4dTWd4gwDq6aKU/sqHgTzleVA7bTCOy59kSOO+0EPkGS7bWuT/2yiCuaADtj/v6d+KwyTj46OQM2MjIq6hTqzVdwLTW8t+UsWMrwHEvc/r814OmVR9yLVQZujbWvpTh0XSNlF7uoIvuHyKD/16mBElrNa/eT8vB1KVUaN8IoaTvZbN4b7iiv8Q8cl5bDecNqCXzTS1Xmsmh5b2UVZniTbtX0AYG5QKiSDC10m0caM6frmEVukpjEWOk7F/0OzFKL0A0HdMWTIMuQj4xBuP3csLyGzVO/MXtPu6woNViO2O9ocxd1YSDcIwhrzHY3a9ewvycRH5q662TcQqdxD6AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKkOA2hfjpCQU+RYEhxm9adq7cdwaqEcgviqlSqPK3h5qVJNNVq4xx0JIWWE9kFLvpQK5lvS5UCde3W3QfWYLIxYjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+Fm0P/on9df2SnTAmx8pWHneSwmrNt/J3VFLMhqns4zl6Mb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hXuFhKBWRymmouYdcNxL6PjM1Bkcio0R+AtqA/P3C3jAFDwYABgALCQwBAQkCAAYMAgAAAEBCDwAAAAAADAEGAREKFQwABgUKEQoQCg0MAAQGAwUHCAECDiTlF8uXeuOtKgEAAAARAWQAAUBCDwAAAAAAtEADAAAAAAAyAAAMAwYAAAEJ";

        // Ethereum transaction
        let ethereum_tx = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

        // Sui transaction
        let sui_tx = "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==";

        // Test each chain
        let test_cases = vec![
            (Chain::Solana, solana_jupiter_tx, "Solana with Jupiter swap"),
            (Chain::Ethereum, ethereum_tx, "Ethereum transfer"),
            (Chain::Sui, sui_tx, "Sui transfer"),
        ];

        for (chain, transaction, description) in test_cases {
            let parse_request = ParseRequest {
                unsigned_payload: transaction.to_string(),
                chain: chain as i32,
                chain_metadata: None,
            };

            let parse_response = test_args
                .parser_client
                .as_ref()
                .unwrap()
                .clone()
                .parse(tonic::Request::new(parse_request))
                .await
                .unwrap_or_else(|e| panic!("Failed to parse {description}: {e:?}"))
                .into_inner();

            let parsed_transaction = parse_response
                .parsed_transaction
                .unwrap_or_else(|| panic!("{description} should have parsed transaction"))
                .payload
                .unwrap_or_else(|| panic!("{description} should have payload"));

            let json_str = &parsed_transaction.signable_payload;

            // Validate charset safety - this will catch ANY non-ASCII characters
            validate_safe_charset(json_str);

            // Verify the JSON can be parsed
            let parsed_json: serde_json::Value = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("{description} should produce valid JSON: {e:?}"));

            // Verify required fields exist
            assert!(
                parsed_json["Fields"].is_array(),
                "{description} should have Fields array",
            );
            assert!(
                parsed_json["Title"].is_string(),
                "{description} should have Title",
            );
            assert!(
                parsed_json["Version"].is_string(),
                "{description} should have Version",
            );

            tracing::debug!("✅ {} passed charset validation", description);
        }
    }

    integration::Builder::new().execute(test).await
}

#[tokio::test]
async fn parser_sui_native_transfer_e2e() {
    async fn test(test_args: TestArgs) {
        let sui_tx_b64 = "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==";

        let parse_request = ParseRequest {
            unsigned_payload: sui_tx_b64.to_string(),
            chain: Chain::Sui as i32,
            chain_metadata: None,
        };

        let parse_response = test_args
            .parser_client
            .unwrap()
            .parse(tonic::Request::new(parse_request))
            .await
            .unwrap()
            .into_inner();

        let parsed_transaction = parse_response.parsed_transaction.unwrap().payload.unwrap();

        let expected_sp = serde_json::json!({
          "Fields": [
            {
              "Type": "text_v2",
              "FallbackText": "Sui Network",
              "Label": "Network",
              "TextV2": {
                "Text": "Sui Network"
              }
            },
            {
              "Type": "preview_layout",
              "FallbackText": "Transfer: 1000000000 MIST (1 SUI)",
              "Label": "Transfer Command",
              "PreviewLayout": {
                "Title": {
                  "Text": "Transfer: 1000000000 MIST (1 SUI)"
                },
                "Subtitle": {
                  "Text": "From 0xd6e9...cac2 to 0xabcd...7890"
                },
                "Condensed": {
                  "Fields": [
                    {
                      "Type": "text_v2",
                      "FallbackText": "Transfer 1000000000 MIST from 0xd6e9...cac2 to 0xabcd...7890",
                      "Label": "Summary",
                      "TextV2": {
                        "Text": "Transfer 1000000000 MIST from 0xd6e9...cac2 to 0xabcd...7890"
                      }
                    }
                  ]
                },
                "Expanded": {
                  "Fields": [
                    {
                      "Type": "text_v2",
                      "FallbackText": "Sui",
                      "Label": "Asset Object ID",
                      "TextV2": {
                        "Text": "Sui"
                      }
                    },
                    {
                      "Type": "address_v2",
                      "FallbackText": "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2",
                      "Label": "From",
                      "AddressV2": {
                        "Address": "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2"
                      }
                    },
                    {
                      "Type": "address_v2",
                      "FallbackText": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                      "Label": "To",
                      "AddressV2": {
                        "Address": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                      }
                    },
                    {
                      "Type": "amount_v2",
                      "FallbackText": "1000000000 MIST",
                      "Label": "Amount",
                      "AmountV2": {
                        "Amount": "1000000000",
                        "Abbreviation": "MIST"
                      }
                    }
                  ]
                }
              }
            },
            {
              "Type": "preview_layout",
              "FallbackText": "Transaction Details",
              "Label": "Transaction Details",
              "PreviewLayout": {
                "Title": {
                  "Text": "Transaction Details"
                },
                "Subtitle": {
                  "Text": "Gas: 5000000 MIST"
                },
                "Condensed": {
                  "Fields": [
                    {
                      "Type": "text_v2",
                      "FallbackText": "Programmable Transaction",
                      "Label": "Transaction Type",
                      "TextV2": {
                        "Text": "Programmable Transaction"
                      }
                    },
                    {
                      "Type": "amount_v2",
                      "FallbackText": "5000000 MIST",
                      "Label": "Gas Budget",
                      "AmountV2": {
                        "Amount": "5000000",
                        "Abbreviation": "MIST"
                      }
                    }
                  ]
                },
                "Expanded": {
                  "Fields": [
                    {
                      "Type": "text_v2",
                      "FallbackText": "Programmable Transaction",
                      "Label": "Transaction Type",
                      "TextV2": {
                        "Text": "Programmable Transaction"
                      }
                    },
                    {
                      "Type": "address_v2",
                      "FallbackText": "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2",
                      "Label": "Gas Owner",
                      "AddressV2": {
                        "Address": "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2"
                      }
                    },
                    {
                      "Type": "amount_v2",
                      "FallbackText": "5000000 MIST",
                      "Label": "Gas Budget",
                      "AmountV2": {
                        "Amount": "5000000",
                        "Abbreviation": "MIST"
                      }
                    },
                    {
                      "Type": "amount_v2",
                      "FallbackText": "1000 MIST",
                      "Label": "Gas Price",
                      "AmountV2": {
                        "Amount": "1000",
                        "Abbreviation": "MIST"
                      }
                    },
                    {
                      "Type": "text_v2",
                      "FallbackText": "0000020020abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890000800ca9a3b00000000020200010101000101020000010000d6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac201cb485ef80a0baf9079a621c42c8b0cb65ce9986e1c81fd16ee88038f004a5aed1b9ad417000000002063805f82e957f1589a7a62346b37cb5aec0f0c1ff4ed74c7949d2bbbd73fef50d6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2e803000000000000404b4c000000000000",
                      "Label": "Raw Data",
                      "TextV2": {
                        "Text": "0000020020abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890000800ca9a3b00000000020200010101000101020000010000d6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac201cb485ef80a0baf9079a621c42c8b0cb65ce9986e1c81fd16ee88038f004a5aed1b9ad417000000002063805f82e957f1589a7a62346b37cb5aec0f0c1ff4ed74c7949d2bbbd73fef50d6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2e803000000000000404b4c000000000000"
                      }
                    }
                  ]
                }
              }
            }
          ],
          "PayloadType": "Sui",
          "Title": "Programmable Transaction",
          "Version": "0"
        });

        let signable_payload: serde_json::Value =
            serde_json::from_str(&parsed_transaction.signable_payload).unwrap();

        validate_required_fields_present(&signable_payload, &expected_sp);
    }

    integration::Builder::new().execute(test).await
}
