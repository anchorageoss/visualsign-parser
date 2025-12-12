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
use crate::core::VisualizerContext;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr;
use visualsign::SignablePayloadField;

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

// Helper to encode instruction bytes to base58 for fixtures
#[allow(dead_code)]
fn encode_instruction_data(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
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
            "Expected error message to contain '{expected_error}', but got: {error_msg}"
        );
        println!("✓ Correctly rejected unsupported instruction: {error_msg}");
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
        let expected_fields = fixture
            .expected_fields
            .as_ref()
            .expect("Expected fields not provided for happy path test");
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

#[test]
fn test_pause_real_transaction() {
    test_real_transaction("pause", "Pause");
}

#[test]
fn test_resume_real_transaction() {
    test_real_transaction("resume", "Resume");
}

#[test]
fn test_set_authority_real_transaction() {
    test_real_transaction("set_authority", "SetAuthority");
}

#[test]
fn test_freeze_real_transaction() {
    test_real_transaction("freeze", "Freeze");
}

#[test]
fn test_thaw_real_transaction() {
    test_real_transaction("thaw", "Thaw");
}

#[test]
fn test_close_account_real_transaction() {
    test_real_transaction("close_account", "CloseAccount");
}

#[test]
fn test_encode_pause_resume_instructions() {
    // Helper test to generate correct base58 encodings for Pause and Resume
    let pause_bytes = [44u8, 1u8];
    let resume_bytes = [44u8, 2u8];

    let pause_b58 = bs58::encode(&pause_bytes).into_string();
    let resume_b58 = bs58::encode(&resume_bytes).into_string();

    println!("Pause [44,1] base58: {pause_b58}");
    println!("Resume [44,2] base58: {resume_b58}");

    // Verify they decode correctly
    let pause_decoded = bs58::decode(&pause_b58).into_vec().unwrap();
    let resume_decoded = bs58::decode(&resume_b58).into_vec().unwrap();

    assert_eq!(pause_decoded, pause_bytes);
    assert_eq!(resume_decoded, resume_bytes);
}

#[test]
fn test_encode_set_authority_instruction() {
    // Helper test to generate correct base58 encoding for SetAuthority
    // Structure: [discriminator (6), authority_type (0 = MintTokens), option_flag (1 = Some), pubkey (32 bytes)]
    use solana_sdk::pubkey::Pubkey;

    let new_authority = Pubkey::from_str("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM").unwrap();
    let mut set_authority_bytes = vec![6u8, 0u8, 1u8]; // discriminator, authority_type (MintTokens), option_flag (Some)
    set_authority_bytes.extend_from_slice(new_authority.as_ref());

    let set_authority_b58 = bs58::encode(&set_authority_bytes).into_string();

    println!("SetAuthority [6, 0, 1, <pubkey>] base58: {set_authority_b58}");
    println!("SetAuthority bytes length: {}", set_authority_bytes.len());

    // Verify it decodes correctly
    let decoded = bs58::decode(&set_authority_b58).into_vec().unwrap();
    assert_eq!(decoded, set_authority_bytes);
    assert_eq!(decoded[0], 6);
    assert_eq!(decoded[1], 0);
    assert_eq!(decoded[2], 1);
}

#[test]
fn test_encode_close_account_instruction() {
    // Helper test to generate correct base58 encoding for CloseAccount
    // CloseAccount is instruction variant 9 (0x09)
    let close_account_bytes = [9u8];

    let close_account_b58 = bs58::encode(&close_account_bytes).into_string();

    println!("CloseAccount [9] base58: {close_account_b58}");

    // Verify it decodes correctly
    let decoded = bs58::decode(&close_account_b58).into_vec().unwrap();
    assert_eq!(decoded, close_account_bytes);
    assert_eq!(decoded[0], 9);
}

#[test]
fn test_encode_freeze_thaw_instructions() {
    // Helper test to generate correct base58 encodings for Freeze and Thaw
    // FreezeAccount is instruction variant 10 (0x0A)
    // ThawAccount is instruction variant 11 (0x0B)
    let freeze_bytes = [10u8];
    let thaw_bytes = [11u8];

    let freeze_b58 = bs58::encode(&freeze_bytes).into_string();
    let thaw_b58 = bs58::encode(&thaw_bytes).into_string();

    println!("Freeze [10] base58: {freeze_b58}");
    println!("Thaw [11] base58: {thaw_b58}");

    // Verify they decode correctly
    let freeze_decoded = bs58::decode(&freeze_b58).into_vec().unwrap();
    let thaw_decoded = bs58::decode(&thaw_b58).into_vec().unwrap();

    assert_eq!(freeze_decoded, freeze_bytes);
    assert_eq!(thaw_decoded, thaw_bytes);
}
