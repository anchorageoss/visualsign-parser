// Transaction-level behaviour of the Jupiter Lend Earn preset: when its one
// user action names the whole payload (title, subtitle, hoisted rows), and
// every case in which it must not.

use super::fixture_test::{
    JL_USDC_MINT, USDC_MINT, instruction_from_fixture, load_fixture, rendered_value,
    synthetic_instruction,
};
use super::*;
use crate::core::{SolanaTransactionWrapper, SolanaVisualSignConverter};
use crate::intermediate::{RegisteredSource, SolanaIntermediateOutput};
use crate::test_utils::payload_from_b64;
use base64::Engine;
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    message::{AddressLookupTableAccount, Message, VersionedMessage, v0},
    pubkey::Pubkey,
    signature::Signature,
    transaction::{Transaction, VersionedTransaction},
};
use std::str::FromStr;
use visualsign::vsptrait::{Transaction as _, VisualSignConverter, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

fn legacy_transaction_b64(instructions: &[Instruction], payer: &Pubkey) -> String {
    let message = Message::new(instructions, Some(payer));
    let tx = Transaction::new_unsigned(message);
    let bytes = bincode::serialize(&tx).expect("serialize transaction");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn v0_transaction_b64(
    instructions: &[Instruction],
    payer: &Pubkey,
    lookup_tables: &[AddressLookupTableAccount],
) -> String {
    let message = v0::Message::try_compile(payer, instructions, lookup_tables, Hash::default())
        .expect("compile v0 message");
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(message),
    };
    let bytes = bincode::serialize(&tx).expect("serialize versioned transaction");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn payload_for(instructions: &[Instruction], payer: &Pubkey) -> SignablePayload {
    payload_from_b64(&legacy_transaction_b64(instructions, payer))
}

fn top_level_value(payload: &SignablePayload, label: &str) -> Option<String> {
    payload
        .fields
        .iter()
        .filter_map(rendered_value)
        .find(|(l, _)| l == label)
        .map(|(_, v)| v)
}

fn top_level_labels(payload: &SignablePayload) -> Vec<&str> {
    payload.fields.iter().map(|f| f.label().as_str()).collect()
}

fn preview_titles(payload: &SignablePayload) -> Vec<String> {
    payload
        .fields
        .iter()
        .filter_map(|f| match f {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.title.as_ref().map(|t| t.text.clone())
            }
            _ => None,
        })
        .collect()
}

fn assert_no_summary(payload: &SignablePayload, default_title: &str) {
    assert_eq!(payload.title, default_title);
    assert_eq!(payload.subtitle, None);
    for label in [
        "From",
        "Program",
        "Amount",
        "Instruction",
        "Receive",
        "Burn",
    ] {
        assert!(
            top_level_value(payload, label).is_none(),
            "no hoisted {label} row expected, got layout {:?}",
            top_level_labels(payload)
        );
    }
}

/// A single-instruction deposit: the flat top-level layout, one row per
/// counterpart of a contract-call payload, then the per-instruction detail
/// block and the accounts block.
#[test]
fn test_single_deposit_gets_title_subtitle_and_rows() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;

    let payload = payload_for(&[deposit], &payer);

    assert_eq!(
        payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
    assert_eq!(payload.subtitle.as_deref(), Some(JUPITER_EARN_DISPLAY_NAME));
    // (`starts_with`: the diagnostics feature appends lint rows after these.)
    let labels = top_level_labels(&payload);
    assert!(
        labels.starts_with(&[
            "Network",
            "From",
            "Program",
            "Amount",
            "Recipient",
            "Instruction",
            "Receive",
            "Instruction 1",
            "Accounts",
        ]),
        "unexpected top-level layout: {labels:?}"
    );
    assert_eq!(
        top_level_value(&payload, "From").unwrap(),
        payer.to_string()
    );
    let SignablePayloadField::AddressV2 { address_v2, .. } = &payload.fields[2] else {
        panic!("Program must be an address_v2, got {:?}", payload.fields[2]);
    };
    assert_eq!(address_v2.address, JUPITER_EARN_PROGRAM_ID);
    assert_eq!(address_v2.name, JUPITER_EARN_DISPLAY_NAME);
    let SignablePayloadField::AmountV2 { amount_v2, common } = &payload.fields[3] else {
        panic!("Amount must be an amount_v2, got {:?}", payload.fields[3]);
    };
    assert_eq!(common.fallback_text, "414.122446 USDC");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDC"));
    assert_eq!(top_level_value(&payload, "Instruction").unwrap(), "deposit");
    assert!(
        top_level_value(&payload, "Receive")
            .unwrap()
            .starts_with("jlUSDC")
    );
    let SignablePayloadField::AddressV2 { address_v2, .. } = &payload.fields[4] else {
        panic!(
            "Recipient must be an address_v2, got {:?}",
            payload.fields[4]
        );
    };
    assert_eq!(address_v2.name, "Signer's associated token account");
    payload
        .validate_charset()
        .expect("payload must be ASCII-clean");
}

#[test]
fn test_single_withdraw_gets_title_subtitle_and_rows() {
    let withdraw = instruction_from_fixture(&load_fixture("withdraw_jupusd"));
    let payer = withdraw.accounts[0].pubkey;

    let payload = payload_for(&[withdraw], &payer);

    assert_eq!(
        payload.title,
        "Withdraw 26.177479 JupUSD from Jupiter Lend Earn"
    );
    assert_eq!(payload.subtitle.as_deref(), Some(JUPITER_EARN_DISPLAY_NAME));
    assert_eq!(
        top_level_value(&payload, "From").unwrap(),
        payer.to_string()
    );
    assert_eq!(top_level_value(&payload, "Amount").unwrap(), "26.177479");
    assert_eq!(
        top_level_value(&payload, "Instruction").unwrap(),
        "withdraw"
    );
    assert!(
        top_level_value(&payload, "Burn")
            .unwrap()
            .starts_with("JUICED")
    );
    payload.validate_charset().unwrap();
}

/// Infrastructure legs (ATA creation, compute budget) move nothing to a third
/// party, so the deposit still names it; the priority fee is hoisted in SOL.
#[test]
fn test_infrastructure_legs_keep_the_summary() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let f_token_mint = deposit.accounts[6].pubkey;
    let create_ata =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &payer,
            &payer,
            &f_token_mint,
            &spl_token::id(),
        );
    let compute_limit =
        solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(300_000);
    let compute_price =
        solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(50_000);

    let payload = payload_for(&[compute_limit, compute_price, create_ata, deposit], &payer);

    assert_eq!(
        payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
    assert_eq!(
        top_level_value(&payload, "From").unwrap(),
        payer.to_string()
    );
    let labels = top_level_labels(&payload);
    assert!(
        labels.starts_with(&[
            "Network",
            "From",
            "Program",
            "Amount",
            "Recipient",
            "Instruction",
            "Receive",
            "Priority fee",
            "Instruction 1",
        ]),
        "unexpected top-level layout: {labels:?}"
    );
    let fee = payload
        .fields
        .iter()
        .find(|f| f.label() == "Priority fee")
        .expect("Priority fee row");
    assert_eq!(fee.fallback_text(), "0.000015 SOL");
}

