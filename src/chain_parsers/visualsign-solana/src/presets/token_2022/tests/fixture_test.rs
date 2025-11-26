// Fixture-based tests for Token 2022 instruction parsing
// See /src/chain_parsers/visualsign-solana/TESTING.md for documentation
//
// To add these tests to the existing tests module in mod.rs, add this line at the end
// of the existing `mod tests` block (before the closing brace):
//
//     mod fixture_test;
//
// This file will then be compiled as `tests::fixture_test`

use super::*;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr;
use visualsign::SignablePayloadField;
use crate::core::VisualizerContext;
use solana_parser::solana::structs::SolanaAccount;

#[derive(Debug, serde::Deserialize)]
struct TestFixture {
    description: String,
    source: String,
    signature: String,
    cluster: String,
    #[serde(default)]
    full_transaction_note: Option<String>,
    #[allow(dead_code)]
    instruction_index: usize,
    instruction_data: String,
    program_id: String,
    accounts: Vec<TestAccount>,
    #[serde(default)]
    expected_fields: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    expected_error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct TestAccount {
    pubkey: String,
    signer: bool,
    writable: bool,
    #[allow(dead_code)]
    description: String,
}

fn load_fixture(name: &str) -> TestFixture {
    let fixture_path = format!(
        "{}/tests/fixtures/token_2022/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let fixture_content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {fixture_path}: {e}"));
    serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {fixture_path}: {e}"))
}

fn create_instruction_from_fixture(fixture: &TestFixture) -> Instruction {
    let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
    let accounts: Vec<AccountMeta> = fixture
        .accounts
        .iter()
        .map(|acc| {
            let pubkey = Pubkey::from_str(&acc.pubkey).unwrap();
            AccountMeta {
                pubkey,
                is_signer: acc.signer,
                is_writable: acc.writable,
            }
        })
        .collect();

    // Instruction data from JSON RPC responses is base58 encoded
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("Failed to decode base58 instruction data");

    Instruction {
        program_id,
        accounts,
        data,
    }
}

fn test_real_transaction(fixture_name: &str, test_name: &str) {

    let fixture: TestFixture = load_fixture(fixture_name);
    println!("\n=== Testing {test_name} Transaction ===");
    println!("Description: {}", fixture.description);
    println!("Source: {}", fixture.source);
    println!("Signature: {}", fixture.signature);
    println!("Cluster: {}", fixture.cluster);
    if let Some(note) = &fixture.full_transaction_note {
        println!("Transaction Context: {note}");
    }
    println!();

    let instruction = create_instruction_from_fixture(&fixture);
    let instructions = vec![instruction.clone()];

    // Create a context - using index 0 since we only loaded the one relevant instruction
    // In reality, the fixture.instruction_index would be used with all transaction instructions
    let sender = SolanaAccount {
        account_key: fixture.accounts.first().unwrap().pubkey.clone(),
        signer: false,
        writable: false,
    };
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = Token2022Visualizer;
    
    // Check if this is an unhappy path test (expected to fail)
    if let Some(expected_error) = &fixture.expected_error {
        let result = visualizer.visualize_tx_commands(&context);
        assert!(
            result.is_err(),
            "Expected error for unsupported instruction, but parsing succeeded"
        );
        let error_msg = result.unwrap_err().to_string();
        // The error message is wrapped, so check if it contains the expected text
        assert!(
            error_msg.contains(expected_error),
            "Expected error message to contain '{}', but got: {}",
            expected_error,
            error_msg
        );
        println!("✓ Correctly rejected unsupported instruction: {}", error_msg);
        return;
    }
    
    let result = visualizer
        .visualize_tx_commands(&context)
        .expect("Failed to visualize instruction");

    // Extract the preview layout
    if let SignablePayloadField::PreviewLayout {
        common,
        preview_layout,
    } = result.signable_payload_field
    {
        println!("\n=== Extracted Fields ===");
        println!("Label: {}", common.label);
        if let Some(title) = &preview_layout.title {
            println!("Title: {}", title.text);
        }

        if let Some(expanded) = &preview_layout.expanded {
            println!("\nExpanded Fields:");
            for field in &expanded.fields {
                match &field.signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 } => {
                        println!("  {}: {}", common.label, text_v2.text);
                    }
                    SignablePayloadField::Number { common, number } => {
                        println!("  {}: {}", common.label, number.number);
                    }
                    SignablePayloadField::AmountV2 { common, amount_v2 } => {
                        println!("  {}: {}", common.label, amount_v2.amount);
                    }
                    _ => {}
                }
            }
        }

        // Validate against expected fields
        println!("\n=== Validation ===");
        let expected_fields = fixture.expected_fields.as_ref().expect("Expected fields not provided for happy path test");
        for (key, expected_value) in expected_fields {
            let expected_str = expected_value
                .as_str()
                .unwrap_or_else(|| panic!("Expected field '{key}' is not a string"));

            if let Some(expanded) = &preview_layout.expanded {
                let found =
                    expanded
                        .fields
                        .iter()
                        .any(|field| match &field.signable_payload_field {
                            SignablePayloadField::TextV2 { common, text_v2 } => {
                                let label_normalized =
                                    common.label.to_lowercase().replace(" ", "_");
                                let key_normalized = key.to_lowercase();
                                let label_matches = label_normalized == key_normalized;
                                let value_matches = text_v2.text == expected_str;

                                if label_matches {
                                    if value_matches {
                                        println!("✓ {key}: {expected_str} (matches)");
                                    } else {
                                        println!(
                                            "✗ {}: expected '{}', got '{}'",
                                            key, expected_str, text_v2.text
                                        );
                                    }
                                    return value_matches;
                                }
                                false
                            }
                            SignablePayloadField::Number { common, number } => {
                                let label_normalized =
                                    common.label.to_lowercase().replace(" ", "_");
                                let key_normalized = key.to_lowercase();
                                let label_matches = label_normalized == key_normalized;
                                let value_matches = number.number == expected_str;

                                if label_matches {
                                    if value_matches {
                                        println!("✓ {key}: {expected_str} (matches)");
                                    } else {
                                        println!(
                                            "✗ {}: expected '{}', got '{}'",
                                            key, expected_str, number.number
                                        );
                                    }
                                    return value_matches;
                                }
                                false
                            }
                            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                                let label_normalized =
                                    common.label.to_lowercase().replace(" ", "_");
                                let key_normalized = key.to_lowercase();
                                let label_matches = label_normalized == key_normalized;
                                let value_matches = amount_v2.amount == expected_str;

                                if label_matches {
                                    if value_matches {
                                        println!("✓ {key}: {expected_str} (matches)");
                                    } else {
                                        println!(
                                            "✗ {}: expected '{}', got '{}'",
                                            key, expected_str, amount_v2.amount
                                        );
                                    }
                                    return value_matches;
                                }
                                false
                            }
                            _ => false,
                        });

                if !found {
                    println!("✗ {key}: field not found in output");
                }

                assert!(
                    found,
                    "Expected field '{key}' with value '{expected_str}' not found in visualization"
                );
            }
        }
    } else {
        panic!("Expected PreviewLayout field type");
    }
}

#[test]
fn test_mint_to_checked_real_transaction() {
    test_real_transaction("mint_to_checked", "MintToChecked");
}

#[test]
fn test_burn_checked_real_transaction() {
    test_real_transaction("burn_checked", "BurnChecked");
}

#[test]
fn test_transfer_checked_unsupported() {
    test_real_transaction("transfer_checked", "TransferChecked (Unsupported)");
}


