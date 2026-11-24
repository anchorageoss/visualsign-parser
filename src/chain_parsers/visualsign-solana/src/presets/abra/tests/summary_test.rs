// Transaction-level behaviour of the Abra preset: when one operation names the whole
// payload (title, subtitle, hoisted rows), and the cases in which it must not.

use super::fixture_test::{FIXTURE_NAMES, USDAF_MINT, fixture, load_fixture_title};
use super::*;
use crate::core::{SolanaTransactionWrapper, SolanaVisualSignConverter};
use crate::intermediate::{RegisteredSource, SolanaIntermediateOutput};
use crate::presets::associated_token_account::interface::get_associated_token_address_with_program_id;
use crate::test_utils::{
    assert_no_summary, legacy_transaction_b64, payload_for, payload_from_b64, preview_titles,
    top_level_labels, top_level_value, v0_transaction_b64,
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::AddressLookupTableAccount;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use visualsign::vsptrait::{Transaction as _, VisualSignConverter, VisualSignOptions};

/// The shape every sample has: a durable-nonce advance, two compute-budget requests and
/// the operation. The first three are infrastructure, so the mint names the transaction.
#[test]
fn test_mint_with_nonce_and_compute_budget_gets_summary() {
    let mint = fixture("mint_usdc");
    let payer = mint.accounts[1].pubkey;
    let nonce_account = Pubkey::new_unique();
    let advance_nonce =
        solana_system_interface::instruction::advance_nonce_account(&nonce_account, &payer);
    let compute_limit = ComputeBudgetInstruction::set_compute_unit_limit(500_000);
    let compute_price = ComputeBudgetInstruction::set_compute_unit_price(5_000_000);

    let payload = payload_for(&[advance_nonce, compute_limit, compute_price, mint], &payer);

    assert_eq!(payload.title, "Supply USDC to Mint USDaf");
    assert_eq!(
        payload.subtitle.as_deref(),
        Some(ABRA_BACKED_TOKEN_DISPLAY_NAME)
    );
    // (`starts_with`: the diagnostics feature appends lint rows after these.)
    let labels = top_level_labels(&payload);
    assert!(
        labels.starts_with(&[
            "Network",
            "From",
            "Program",
            "Amount",
            "Recipient",
            "Paid to",
            "Instruction",
            "Priority fee",
            "Max Fee (default)",
            "Instruction 1",
        ]),
        "unexpected top-level layout: {labels:?}"
    );
    assert_eq!(
        top_level_value(&payload, "From").unwrap(),
        payer.to_string()
    );
    assert_eq!(top_level_value(&payload, "Amount").unwrap(), "25");
    assert_eq!(
        top_level_value(&payload, "Paid to").unwrap(),
        "dqttTLpHzRj7CiKHPRQ7M43GYXB6iZ2U6vBn6oBZnDB"
    );
    assert_eq!(top_level_value(&payload, "Instruction").unwrap(), "mint");
    let fee = payload
        .fields
        .iter()
        .find(|f| f.label() == "Priority fee")
        .expect("Priority fee row");
    assert_eq!(fee.fallback_text(), "0.0025 SOL");
    payload
        .validate_charset()
        .expect("payload must be ASCII-clean");
}

/// A request hoists the verified escrow instead of a recipient.
#[test]
fn test_request_unstake_hoists_the_escrow() {
    let request = fixture("request_unstake_susdaf");
    let payer = request.accounts[1].pubkey;

    let payload = payload_for(&[request], &payer);

    assert_eq!(payload.title, "Request Unstake sUSDaf");
    assert_eq!(
        payload.subtitle.as_deref(),
        Some(ABRA_LIQUID_STAKING_DISPLAY_NAME)
    );
    let labels = top_level_labels(&payload);
    assert!(
        labels.starts_with(&[
            "Network",
            "From",
            "Program",
            "Amount",
            "Escrow",
            "Instruction",
            "Max Fee (default)",
            "Instruction 1",
            "Accounts",
        ]),
        "unexpected top-level layout: {labels:?}"
    );
    assert_eq!(
        top_level_value(&payload, "Escrow").unwrap(),
        "9pke46FNJFNXYaNNMwyjAci8egcoZovLuQ9jF3TrMo6G"
    );
    assert_eq!(top_level_value(&payload, "Amount").unwrap(), "15");
    payload.validate_charset().unwrap();
}

/// Every operation signed by its fee payer proposes a summary with its own title; a
/// renamed IDL argument or account would surface here as a default title.
#[test]
fn test_each_operation_proposes_a_summary() {
    for name in FIXTURE_NAMES {
        let instruction = fixture(name);
        let payer = instruction.accounts[1].pubkey;
        let payload = payload_for(&[instruction], &payer);
        assert_eq!(payload.title, load_fixture_title(name), "{name}");
        assert!(payload.subtitle.is_some(), "{name}: subtitle");
        assert!(
            top_level_value(&payload, "Instruction").is_some(),
            "{name}: hoisted Instruction row"
        );
        payload.validate_charset().unwrap();
    }
}

/// An escrow of another signer's request must not be hoisted: default title, and the
/// instruction keeps its semantic view with the badged row.
#[test]
fn test_escrow_of_another_request_keeps_default_title() {
    let mut request = fixture("request_unstake_susdaf");
    let payer = request.accounts[1].pubkey;
    request.accounts[7].pubkey = Pubkey::new_unique(); // `user_unstake_request`

    let payload = payload_for(&[request], &payer);

    assert_no_summary(&payload, "Solana Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles.iter().any(|t| t == "Request Unstake sUSDaf"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// A forged `token_program`, with the escrow derived under it, is unverified: no summary.
#[test]
fn test_forged_token_program_keeps_default_title() {
    let mut request = fixture("request_unstake_susdaf");
    let payer = request.accounts[1].pubkey;
    let request_account = request.accounts[7].pubkey; // `user_unstake_request`
    let bogus_token_program = Pubkey::new_unique();
    request.accounts[9].pubkey = bogus_token_program; // `token_program`
    request.accounts[6].pubkey = get_associated_token_address_with_program_id(
        &request_account,
        &Pubkey::from_str(USDAF_MINT).unwrap(),
        &bogus_token_program,
    ); // `escrow_token_account`

    let payload = payload_for(&[request], &payer);

    assert_no_summary(&payload, "Solana Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles.iter().any(|t| t == "Request Unstake sUSDaf"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// A v0 mint whose treasury sits in a lookup table keeps its semantic view with the badged
/// row, but an unresolved destination is never hoisted: default title.
#[test]
fn test_v0_mint_with_treasury_in_lookup_table_keeps_default_title() {
    let mint = fixture("mint_usdc");
    let payer = mint.accounts[1].pubkey;
    let table = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: vec![mint.accounts[6].pubkey], // `treasury_token_account`
    };

    let payload = payload_from_b64(&v0_transaction_b64(&[mint], &payer, &[table]));

    assert_no_summary(&payload, "Solana V0 Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles.iter().any(|t| t == "Supply USDC to Mint USDaf"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// A relayed transaction: the fee payer is not the user. The hoisted From row would name
/// the relayer, so no summary is proposed.
#[test]
fn test_relayed_mint_keeps_default_title() {
    let mint = fixture("mint_usdc");
    let relayer = Pubkey::new_unique();

    let payload = payload_for(&[mint], &relayer);

    assert_no_summary(&payload, "Solana Transaction");
    assert_eq!(preview_titles(&payload)[0], "Supply USDC to Mint USDaf");
}

/// Two operations in one transaction, even across the two programs, are two actions.
#[test]
fn test_mint_and_stake_together_keep_default_title() {
    let mint = fixture("mint_usdc");
    let stake = fixture("stake_usdaf");
    let payer = mint.accounts[1].pubkey;

    let payload = payload_for(&[mint, stake], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// The machine-readable intermediate output decodes both programs through the preset
/// IDLs and agrees with the human view.
#[test]
fn test_intermediate_output_registers_both_programs() {
    for (name, program_id, instruction_name, mint_account) in [
        (
            "mint_usdc",
            ABRA_BACKED_TOKEN_PROGRAM_ID,
            "mint",
            "collateral_token_mint",
        ),
        (
            "stake_usdaf",
            ABRA_LIQUID_STAKING_PROGRAM_ID,
            "stake",
            "underlying_token_mint",
        ),
    ] {
        let instruction = fixture(name);
        let payer = instruction.accounts[1].pubkey;
        let wrapper =
            SolanaTransactionWrapper::from_string(&legacy_transaction_b64(&[instruction], &payer))
                .unwrap();

        let result = SolanaVisualSignConverter
            .to_visual_sign_payload(
                wrapper,
                VisualSignOptions {
                    include_intermediate_output: true,
                    decode_transfers: true,
                    ..VisualSignOptions::default()
                },
            )
            .unwrap();

        let bytes = result.intermediate_output.expect("intermediate output");
        let decoded: SolanaIntermediateOutput = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded.instructions.len(), 1, "{name}");
        let ix = &decoded.instructions[0];
        assert_eq!(ix.program_key, program_id, "{name}");
        assert_eq!(ix.registered_source, RegisteredSource::Preset, "{name}");
        let parsed = ix.parsed_instruction_data.as_ref().expect("IDL decode");
        assert_eq!(parsed.instruction_name, instruction_name, "{name}");
        assert!(
            parsed.named_accounts.contains_key(mint_account),
            "{name}: named accounts {:?}",
            parsed.named_accounts
        );
        assert_eq!(result.payload.title, load_fixture_title(name), "{name}");
    }
}
