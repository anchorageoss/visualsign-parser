// Fixture-based tests for Jupiter Swap instruction parsing
// See /src/chain_parsers/visualsign-solana/TESTING.md for documentation
//
// To add these tests to the existing tests module in mod.rs, add this line at the end
// of the existing `mod tests` block (before the closing brace):
//
//     mod fixture_tests;
//
// This file will then be compiled as `tests::fixture_tests`

use super::*;
use crate::core::InstructionVisualizationContext;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr;
use visualsign::{AnnotatedPayloadField, SignablePayloadField};

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
    expected_fields: serde_json::Map<String, serde_json::Value>,
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
        "{}/tests/fixtures/jupiter_swap/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let fixture_content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", fixture_path, e));
    serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {}", fixture_path, e))
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

// Example test template - uncomment and fill in when you have a real fixture
/*
#[test]
fn test_route_real_transaction() {
    let fixture = load_fixture("route_example");
    println!("\n=== Testing Real Transaction ===");
    println!("Description: {}", fixture.description);
    println!("Source: {}", fixture.source);
    println!("Signature: {}", fixture.signature);
    println!("Cluster: {}", fixture.cluster);
    if let Some(note) = &fixture.full_transaction_note {
        println!("Transaction Context: {}", note);
    }

    // Create instruction from fixture
    let instruction = create_instruction_from_fixture(&fixture);

    // Create a minimal transaction context for visualization
    let context = InstructionVisualizationContext {
        all_instructions: &[instruction.clone()],
        instruction_index: 0,
        is_inner_instruction: false,
        inner_instruction_index: None,
        transaction_accounts: &[],
    };

    // Visualize the instruction using the Jupiter preset
    let preview_layout = super::visualize_jupiter_swap_instruction(&instruction, &context)
        .expect("Failed to visualize instruction");

    println!("\n=== Extracted Fields ===");
    println!("Label: {}", preview_layout.label);
    println!("Title: {}", preview_layout.title);

    if let Some(expanded) = &preview_layout.expanded {
        println!("\nExpanded Fields:");
        for field in &expanded.fields {
            if let SignablePayloadField::TextV2 { common, text_v2 } =
                &field.signable_payload_field
            {
                println!("  {}: {}", common.label, text_v2.text);
            }
        }
    }

    // Validate against expected fields
    println!("\n=== Validation ===");
    for (key, expected_value) in &fixture.expected_fields {
        let expected_str = expected_value.as_str()
            .unwrap_or_else(|| panic!("Expected field '{}' is not a string", key));

        if let Some(expanded) = &preview_layout.expanded {
            let found = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } =
                    &field.signable_payload_field
                {
                    // Match field label (case-insensitive, ignoring spaces)
                    let label_normalized = common.label.to_lowercase().replace(" ", "_");
                    let key_normalized = key.to_lowercase();
                    let label_matches = label_normalized == key_normalized;
                    let value_matches = text_v2.text == expected_str;

                    if label_matches {
                        if value_matches {
                            println!("✓ {}: {} (matches)", key, expected_str);
                        } else {
                            println!(
                                "✗ {}: expected '{}', got '{}'",
                                key, expected_str, text_v2.text
                            );
                        }
                        return value_matches;
                    }
                    false
                } else {
                    false
                }
            });

            if !found {
                println!("✗ {}: field not found in output", key);
            }

            assert!(
                found,
                "Expected field '{}' with value '{}' not found in visualization",
                key, expected_str
            );
        }
    }
}
*/