/// No unit limit: the fee row is an upper bound at the runtime maximum and says so.
#[test]
fn test_uncapped_priority_fee_is_hoisted_as_a_maximum() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let compute_price =
        solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(1_000_000_000);

    let payload = payload_for(&[compute_price, deposit], &payer);

    assert_eq!(
        payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
    assert!(top_level_value(&payload, "Priority fee").is_none());
    let fee = payload
        .fields
        .iter()
        .find(|f| f.label() == "Maximum priority fee")
        .expect("Maximum priority fee row");
    assert_eq!(fee.fallback_text(), "1.4 SOL");
}

/// A deposit with no compute-unit price pays only the base fee: no fee row.
#[test]
fn test_summary_without_a_unit_price_has_no_fee_row() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let compute_limit =
        solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(300_000);

    let payload = payload_for(&[compute_limit, deposit], &payer);

    assert_eq!(
        payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
    let labels = top_level_labels(&payload);
    assert!(
        !labels.iter().any(|l| l.contains("riority fee")),
        "no fee row expected, got {labels:?}"
    );
}

/// A forged `token_program`, with the recipient set to the ATA derived under it,
/// must not pass as the signer's account: unverified recipient, no summary.
#[test]
fn test_forged_token_program_keeps_default_title() {
    let mut deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let f_token_mint = deposit.accounts[6].pubkey;
    let bogus_token_program = Pubkey::new_unique();
    deposit.accounts[14].pubkey = bogus_token_program; // `token_program`
    deposit.accounts[2].pubkey =
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &payer,
            &f_token_mint,
            &bogus_token_program,
        ); // `recipient_token_account`

    let payload = payload_for(&[deposit], &payer);

    assert_no_summary(&payload, "Solana Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles
            .iter()
            .any(|t| t == "Deposit 414.122446 USDC to Jupiter Lend Earn"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// Creating an associated token account for another wallet spends the fee
/// payer's rent on a third party. That is not infrastructure, so it blocks
/// the summary, unlike the payer's own account creation above.
#[test]
fn test_ata_creation_for_another_wallet_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let f_token_mint = deposit.accounts[6].pubkey;
    let someone_else = Pubkey::new_unique();
    let create_ata =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &payer,
            &someone_else,
            &f_token_mint,
            &spl_token::id(),
        );

    let payload = payload_for(&[create_ata, deposit], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// A withdraw paying out to an account that is not the signer's associated token
/// account is not summarized: the top-level title is reserved for value staying
/// with the signer. The instruction keeps its semantic view with the badged row.
#[test]
fn test_third_party_recipient_keeps_default_title() {
    let mut withdraw = instruction_from_fixture(&load_fixture("withdraw_jupusd"));
    let payer = withdraw.accounts[0].pubkey;
    withdraw.accounts[2].pubkey = Pubkey::new_unique(); // `recipient_token_account`

    let payload = payload_for(&[withdraw], &payer);

    assert_no_summary(&payload, "Solana Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles
            .iter()
            .any(|t| t == "Withdraw 26.177479 JupUSD from Jupiter Lend Earn"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// An associated-token-account creation paid for by a second signer spends
/// that signer's lamports, not the fee payer's. That is not infrastructure.
#[test]
fn test_ata_creation_funded_by_another_signer_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let f_token_mint = deposit.accounts[6].pubkey;
    let other_funder = Pubkey::new_unique();
    let create_ata =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &other_funder,
            &payer,
            &f_token_mint,
            &spl_token::id(),
        );

    let payload = payload_for(&[create_ata, deposit], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// A deposit bundled with a token transfer to someone else is two actions.
/// Naming it after the deposit alone would hide the transfer from the title,
/// so the default title and layout are kept.
#[test]
fn test_deposit_plus_token_transfer_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let source_ata = deposit.accounts[1].pubkey;
    let transfer = spl_token::instruction::transfer(
        &spl_token::id(),
        &source_ata,
        &Pubkey::new_unique(),
        &payer,
        &[],
        1_000_000_000,
    )
    .unwrap();

    let payload = payload_for(&[deposit, transfer], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// A native transfer is likewise a second action, not infrastructure.
#[test]
fn test_deposit_plus_sol_transfer_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let transfer = solana_sdk::system_instruction::transfer(&payer, &Pubkey::new_unique(), 1_000);

    let payload = payload_for(&[transfer, deposit], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// Two user actions in one transaction: a single action's title would
/// under-describe what is signed.
#[test]
fn test_two_user_actions_keep_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;

    let payload = payload_for(&[deposit.clone(), deposit], &payer);

    assert_no_summary(&payload, "Solana Transaction");
}

/// A relayed transaction: the fee payer is not the depositing signer. The
/// hoisted From row would name the relayer, so no summary is proposed.
#[test]
fn test_relayed_deposit_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let relayer = Pubkey::new_unique();

    let payload = payload_for(&[deposit], &relayer);

    assert_no_summary(&payload, "Solana Transaction");
    // The per-instruction view still renders the action.
    assert_eq!(
        preview_titles(&payload)[0],
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
}

/// A caller-supplied `transaction_name` always wins over the summary.
#[test]
fn test_caller_title_disables_the_summary() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let wrapper =
        SolanaTransactionWrapper::from_string(&legacy_transaction_b64(&[deposit], &payer)).unwrap();

    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(
            wrapper,
            VisualSignOptions {
                transaction_name: Some("Caller Title".to_string()),
                decode_transfers: true,
                ..VisualSignOptions::default()
            },
        )
        .unwrap()
        .payload;

    assert_no_summary(&payload, "Caller Title");
}

/// The v0 path applies the same rule with the same layout.
#[test]
fn test_v0_single_deposit_gets_summary() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;

    let payload = payload_from_b64(&v0_transaction_b64(&[deposit], &payer, &[]));

    assert_eq!(
        payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
    assert_eq!(payload.subtitle.as_deref(), Some(JUPITER_EARN_DISPLAY_NAME));
    let labels = top_level_labels(&payload);
    assert!(
        labels.starts_with(&[
            "Network",
            "From",
            "Program",
            "Amount",
            "Recipient",
            "Instruction",
            "Receive"
        ]),
        "unexpected v0 top-level layout: {labels:?}"
    );
    assert_eq!(
        top_level_value(&payload, "From").unwrap(),
        payer.to_string()
    );
}

/// A v0 deposit whose mints sit in an address lookup table cannot be resolved
/// statically. It must not be summarized from placeholders: default title, no
/// hoisted rows, and the instruction keeps the generic view.
#[test]
fn test_v0_deposit_with_mints_in_lookup_table_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let mint = deposit.accounts[3].pubkey;
    let f_token_mint = deposit.accounts[6].pubkey;
    let table = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: vec![mint, f_token_mint],
    };

    let payload = payload_from_b64(&v0_transaction_b64(&[deposit], &payer, &[table]));

    assert_no_summary(&payload, "Solana V0 Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles.iter().any(|t| t == "Jupiter Lend Earn: deposit"),
        "instruction must fall back to the generic view, got {titles:?}"
    );
}

/// A v0 deposit whose recipient token account sits in a lookup table cannot
/// be shown or classified. Same outcome as an unresolved mint: default title,
/// no hoisted rows, generic instruction view.
#[test]
fn test_v0_deposit_with_recipient_in_lookup_table_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let recipient = deposit.accounts[2].pubkey;
    let table = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: vec![recipient],
    };

    let payload = payload_from_b64(&v0_transaction_b64(&[deposit], &payer, &[table]));

    assert_no_summary(&payload, "Solana V0 Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles.iter().any(|t| t == "Jupiter Lend Earn: deposit"),
        "instruction must fall back to the generic view, got {titles:?}"
    );
}

/// A v0 deposit whose token program sits in a lookup table leaves the
/// recipient unverifiable. The instruction still renders semantically, but
/// it proposes no summary: the unverified recipient is not hoisted.
#[test]
fn test_v0_deposit_with_unverifiable_recipient_keeps_default_title() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let token_program = deposit.accounts[14].pubkey;
    let table = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: vec![token_program],
    };

    let payload = payload_from_b64(&v0_transaction_b64(&[deposit], &payer, &[table]));

    assert_no_summary(&payload, "Solana V0 Transaction");
    let titles = preview_titles(&payload);
    assert!(
        titles
            .iter()
            .any(|t| t == "Deposit 414.122446 USDC to Jupiter Lend Earn"),
        "instruction keeps its semantic view, got {titles:?}"
    );
}

