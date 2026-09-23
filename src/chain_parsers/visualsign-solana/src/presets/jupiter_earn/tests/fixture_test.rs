// Fixture-based tests for Jupiter Lend Earn instruction parsing.
// See /src/chain_parsers/visualsign-solana/TESTING.md for documentation.
//
// The two fixtures are real mainnet transactions, one `deposit` and one
// `withdraw`. The remaining instruction families have no fixture and are
// exercised with data assembled from the IDL (discriminator plus Borsh args)
// over the fixture's accounts, so a renamed IDL argument fails loudly here
// instead of silently degrading to the generic view.

use super::*;
use crate::test_utils::InstructionTestContext;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::str::FromStr;
use visualsign::SignablePayloadField;

#[derive(Debug, serde::Deserialize)]
pub(super) struct TestFixture {
    description: String,
    signature: String,
    #[allow(dead_code)]
    source: String,
    #[allow(dead_code)]
    cluster: String,
    #[allow(dead_code)]
    full_transaction_note: Option<String>,
    #[allow(dead_code)]
    instruction_index: usize,
    instruction_data: String,
    program_id: String,
    accounts: Vec<TestAccount>,
    expected_title: String,
    expected_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct TestAccount {
    pubkey: String,
    signer: bool,
    writable: bool,
    #[allow(dead_code)]
    description: String,
}

pub(super) fn load_fixture(name: &str) -> TestFixture {
    let fixture_path = format!(
        "{}/tests/fixtures/jupiter_earn/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {fixture_path}: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {fixture_path}: {e}"))
}

pub(super) fn instruction_from_fixture(fixture: &TestFixture) -> Instruction {
    let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
    let accounts = fixture
        .accounts
        .iter()
        .map(|acc| AccountMeta {
            pubkey: Pubkey::from_str(&acc.pubkey).unwrap(),
            is_signer: acc.signer,
            is_writable: acc.writable,
        })
        .collect();
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("fixture instruction_data is base58");
    Instruction {
        program_id,
        accounts,
        data,
    }
}

/// Text rendered for a field, whatever its type: `text_v2` text, `amount_v2`
/// amount, or `address_v2` address.
pub(super) fn rendered_value(field: &SignablePayloadField) -> Option<(String, String)> {
    match field {
        SignablePayloadField::TextV2 { common, text_v2 } => {
            Some((common.label.clone(), text_v2.text.clone()))
        }
        SignablePayloadField::AmountV2 { common, amount_v2 } => {
            Some((common.label.clone(), amount_v2.amount.clone()))
        }
        SignablePayloadField::AddressV2 { common, address_v2 } => {
            Some((common.label.clone(), address_v2.address.clone()))
        }
        _ => None,
    }
}

fn normalized(label: &str) -> String {
    label.to_lowercase().replace(' ', "_")
}

/// Look a label up across the condensed and expanded lists.
fn find_value(layout: &SignablePayloadFieldPreviewLayout, key: &str) -> Option<String> {
    layout
        .condensed
        .iter()
        .chain(layout.expanded.iter())
        .flat_map(|list| list.fields.iter())
        .filter_map(|f| rendered_value(&f.signable_payload_field))
        .find(|(label, _)| normalized(label) == normalized(key))
        .map(|(_, value)| value)
}

fn condensed_value(layout: &SignablePayloadFieldPreviewLayout, key: &str) -> Option<String> {
    layout
        .condensed
        .iter()
        .flat_map(|list| list.fields.iter())
        .filter_map(|f| rendered_value(&f.signable_payload_field))
        .find(|(label, _)| label == key)
        .map(|(_, value)| value)
}

fn visualize(instruction: &Instruction) -> SignablePayloadFieldPreviewLayout {
    let data = InstructionTestContext::from_instruction(instruction);
    let field = JupiterEarnVisualizer
        .visualize_tx_commands(&data.context())
        .expect("visualize failed");
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected a preview_layout");
    };
    preview_layout
}

