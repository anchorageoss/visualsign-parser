// Fixture-based tests for Orca Whirlpool instruction parsing
// See /src/chain_parsers/visualsign-solana/TESTING.md for documentation
//
// This file is wired into `mod tests` (mod.rs) via `mod fixture_tests;`.

use super::*;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr;
use visualsign::{SignablePayload, SignablePayloadField};

#[derive(Debug, serde::Deserialize)]
struct TestFixture {
    #[allow(dead_code)]
    description: String,
    #[allow(dead_code)]
    source: String,
    #[allow(dead_code)]
    signature: String,
    #[allow(dead_code)]
    cluster: String,
    #[allow(dead_code)]
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
        "{}/tests/fixtures/orca_whirlpool/{}.json",
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

    // Instruction data from JSON RPC responses is base58 encoded.
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("Failed to decode base58 instruction data");

    Instruction {
        program_id,
        accounts,
        data,
    }
}

/// PRS-572 regression: `increase_liquidity_by_token_amounts_v2`'s `method` arg
/// is a struct rendered via `serde_json::Value::to_string()`, which embeds
/// real quotes/braces in the field text. This previously tripped
/// `SignablePayload::validate_charset`'s `\"` ban ("Restricted Characters
/// Detected") for every instruction with a non-scalar arg. Assert both that
/// the field renders the expected embedded-JSON text AND that the resulting
/// payload passes charset validation end to end.
#[test]
fn test_increase_liquidity_by_token_amounts_v2_real_transaction() {
    use crate::core::VisualizerContext;
    use solana_parser::solana::structs::SolanaAccount;

    let fixture: TestFixture = load_fixture("increase_liquidity_by_token_amounts_v2");
    let instruction = create_instruction_from_fixture(&fixture);

    let mut account_keys = vec![instruction.program_id];
    for meta in &instruction.accounts {
        account_keys.push(meta.pubkey);
    }
    let compiled = solana_sdk::instruction::CompiledInstruction {
        program_id_index: 0,
        accounts: (1..=instruction.accounts.len() as u8).collect(),
        data: instruction.data.clone(),
    };

    let sender = SolanaAccount {
        account_key: fixture.accounts.first().unwrap().pubkey.clone(),
        signer: true,
        writable: true,
    };
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

    let visualizer = super::OrcaWhirlpoolVisualizer;
    let result = visualizer
        .visualize_tx_commands(&context)
        .expect("Failed to visualize instruction");

    let SignablePayloadField::PreviewLayout {
        ref preview_layout, ..
    } = result.signable_payload_field
    else {
        panic!("Expected PreviewLayout field type");
    };

    let expanded = preview_layout
        .expanded
        .as_ref()
        .expect("expanded fields must be present");

    for (key, expected_value) in &fixture.expected_fields {
        let expected_str = expected_value
            .as_str()
            .unwrap_or_else(|| panic!("Expected field '{key}' is not a string"));
        // `serde_json::Map`'s key order isn't guaranteed stable across builds
        // (depends on whether the `preserve_order` feature gets pulled in
        // transitively by whatever else is compiled alongside this crate), so
        // an embedded-JSON arg like `method` can render with its object keys
        // in a different order without the underlying value actually
        // changing. Compare structurally (as serde_json::Value, which is
        // order-independent for objects) when both sides parse as JSON;
        // fall back to plain string equality otherwise.
        let expected_json: Option<serde_json::Value> = serde_json::from_str(expected_str).ok();

        let found = expanded.fields.iter().any(|field| {
            let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
            else {
                return false;
            };
            let label_normalized = common.label.to_lowercase().replace(' ', "_");
            if label_normalized != key.to_lowercase() {
                return false;
            }
            match &expected_json {
                Some(expected_value) => serde_json::from_str::<serde_json::Value>(&text_v2.text)
                    .is_ok_and(|actual_value| actual_value == *expected_value),
                None => text_v2.text == expected_str,
            }
        });

        assert!(
            found,
            "Expected field '{key}' with value '{expected_str}' not found in visualization"
        );
    }

    // The whole point of this fixture: the embedded-JSON field must not trip
    // validate_charset.
    let payload = SignablePayload::new(
        1,
        "Orca Whirlpool".to_string(),
        None,
        vec![result.signable_payload_field],
        "SolanaTx".to_string(),
    );
    payload
        .validate_charset()
        .expect("payload with embedded-JSON arg field must pass charset validation");
}
