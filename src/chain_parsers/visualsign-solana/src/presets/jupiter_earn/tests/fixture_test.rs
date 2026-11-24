// Fixture-based tests for Jupiter Lend Earn instruction parsing.
// See /src/chain_parsers/visualsign-solana/TESTING.md for documentation.
//
// The two fixtures are real mainnet transactions, one `deposit` and one
// `withdraw`. The remaining instruction families have no fixture and are
// exercised with data assembled from the IDL (discriminator plus Borsh args)
// over the fixture's accounts, so a renamed IDL argument fails loudly here
// instead of silently degrading to the generic view.

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

const FIXTURES: &str = "jupiter_earn";

pub(super) const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub(super) const JL_USDC_MINT: &str = "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D";

pub(super) fn fixture(name: &str) -> Instruction {
    instruction_from_fixture(&load_fixture(FIXTURES, name))
}

fn render(instruction: &Instruction) -> SignablePayloadFieldPreviewLayout {
    visualize(&JupiterEarnVisualizer, instruction)
}

/// Builds `name` from the bundled IDL with `mint` and `f_token_mint` set to USDC / jlUSDC
/// unless overridden; every other account is a fresh key.
pub(super) fn synthetic_instruction(
    name: &str,
    args: &[u64],
    overrides: &[(&str, &str)],
) -> Instruction {
    let mut accounts: Vec<(&str, &str)> = overrides.to_vec();
    accounts.extend([("mint", USDC_MINT), ("f_token_mint", JL_USDC_MINT)]);
    test_support::synthetic_instruction(&JUPITER_EARN, name, args, &accounts)
}

#[test]
fn test_deposit_usdc_fixture() {
    assert_fixture(&JupiterEarnVisualizer, FIXTURES, "deposit_usdc");
}

#[test]
fn test_withdraw_jupusd_fixture() {
    assert_fixture(&JupiterEarnVisualizer, FIXTURES, "withdraw_jupusd");
}