fn title_of(layout: &SignablePayloadFieldPreviewLayout) -> String {
    layout.title.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

fn assert_fixture(name: &str) {
    let fixture = load_fixture(name);
    let layout = visualize(&instruction_from_fixture(&fixture));
    assert_eq!(
        title_of(&layout),
        fixture.expected_title,
        "{} ({}): title",
        fixture.description,
        fixture.signature
    );
    for (key, expected) in &fixture.expected_fields {
        let expected = expected
            .as_str()
            .unwrap_or_else(|| panic!("expected field {key} must be a string"));
        let actual = find_value(&layout, key)
            .unwrap_or_else(|| panic!("{}: field {key} not rendered", fixture.description));
        assert_eq!(actual, expected, "{}: field {key}", fixture.description);
    }
}

#[test]
fn test_deposit_usdc_fixture() {
    assert_fixture("deposit_usdc");
}

#[test]
fn test_withdraw_jupusd_fixture() {
    assert_fixture("withdraw_jupusd");
}

/// The condensed view carries the program as a named address and the asset
/// amount as a typed `amount_v2` with the registry symbol, not raw base units.
#[test]
fn test_deposit_condensed_view_is_typed() {
    let layout = visualize(&instruction_from_fixture(&load_fixture("deposit_usdc")));
    let condensed = layout.condensed.as_ref().expect("condensed view");
    let fields: Vec<&SignablePayloadField> =
        condensed.fields.iter().map(|f| &f.signable_payload_field).collect();

    let program = fields
        .iter()
        .find(|f| f.label() == "Program")
        .expect("Program row");
    let SignablePayloadField::AddressV2 { address_v2, .. } = program else {
        panic!("Program must be an address_v2, got {program:?}");
    };
    assert_eq!(address_v2.address, JUPITER_EARN_PROGRAM_ID);
    assert_eq!(address_v2.name, JUPITER_EARN_DISPLAY_NAME);

    let amount = fields
        .iter()
        .find(|f| f.label() == "Amount")
        .expect("Amount row");
    let SignablePayloadField::AmountV2 { amount_v2, common } = amount else {
        panic!("Amount must be an amount_v2, got {amount:?}");
    };
    assert_eq!(amount_v2.amount, "414.122446");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDC"));
    assert_eq!(common.fallback_text, "414.122446 USDC");

    let asset = fields
        .iter()
        .find(|f| f.label() == "Asset")
        .expect("Asset row");
    let SignablePayloadField::AddressV2 { address_v2, .. } = asset else {
        panic!("Asset must be an address_v2");
    };
    assert_eq!(address_v2.name, "USD Coin");
    assert_eq!(address_v2.asset_label, "USDC");

    let receipt = fields
        .iter()
        .find(|f| f.label() == "Receipt token")
        .expect("Receipt token row");
    let SignablePayloadField::AddressV2 { address_v2, .. } = receipt else {
        panic!("Receipt token must be an address_v2");
    };
    assert_eq!(address_v2.address, "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D");
    assert_eq!(address_v2.asset_label, "jlUSDC");

    let receive = find_value(&layout, "Receive").expect("Receive row");
    assert!(
        receive.starts_with("jlUSDC"),
        "receive leg names the receipt token: {receive}"
    );
}

/// `withdraw(u64::MAX)` is the full-exit sentinel; it must never render as an
/// 18446744073709551615-unit amount.
#[test]
fn test_withdraw_all_sentinel_renders_full_position() {
    let mut instruction = instruction_from_fixture(&load_fixture("withdraw_jupusd"));
    instruction.data[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    let layout = visualize(&instruction);

    assert_eq!(
        title_of(&layout),
        "Withdraw full JupUSD position from Jupiter Lend Earn"
    );
    assert_eq!(
        condensed_value(&layout, "Amount").unwrap(),
        "Full JupUSD position (all JUICED burned)"
    );
    assert!(condensed_value(&layout, "Burn").is_none());
    // The raw arg stays visible in the expanded view for integrity.
    let expanded = layout.expanded.as_ref().expect("expanded view");
    assert!(
        expanded.fields.iter().any(|f| matches!(
            &f.signable_payload_field,
            SignablePayloadField::TextV2 { common, text_v2 }
                if common.label == "amount" && text_v2.text == u64::MAX.to_string()
        )),
        "expanded view must keep the raw amount arg"
    );
}

/// An unknown asset mint must not be labelled with a guessed symbol or scaled
/// with guessed decimals.
#[test]
fn test_unknown_mint_falls_back_to_raw_units() {
    let mut instruction = instruction_from_fixture(&load_fixture("deposit_usdc"));
    instruction.accounts[3].pubkey = Pubkey::new_unique(); // `mint`
    let layout = visualize(&instruction);

    let title = title_of(&layout);
    assert!(
        title.starts_with("Deposit 414122446 raw units of "),
        "unknown mint must render raw units: {title}"
    );
    assert!(
        condensed_value(&layout, "Amount (raw units)").is_some(),
        "amount row must be labelled as raw units"
    );
}

/// An unresolved lookup-table mint (v0 transaction, ALT contents unavailable)
/// is not an address. The instruction keeps the generic view rather than
/// showing a placeholder as the asset.
#[test]
fn test_unresolved_mint_keeps_generic_view() {
    let instruction = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let mut data = InstructionTestContext::from_instruction(&instruction);
    // Point the `mint` account (position 3) at an index outside account_keys,
    // which is how an ALT-loaded account looks to the static decoder.
    data.compiled_mut().accounts[3] = 200;
    let field = JupiterEarnVisualizer
        .visualize_tx_commands(&data.context())
        .unwrap();
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected preview_layout");
    };

    assert_eq!(title_of(&preview_layout), "Jupiter Lend Earn: deposit");
    assert!(condensed_value(&preview_layout, "Amount").is_none());
    assert!(condensed_value(&preview_layout, "Asset").is_none());
    let mint = find_value(&preview_layout, "mint").unwrap();
    assert!(mint.starts_with("unresolved("), "expanded view keeps the placeholder: {mint}");
}

/// Admin instructions are not user actions: generic condensed view.
#[test]
fn test_admin_instruction_keeps_generic_view() {
    let layout = visualize(&synthetic_instruction("update_rate", &[], &[]));
    assert_eq!(title_of(&layout), "Jupiter Lend Earn: update_rate");
    assert!(condensed_value(&layout, "Amount").is_none());
}

// ---------------------------------------------------------------------------
// Instruction families without a fixture, assembled from the IDL.
// ---------------------------------------------------------------------------

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const JL_USDC_MINT: &str = "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D";

/// Builds `name` from the bundled IDL: its discriminator followed by the `u64`
/// args in order, over one account per IDL account with `mint` and
/// `f_token_mint` set to USDC / jlUSDC and every other account a fresh key.
pub(super) fn synthetic_instruction(name: &str, args: &[u64], overrides: &[(&str, &str)]) -> Instruction {
    let idl = get_jupiter_earn_idl().unwrap();
    let idl_instruction = idl
        .instructions
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("IDL has no instruction {name}"));
    let mut data = idl_instruction.discriminator.clone().unwrap();
    for arg in args {
        data.extend_from_slice(&arg.to_le_bytes());
    }
    let accounts = idl_instruction
        .accounts
        .iter()
        .enumerate()
        .map(|(i, account)| {
            let pubkey = overrides
                .iter()
                .find(|(n, _)| *n == account.name)
                .map(|(_, key)| Pubkey::from_str(key).unwrap())
                .unwrap_or_else(|| match account.name.as_str() {
                    "mint" => Pubkey::from_str(USDC_MINT).unwrap(),
                    "f_token_mint" => Pubkey::from_str(JL_USDC_MINT).unwrap(),
                    _ => Pubkey::new_unique(),
                });
            AccountMeta {
                pubkey,
                is_signer: i == 0,
                is_writable: i == 0,
            }
        })
        .collect();
    Instruction {
        program_id: Pubkey::from_str(JUPITER_EARN_PROGRAM_ID).unwrap(),
        accounts,
        data,
    }
}

