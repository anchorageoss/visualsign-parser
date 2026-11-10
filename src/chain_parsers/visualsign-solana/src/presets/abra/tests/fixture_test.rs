// Fixture-based tests for the Abra preset, one mainnet fixture per operation (see TESTING.md).

use super::*;
use crate::core::receipt_token::test_support;
use crate::core::receipt_token::token_2022_program_id;
use crate::presets::associated_token_account::interface::get_associated_token_address_with_program_id;
use crate::test_utils::{
    InstructionTestContext, assert_fixture, condensed_field, condensed_value, find_value,
    instruction_from_fixture, load_fixture, title_of, visualize, visualize_context,
};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use visualsign::{SignablePayloadField, SignablePayloadFieldPreviewLayout};

const FIXTURES: &str = "abra";

pub(super) const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub(super) const USDAF_MINT: &str = "usdajSQuzPmUpxHwRRk6ybYTYSgdhqUNhqDLLaaBULP";
pub(super) const SUSDAF_MINT: &str = "susdLJ23peDWNrXk44p5gb8Z8J8FvPMRjYrRHeWnh7b";

pub(super) const FIXTURE_NAMES: [&str; 7] = [
    "mint_usdc",
    "request_unmint_usdaf",
    "claim_unmint_usdc",
    "cancel_unmint_usdaf",
    "stake_usdaf",
    "request_unstake_susdaf",
    "claim_unstake_usdaf",
];

pub(super) fn fixture(name: &str) -> Instruction {
    instruction_from_fixture(&load_fixture(FIXTURES, name))
}

pub(super) fn load_fixture_title(name: &str) -> String {
    load_fixture(FIXTURES, name)
        .expected_title
        .unwrap_or_else(|| panic!("{name}: fixture has no expected_title"))
}

fn render(instruction: &Instruction) -> SignablePayloadFieldPreviewLayout {
    visualize(&AbraVisualizer, instruction)
}

/// Builds `name` from the program's IDL with Abra's mints in place unless overridden;
/// every other account is a fresh key.
pub(super) fn synthetic_instruction(
    program: &ReceiptTokenProgram,
    name: &str,
    args: &[u64],
    overrides: &[(&str, &str)],
) -> Instruction {
    let mut accounts: Vec<(&str, &str)> = overrides.to_vec();
    accounts.extend([
        ("abrafi_backed_token_mint", USDAF_MINT),
        ("collateral_token_mint", USDC_MINT),
        ("claim_token_mint", USDC_MINT),
        ("liquid_staking_token_mint", SUSDAF_MINT),
        ("underlying_token_mint", USDAF_MINT),
    ]);
    test_support::synthetic_instruction(program, name, args, &accounts)
}

/// (address, name, badge) of a condensed `address_v2` row.
fn address_row(
    layout: &SignablePayloadFieldPreviewLayout,
    label: &str,
) -> (String, String, Option<String>) {
    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(layout, label) else {
        panic!("{label} must be an address_v2");
    };
    (
        address_v2.address.clone(),
        address_v2.name.clone(),
        address_v2.badge_text.clone(),
    )
}

#[test]
fn test_every_operation_fixture() {
    for name in FIXTURE_NAMES {
        assert_fixture(&AbraVisualizer, FIXTURES, name);
    }
}

/// The condensed view carries the program as a named address, the typed amount with
/// the registry symbol, the signer's own receipt account and the treasury the USDC goes to.
#[test]
fn test_mint_condensed_view_is_typed() {
    let layout = render(&fixture("mint_usdc"));

    let (address, name, _) = address_row(&layout, "Program");
    assert_eq!(address, ABRA_BACKED_TOKEN_PROGRAM_ID);
    assert_eq!(name, ABRA_BACKED_TOKEN_DISPLAY_NAME);
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Mint");

    let amount = condensed_field(&layout, "Amount");
    let SignablePayloadField::AmountV2 { amount_v2, common } = amount else {
        panic!("Amount must be an amount_v2, got {amount:?}");
    };
    assert_eq!(amount_v2.amount, "25");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDC"));
    assert_eq!(common.fallback_text, "25 USDC");

    let (address, name, _) = address_row(&layout, "Asset");
    assert_eq!(address, USDC_MINT);
    assert_eq!(name, "USD Coin");
    let SignablePayloadField::AddressV2 { address_v2, .. } =
        condensed_field(&layout, "Receipt token")
    else {
        panic!("Receipt token must be an address_v2");
    };
    assert_eq!(address_v2.address, USDAF_MINT);
    assert_eq!(address_v2.asset_label, "USDaf");

    let (address, name, badge) = address_row(&layout, "Recipient");
    assert_eq!(address, "74sSAVU4ELgAevgPKDzBi6qv2TTYQF7NqGvS6ZVaYiV4");
    assert_eq!(name, "Signer's associated token account");
    assert_eq!(badge, None);
    let (address, _, _) = address_row(&layout, "Paid to");
    assert_eq!(address, "dqttTLpHzRj7CiKHPRQ7M43GYXB6iZ2U6vBn6oBZnDB");
}