/// The condensed view carries the program as a named address and the asset
/// amount as a typed `amount_v2` with the registry symbol, not raw base units.
#[test]
fn test_deposit_condensed_view_is_typed() {
    let layout = render(&fixture("deposit_usdc"));

    let program = condensed_field(&layout, "Program");
    let SignablePayloadField::AddressV2 { address_v2, .. } = program else {
        panic!("Program must be an address_v2, got {program:?}");
    };
    assert_eq!(address_v2.address, JUPITER_EARN_PROGRAM_ID);
    assert_eq!(address_v2.name, JUPITER_EARN_DISPLAY_NAME);

    let amount = condensed_field(&layout, "Amount");
    let SignablePayloadField::AmountV2 { amount_v2, common } = amount else {
        panic!("Amount must be an amount_v2, got {amount:?}");
    };
    assert_eq!(amount_v2.amount, "414.122446");
    assert_eq!(amount_v2.abbreviation.as_deref(), Some("USDC"));
    assert_eq!(common.fallback_text, "414.122446 USDC");

    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(&layout, "Asset")
    else {
        panic!("Asset must be an address_v2");
    };
    assert_eq!(address_v2.name, "USD Coin");
    assert_eq!(address_v2.asset_label, "USDC");

    let SignablePayloadField::AddressV2 { address_v2, .. } =
        condensed_field(&layout, "Receipt token")
    else {
        panic!("Receipt token must be an address_v2");
    };
    assert_eq!(address_v2.address, JL_USDC_MINT);
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
    let mut instruction = fixture("withdraw_jupusd");
    instruction.data[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    let layout = render(&instruction);

    assert_eq!(
        title_of(&layout),
        "Redeem full JUICED position via Jupiter Lend"
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
    let mut instruction = fixture("deposit_usdc");
    instruction.accounts[3].pubkey = Pubkey::new_unique(); // `mint`
    let layout = render(&instruction);

    let title = title_of(&layout);
    assert!(
        !title.contains("USDC"),
        "unknown mint must not be labelled with a guessed symbol: {title}"
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
    let mut data = InstructionTestContext::from_instruction(&fixture("deposit_usdc"));
    // Point the `mint` account (position 3) at an index outside account_keys,
    // which is how an ALT-loaded account looks to the static decoder.
    data.compiled_mut().accounts[3] = 200;
    let preview_layout = visualize_context(&JupiterEarnVisualizer, &data);

    assert_eq!(title_of(&preview_layout), "Jupiter Lend Earn: deposit");
    assert!(condensed_value(&preview_layout, "Amount").is_none());
    assert!(condensed_value(&preview_layout, "Asset").is_none());
    let mint = find_value(&preview_layout, "mint").unwrap();
    assert!(
        mint.starts_with("unresolved("),
        "expanded view keeps the placeholder: {mint}"
    );
}

/// An unresolved `recipient_token_account` is treated like an unresolved
/// mint: no address can be shown or classified, so the instruction keeps the
/// generic view.
#[test]
fn test_unresolved_recipient_keeps_generic_view() {
    let mut data = InstructionTestContext::from_instruction(&fixture("deposit_usdc"));
    data.compiled_mut().accounts[2] = 200; // `recipient_token_account`
    let preview_layout = visualize_context(&JupiterEarnVisualizer, &data);

    assert_eq!(title_of(&preview_layout), "Jupiter Lend Earn: deposit");
    assert!(condensed_value(&preview_layout, "Recipient").is_none());
    assert!(condensed_value(&preview_layout, "Amount").is_none());
    let recipient = find_value(&preview_layout, "recipient_token_account").unwrap();
    assert!(
        recipient.starts_with("unresolved("),
        "expanded view keeps the placeholder: {recipient}"
    );
}

/// When the token program is unresolved the recipient cannot be classified.
/// The row must not look safer than a checked third party: it is badged.
#[test]
fn test_unverifiable_recipient_is_badged() {
    let mut data = InstructionTestContext::from_instruction(&fixture("deposit_usdc"));
    data.compiled_mut().accounts[14] = 200; // `token_program`
    let preview_layout = visualize_context(&JupiterEarnVisualizer, &data);

    assert_eq!(title_of(&preview_layout), "Supply USDC to Jupiter Lend");
    let SignablePayloadField::AddressV2 { address_v2, .. } =
        condensed_field(&preview_layout, "Recipient")
    else {
        panic!("Recipient must be an address_v2");
    };
    assert_eq!(address_v2.name, "Ownership not verified");
    assert_eq!(address_v2.badge_text.as_deref(), Some("UNVERIFIED"));
}

/// A bogus `token_program`, with the recipient set to the ATA derived under it, must stay
/// unverified.
#[test]
fn test_recipient_with_unknown_token_program_is_unverified() {
    let mut instruction = fixture("deposit_usdc");
    let signer = instruction.accounts[0].pubkey;
    let f_token_mint = instruction.accounts[6].pubkey;
    let bogus_token_program = Pubkey::new_unique();
    instruction.accounts[14].pubkey = bogus_token_program; // `token_program`
    instruction.accounts[2].pubkey =
        get_associated_token_address_with_program_id(&signer, &f_token_mint, &bogus_token_program); // `recipient_token_account`
    let layout = render(&instruction);

    assert_eq!(title_of(&layout), "Supply USDC to Jupiter Lend");
    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(&layout, "Recipient")
    else {
        panic!("Recipient must be an address_v2");
    };
    assert_eq!(address_v2.name, "Ownership not verified");
    assert_eq!(address_v2.badge_text.as_deref(), Some("UNVERIFIED"));
}

/// Token-2022 is the other real token program: the signer's Token-2022 ATA is recognized.
#[test]
fn test_recipient_under_token_2022_is_the_signers_ata() {
    let signer = Pubkey::new_unique();
    let token_program = token_2022_program_id();
    let recipient = get_associated_token_address_with_program_id(
        &signer,
        &Pubkey::from_str(JL_USDC_MINT).unwrap(),
        &token_program,
    );
    let instruction = synthetic_instruction(
        "deposit",
        &[10_000_000],
        &[
            ("signer", &signer.to_string()),
            ("recipient_token_account", &recipient.to_string()),
            ("token_program", &token_program.to_string()),
        ],
    );
    let layout = render(&instruction);

    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(&layout, "Recipient")
    else {
        panic!("Recipient must be an address_v2");
    };
    assert_eq!(address_v2.address, recipient.to_string());
    assert_eq!(address_v2.name, "Signer's associated token account");
    assert_eq!(address_v2.badge_text, None);
}

/// Admin instructions are not user actions: generic condensed view.
#[test]
fn test_admin_instruction_keeps_generic_view() {
    let layout = render(&synthetic_instruction("update_rate", &[], &[]));
    assert_eq!(title_of(&layout), "Jupiter Lend Earn: update_rate");
    assert!(condensed_value(&layout, "Amount").is_none());
}

// ---------------------------------------------------------------------------
// Instruction families without a fixture, assembled from the IDL.
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_with_min_amount_out() {
    let layout = render(&synthetic_instruction(
        "deposit_with_min_amount_out",
        &[100_000_000, 95_000_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Supply USDC to Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "100");
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "95 jlUSDC"
    );
}

#[test]
fn test_mint() {
    let layout = render(&synthetic_instruction("mint", &[50_000_000], &[]));
    assert_eq!(title_of(&layout), "Mint jlUSDC on Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Mint");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), "50");
    assert!(
        condensed_value(&layout, "Pay").unwrap().starts_with("USDC"),
        "mint pays the asset"
    );
}

#[test]
fn test_mint_with_max_assets() {
    let layout = render(&synthetic_instruction(
        "mint_with_max_assets",
        &[50_000_000, 52_500_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Mint jlUSDC on Jupiter Lend");
    assert_eq!(
        condensed_value(&layout, "Maximum paid").unwrap(),
        "52.5 USDC"
    );
}

#[test]
fn test_withdraw_with_max_shares_burn() {
    let layout = render(&synthetic_instruction(
        "withdraw_with_max_shares_burn",
        &[10_000_000, 9_800_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Redeem jlUSDC for USDC via Jupiter Lend");
    assert!(
        condensed_value(&layout, "Burn")
            .unwrap()
            .starts_with("jlUSDC")
    );
    assert_eq!(
        condensed_value(&layout, "Maximum burned").unwrap(),
        "9.8 jlUSDC"
    );
}

#[test]
fn test_redeem() {
    let layout = render(&synthetic_instruction("redeem", &[1_500_000], &[]));
    assert_eq!(title_of(&layout), "Redeem jlUSDC via Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Action").unwrap(), "Redeem");
    assert!(
        condensed_value(&layout, "Receive")
            .unwrap()
            .starts_with("USDC")
    );
}

#[test]
fn test_redeem_with_min_amount_out() {
    let layout = render(&synthetic_instruction(
        "redeem_with_min_amount_out",
        &[1_500_000, 1_490_000],
        &[],
    ));
    assert_eq!(title_of(&layout), "Redeem jlUSDC via Jupiter Lend");
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "1.49 USDC"
    );
}

/// The recipient token account is the one account the IDL leaves unconstrained,
/// so it is always shown, and classified as the signer's own associated token
/// account or not. The fixture deposit mints its receipt tokens to the signer.
#[test]
fn test_recipient_row_marks_the_signers_own_account() {
    let layout = render(&fixture("deposit_usdc"));
    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(&layout, "Recipient")
    else {
        panic!("Recipient must be an address_v2");
    };
    assert_eq!(
        address_v2.address,
        "678f85kKQLNkg6eNhnUmTRXk3Z4LCSKgsVAGW5KPtvq"
    );
    assert_eq!(address_v2.name, "Signer's associated token account");
    assert_eq!(address_v2.badge_text, None);
}

/// A withdraw paying out to someone else's token account must say so: same
/// title, same amount, but a flagged recipient.
#[test]
fn test_recipient_row_flags_a_third_party_account() {
    let mut instruction = fixture("withdraw_jupusd");
    instruction.accounts[2].pubkey = Pubkey::new_unique(); // `recipient_token_account`
    let layout = render(&instruction);

    assert_eq!(
        title_of(&layout),
        "Redeem JUICED for JupUSD via Jupiter Lend"
    );
    let SignablePayloadField::AddressV2 { address_v2, .. } = condensed_field(&layout, "Recipient")
    else {
        panic!("Recipient must be an address_v2");
    };
    assert_eq!(
        address_v2.address,
        instruction.accounts[2].pubkey.to_string()
    );
    assert_eq!(address_v2.name, "Not the signer's associated token account");
    assert_eq!(address_v2.badge_text.as_deref(), Some("THIRD PARTY"));
}

/// Anchor lets a client omit trailing optional accounts. With fewer accounts than
/// the IDL lists, positional naming would label the wrong pubkey (for example the
/// wrong account as `token_program`), so the recipient check could misfire and a
/// self-withdraw could be badged as third party. The instruction must keep the
/// generic view instead.
#[test]
fn test_fewer_accounts_than_idl_keeps_generic_view() {
    let mut instruction = fixture("withdraw_jupusd");
    instruction.accounts.pop(); // drop the trailing `system_program`
    let layout = render(&instruction);

    assert_eq!(title_of(&layout), "Jupiter Lend Earn: withdraw");
    assert!(condensed_value(&layout, "Recipient").is_none());
    assert!(condensed_value(&layout, "Amount").is_none());
}

/// Only `withdraw(u64::MAX)` has verified sentinel semantics. Everywhere else
/// the literal is shown and flagged; no "maximum" or "all" is claimed.
#[test]
fn test_u64_max_outside_withdraw_renders_the_flagged_literal() {
    const LITERAL: &str = "18446744073709551615 raw units of jlUSDC (u64::MAX)";

    let layout = render(&synthetic_instruction("redeem", &[u64::MAX], &[]));
    assert_eq!(title_of(&layout), "Redeem jlUSDC via Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), LITERAL);

    let layout = render(&synthetic_instruction("mint", &[u64::MAX], &[]));
    assert_eq!(title_of(&layout), "Mint jlUSDC on Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), LITERAL);

    let layout = render(&synthetic_instruction("deposit", &[u64::MAX], &[]));
    assert_eq!(title_of(&layout), "Supply USDC to Jupiter Lend");
}

/// The full-exit sentinel is verified for plain `withdraw` only. The capped
/// variant with `u64::MAX` renders the flagged literal, an estimated Burn row
/// and the cap, so no row contradicts another.
#[test]
fn test_capped_withdraw_with_u64_max_renders_the_flagged_literal() {
    let layout = render(&synthetic_instruction(
        "withdraw_with_max_shares_burn",
        &[u64::MAX, 9_800_000],
        &[],
    ));
    const LITERAL: &str = "18446744073709551615 raw units of USDC (u64::MAX)";
    assert_eq!(title_of(&layout), "Redeem jlUSDC for USDC via Jupiter Lend");
    assert_eq!(condensed_value(&layout, "Amount").unwrap(), LITERAL);
    assert!(
        condensed_value(&layout, "Burn")
            .unwrap()
            .starts_with("jlUSDC")
    );
    assert_eq!(
        condensed_value(&layout, "Maximum burned").unwrap(),
        "9.8 jlUSDC"
    );
}

/// `u64::MAX` disables a cap, but it does not disable a floor: a minimum of
/// `u64::MAX` is an unsatisfiable requirement and must be shown as the literal.
#[test]
fn test_u64_max_bound_is_no_limit_only_for_maximums() {
    let layout = render(&synthetic_instruction(
        "withdraw_with_max_shares_burn",
        &[10_000_000, u64::MAX],
        &[],
    ));
    assert_eq!(
        condensed_value(&layout, "Maximum burned").unwrap(),
        "no limit"
    );

    let layout = render(&synthetic_instruction(
        "mint_with_max_assets",
        &[50_000_000, u64::MAX],
        &[],
    ));
    assert_eq!(
        condensed_value(&layout, "Maximum paid").unwrap(),
        "no limit"
    );

    let layout = render(&synthetic_instruction(
        "deposit_with_min_amount_out",
        &[10_000_000, u64::MAX],
        &[],
    ));
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "18446744073709551615 raw units of jlUSDC (u64::MAX)"
    );

    let layout = render(&synthetic_instruction(
        "redeem_with_min_amount_out",
        &[10_000_000, u64::MAX],
        &[],
    ));
    assert_eq!(
        condensed_value(&layout, "Minimum received").unwrap(),
        "18446744073709551615 raw units of USDC (u64::MAX)"
    );
}