/// Every instruction family that proposes a summary does so with its own
/// wording; a renamed IDL argument would surface here as a default title. The
/// recipient is the signer's associated token account for the received mint,
/// since any other recipient withholds the summary.
#[test]
fn test_each_user_action_family_proposes_a_summary() {
    let payer = Pubkey::new_unique();
    let token_program = spl_token::id();
    let own_ata = |mint: &str| {
        spl_associated_token_account::get_associated_token_address_with_program_id(
            &payer,
            &Pubkey::from_str(mint).unwrap(),
            &token_program,
        )
        .to_string()
    };
    let receipt_recipient = own_ata(JL_USDC_MINT);
    let asset_recipient = own_ata(USDC_MINT);
    let signer = ("signer", payer.to_string());
    let token_program_str = token_program.to_string();
    for (name, args, expected) in [
        (
            "deposit_with_min_amount_out",
            vec![100_000_000u64, 95_000_000],
            "Deposit 100 USDC to Jupiter Lend Earn",
        ),
        (
            "mint",
            vec![50_000_000],
            "Mint 50 jlUSDC on Jupiter Lend Earn",
        ),
        (
            "mint_with_max_assets",
            vec![50_000_000, 52_500_000],
            "Mint 50 jlUSDC on Jupiter Lend Earn",
        ),
        (
            "withdraw_with_max_shares_burn",
            vec![10_000_000, 9_800_000],
            "Withdraw 10 USDC from Jupiter Lend Earn",
        ),
        (
            "redeem",
            vec![1_500_000],
            "Redeem 1.5 jlUSDC from Jupiter Lend Earn",
        ),
        (
            "redeem_with_min_amount_out",
            vec![1_500_000, 1_490_000],
            "Redeem 1.5 jlUSDC from Jupiter Lend Earn",
        ),
    ] {
        let recipient = if name.starts_with("deposit") || name.starts_with("mint") {
            receipt_recipient.as_str()
        } else {
            asset_recipient.as_str()
        };
        let instruction = synthetic_instruction(
            name,
            &args,
            &[
                (signer.0, signer.1.as_str()),
                ("recipient_token_account", recipient),
                ("token_program", token_program_str.as_str()),
            ],
        );
        let payload = payload_for(&[instruction], &payer);
        assert_eq!(payload.title, expected, "{name}");
        assert_eq!(
            top_level_value(&payload, "Instruction").unwrap(),
            name,
            "{name}"
        );
    }
}

/// The machine-readable intermediate output that downstream policy engines
/// consume must agree with the human view: same program, `deposit` from the
/// in-crate preset.
#[test]
fn test_intermediate_output_matches_human_view() {
    let deposit = instruction_from_fixture(&load_fixture("deposit_usdc"));
    let payer = deposit.accounts[0].pubkey;
    let wrapper =
        SolanaTransactionWrapper::from_string(&legacy_transaction_b64(&[deposit], &payer)).unwrap();

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
    assert_eq!(decoded.instructions.len(), 1);
    let ix = &decoded.instructions[0];
    assert_eq!(ix.program_key, JUPITER_EARN_PROGRAM_ID);
    assert_eq!(ix.registered_source, RegisteredSource::Preset);
    let parsed = ix.parsed_instruction_data.as_ref().expect("IDL decode");
    assert_eq!(parsed.instruction_name, "deposit");
    assert_eq!(
        parsed.named_accounts.get("mint").map(String::as_str),
        Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
    );
    assert_eq!(
        result.payload.title,
        "Deposit 414.122446 USDC to Jupiter Lend Earn"
    );
}