/// What the signer receives depends on the rate at execution and is not an instruction
/// argument, so no row estimates it on either program.
#[test]
fn test_no_counter_leg_is_estimated() {
    for name in FIXTURE_NAMES {
        let layout = render(&fixture(name));
        for label in ["Receive", "Receive at claim", "Pay", "Burn"] {
            assert!(
                condensed_value(&layout, label).is_none(),
                "{name}: unexpected {label} row"
            );
        }
    }
}

#[test]
fn test_stake_names_the_vault_and_the_receipt() {
    let layout = render(&fixture("stake_usdaf"));
    assert_eq!(title_of(&layout), "Stake USDaf to Mint sUSDaf");
    let SignablePayloadField::AmountV2 { amount_v2, .. } = condensed_field(&layout, "Amount")
    else {
        panic!("Amount must be an amount_v2");
    };
    assert_eq!(amount_v2.amount, "25");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDaf"));
    let (address, _, _) = address_row(&layout, "Staked into");
    assert_eq!(address, "FJJPuLnJZRi7kBXzX6PttpFm9z7CSU2zQZWt9esbmvFe");
    let (address, name, badge) = address_row(&layout, "Recipient");
    assert_eq!(address, "E9T8ta4xJgCSpKovaTuSpLuLTQGfoq5QVpmuxBgdgsGU");
    assert_eq!(name, "Signer's associated token account");
    assert_eq!(badge, None);
}

/// A request escrows the funds in the token account of the signer's request account; the
/// row re-derives the request from the IDL seeds and the escrow as its token account.
#[test]
fn test_requests_verify_the_signers_escrow() {
    let layout = render(&fixture("request_unstake_susdaf"));
    assert_eq!(title_of(&layout), "Request Unstake sUSDaf");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "15");
    let (address, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(address, "9pke46FNJFNXYaNNMwyjAci8egcoZovLuQ9jF3TrMo6G");
    assert_eq!(name, "Signer's escrow account");
    assert_eq!(badge, None);

    let layout = render(&fixture("request_unmint_usdaf"));
    assert_eq!(title_of(&layout), "Request Unmint USDaf");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "2.000638");
    let (address, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(address, "FNASyWsW4TQrVyDeg263Qrny4aJWRjmDAXYecmqo4sLk");
    assert_eq!(name, "Signer's escrow account");
    assert_eq!(badge, None);
}