#[test]
fn test_deposit_with_min_amount_out() {
    let layout = visualize(&synthetic_instruction(
        "deposit_with_min_amount_out",
        &[100_000_000, 95_000_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Deposit 100 USDC to Jupiter Lend Earn");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "100");
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "95 jlUSDC"
    );
}

#[test]
fn test_mint() {
    let layout = visualize(&synthetic_instruction("mint", &[50_000_000], &[]));
    assert_eq!(title_of(&layout), "Mint 50 jlUSDC on Jupiter Lend Earn");
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Mint");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "50");
    assert!(
        condensed_value(&layout, "Pay").unwrap().starts_with("USDC"),
        "mint pays the asset"
    );
}

#[test]
fn test_mint_with_max_assets() {
    let layout = visualize(&synthetic_instruction(
        "mint_with_max_assets",
        &[50_000_000, 52_500_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Mint 50 jlUSDC on Jupiter Lend Earn");
    assert_eq!(condensed_value(&layout, "Maximum paid").unwrap(), "52.5 USDC");
}

#[test]
fn test_withdraw_with_max_shares_burn() {
    let layout = visualize(&synthetic_instruction(
        "withdraw_with_max_shares_burn",
        &[10_000_000, 9_800_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Withdraw 10 USDC from Jupiter Lend Earn");
    assert!(condensed_value(&layout, "Burn").unwrap().starts_with("jlUSDC"));
    assert_eq!(condensed_value(&layout, "Maximum burned").unwrap(), "9.8 jlUSDC");
}

#[test]
fn test_redeem() {
    let layout = visualize(&synthetic_instruction("redeem", &[1_500_000], &[]));
    assert_eq!(title_of(&layout), "Redeem 1.5 jlUSDC from Jupiter Lend Earn");
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Redeem");
    assert!(condensed_value(&layout, "Receive").unwrap().starts_with("USDC"));
}

#[test]
fn test_redeem_with_min_amount_out() {
    let layout = visualize(&synthetic_instruction(
        "redeem_with_min_amount_out",
        &[1_500_000, 1_490_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Redeem 1.5 jlUSDC from Jupiter Lend Earn");
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "1.49 USDC"
    );
}