/// An escrow that is not derived from the signer is flagged: whether the request account
/// or the escrow itself is swapped, the row says so and the title stays semantic.
#[test]
fn test_escrow_of_another_request_is_third_party() {
    let mut instruction = fixture("request_unstake_susdaf");
    instruction.accounts[7].pubkey = Pubkey::new_unique(); // `user_unstake_request`
    let layout = render(&instruction);
    assert_eq!(title_of(&layout), "Request Unstake sUSDaf");
    let (_, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(name, "Not the signer's escrow account");
    assert_eq!(badge.as_deref(), Some("THIRD PARTY"));

    let mut instruction = fixture("request_unmint_usdaf");
    instruction.accounts[5].pubkey = Pubkey::new_unique(); // `escrow_token_account`
    let layout = render(&instruction);
    let (_, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(name, "Not the signer's escrow account");
    assert_eq!(badge.as_deref(), Some("THIRD PARTY"));
}

/// A forged `token_program`, with the escrow set to the ATA derived under it, must stay
/// unverified: ATA derivation means nothing outside the two real token programs.
#[test]
fn test_escrow_under_forged_token_program_is_unverified() {
    let mut instruction = fixture("request_unstake_susdaf");
    let request = instruction.accounts[7].pubkey; // `user_unstake_request`
    let bogus_token_program = Pubkey::new_unique();
    instruction.accounts[9].pubkey = bogus_token_program; // `token_program`
    instruction.accounts[6].pubkey = get_associated_token_address_with_program_id(
        &request,
        &Pubkey::from_str(USDAF_MINT).unwrap(),
        &bogus_token_program,
    ); // `escrow_token_account`
    let layout = render(&instruction);
    assert_eq!(title_of(&layout), "Request Unstake sUSDaf");
    let (_, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(name, "Ownership not verified");
    assert_eq!(badge.as_deref(), Some("UNVERIFIED"));
}

/// Token-2022 is the other real token program: the escrow derived under it is recognized.
#[test]
fn test_escrow_under_token_2022_is_the_signers_escrow() {
    let mut instruction = fixture("request_unstake_susdaf");
    let request = instruction.accounts[7].pubkey; // `user_unstake_request`
    let token_program = token_2022_program_id();
    let escrow = get_associated_token_address_with_program_id(
        &request,
        &Pubkey::from_str(USDAF_MINT).unwrap(),
        &token_program,
    );
    instruction.accounts[9].pubkey = token_program; // `token_program`
    instruction.accounts[6].pubkey = escrow; // `escrow_token_account`
    let layout = render(&instruction);
    let (address, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(address, escrow.to_string());
    assert_eq!(name, "Signer's escrow account");
    assert_eq!(badge, None);
}

/// A destination in a lookup table cannot be shown: the row says so instead of vanishing.
#[test]
fn test_unresolved_account_row_is_badged() {
    let mut data = InstructionTestContext::from_instruction(&fixture("mint_usdc"));
    data.compiled_mut().accounts[6] = 200; // `treasury_token_account`
    let layout = visualize_context(&AbraVisualizer, &data);
    assert_eq!(title_of(&layout), "Supply USDC to Mint USDaf");
    let (address, name, badge) = address_row(&layout, "Paid to");
    assert_eq!(address, "unresolved(200)");
    assert_eq!(name, "Not resolved (address lookup table)");
    assert_eq!(badge.as_deref(), Some("UNVERIFIED"));
}

/// An escrow whose request account sits in a lookup table cannot be re-derived: the row
/// must not look safer than a checked third party, so it is badged.
#[test]
fn test_unresolved_request_account_is_unverified() {
    let mut data = InstructionTestContext::from_instruction(&fixture("request_unstake_susdaf"));
    data.compiled_mut().accounts[7] = 200; // `user_unstake_request`
    let layout = visualize_context(&AbraVisualizer, &data);
    assert_eq!(title_of(&layout), "Request Unstake sUSDaf");
    let (_, name, badge) = address_row(&layout, "Escrow");
    assert_eq!(name, "Ownership not verified");
    assert_eq!(badge.as_deref(), Some("UNVERIFIED"));
}

/// An unresolved escrow is not an address: the instruction keeps the generic view.
#[test]
fn test_unresolved_escrow_keeps_generic_view() {
    let mut data = InstructionTestContext::from_instruction(&fixture("request_unstake_susdaf"));
    data.compiled_mut().accounts[6] = 200; // `escrow_token_account`
    let layout = visualize_context(&AbraVisualizer, &data);
    assert_eq!(title_of(&layout), "Abra Liquid Staking: request_unstake");
    assert!(condensed_value(&layout, "Escrow").is_none());
    assert!(condensed_value(&layout, "Amount").is_none());
}

/// A claim burns the escrowed USDaf and pays USDC from Abra's withdrawal vault into the
/// signer's own USDC account; the amount is the USDaf burned.
#[test]
fn test_claim_unmint_burns_the_escrow_and_pays_from_the_vault() {
    let layout = render(&fixture("claim_unmint_usdc"));
    assert_eq!(title_of(&layout), "Collect USDC from Abra");
    let SignablePayloadField::AmountV2 { amount_v2, .. } = condensed_field(&layout, "Amount")
    else {
        panic!("Amount must be an amount_v2");
    };
    assert_eq!(amount_v2.amount, "24999.999999");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDaf"));
    let (address, name, badge) = address_row(&layout, "Recipient");
    assert_eq!(address, "C9MPUHh4KdzirmLcwZSL63WNNmMsBd7gz2gnenVHuacR");
    assert_eq!(name, "Signer's associated token account");
    assert_eq!(badge, None);
    let (address, _, _) = address_row(&layout, "Paid from");
    assert_eq!(address, "Fyvh6Mb9XdVMkhUdLQRWiQDTdWSWM3K4k4YrKisYm3iX");
    let (address, _, _) = address_row(&layout, "Burned from");
    assert_eq!(address, "C2JgWMy5gW3qAKMkAqepYcXLC8KsoLN93GSF95TEfuw1");
}

#[test]
fn test_claim_unstake_collects_from_the_escrow() {
    let layout = render(&fixture("claim_unstake_usdaf"));
    assert_eq!(title_of(&layout), "Collect USDaf from Abra");
    let SignablePayloadField::AmountV2 { amount_v2, .. } = condensed_field(&layout, "Amount")
    else {
        panic!("Amount must be an amount_v2");
    };
    assert_eq!(amount_v2.amount, "2.000638");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDaf"));
    let (address, _, _) = address_row(&layout, "Collected from");
    assert_eq!(address, "9pke46FNJFNXYaNNMwyjAci8egcoZovLuQ9jF3TrMo6G");
}

#[test]
fn test_cancel_unmint_returns_the_escrow_to_the_signer() {
    let layout = render(&fixture("cancel_unmint_usdaf"));
    assert_eq!(title_of(&layout), "Cancel Unmint USDaf");
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Cancel unmint");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "2.000638");
    let (address, name, _) = address_row(&layout, "Recipient");
    assert_eq!(address, "74sSAVU4ELgAevgPKDzBi6qv2TTYQF7NqGvS6ZVaYiV4");
    assert_eq!(name, "Signer's associated token account");
    let (address, _, _) = address_row(&layout, "Returned from");
    assert_eq!(address, "FNASyWsW4TQrVyDeg263Qrny4aJWRjmDAXYecmqo4sLk");
}

/// A receipt paid to someone else's token account is flagged, with the same title.
#[test]
fn test_third_party_recipient_is_flagged() {
    let mut instruction = fixture("mint_usdc");
    instruction.accounts[4].pubkey = Pubkey::new_unique(); // `user_abrafi_backed_token_account`
    let layout = render(&instruction);
    assert_eq!(title_of(&layout), "Supply USDC to Mint USDaf");
    let (_, name, badge) = address_row(&layout, "Recipient");
    assert_eq!(name, "Not the signer's associated token account");
    assert_eq!(badge.as_deref(), Some("THIRD PARTY"));
}

/// No Abra instruction has a verified `u64::MAX` sentinel: the literal is shown and flagged.
#[test]
fn test_u64_max_has_no_sentinel_meaning() {
    let mut instruction = fixture("mint_usdc");
    instruction.data[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    let layout = render(&instruction);
    assert_eq!(title_of(&layout), "Supply USDC to Mint USDaf");
    assert_eq!(
        condensed_value(&layout, "Amount").unwrap(),
        "18446744073709551615 raw units of USDC (u64::MAX)"
    );
}

/// An unknown collateral mint must not be labelled with a guessed symbol or scaled with
/// guessed decimals. The programs are generic builds, so other assets may share them.
#[test]
fn test_unknown_collateral_mint_renders_raw_units() {
    let mut instruction = fixture("mint_usdc");
    instruction.accounts[3].pubkey = Pubkey::new_unique(); // `collateral_token_mint`
    let layout = render(&instruction);
    let title = title_of(&layout);
    assert!(
        !title.contains("USDC"),
        "unknown mint must not be labelled with a guessed symbol: {title}"
    );
    assert!(condensed_value(&layout, "Amount (raw units)").is_some());
}

/// With fewer accounts than the IDL lists, positional names may be misaligned, so the
/// instruction keeps the generic view instead of classifying the wrong account.
#[test]
fn test_fewer_accounts_than_idl_keeps_generic_view() {
    let mut instruction = fixture("stake_usdaf");
    instruction.accounts.pop(); // drop the trailing `system_program`
    let layout = render(&instruction);
    assert_eq!(title_of(&layout), "Abra Liquid Staking: stake");
    assert!(condensed_value(&layout, "Recipient").is_none());
    assert!(condensed_value(&layout, "Amount").is_none());
}

/// `restake` and the read-only calculators are not allowlisted operations: generic view.
#[test]
fn test_other_instructions_keep_generic_view() {
    let staking = &ABRA_PROGRAMS[1];
    let layout = render(&synthetic_instruction(
        staking,
        "restake",
        &[1_000_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Abra Liquid Staking: restake");
    assert!(condensed_value(&layout, "Amount").is_none());
    assert_eq!(
        find_value(&layout, "restake_underlying_amount").unwrap(),
        "1000000"
    );

    let layout = render(&synthetic_instruction(
        staking,
        "calculate_stake_amount",
        &[1_000_000],
        &[],
    ));
    assert_eq!(
        title_of(&layout),
        "Abra Liquid Staking: calculate_stake_amount"
    );
}
