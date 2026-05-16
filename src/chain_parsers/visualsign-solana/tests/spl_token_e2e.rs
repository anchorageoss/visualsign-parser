#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Full-pipeline integration tests for the SPL Token preset.
//!
//! Drives the complete stack end-to-end via the public
//! `transaction_to_visual_sign` API:
//!
//!   SolanaTransaction (built with spl_token::instruction::*)
//!     -> transaction_to_visual_sign
//!       -> SolanaVisualSignConverter
//!         -> SplTokenVisualizer  (dispatched by program id)
//!         -> SignablePayload
//!
//! Each test asserts that the rendered Expanded layout surfaces the mint
//! pubkey (or, for variants whose accounts cannot carry it, the explicit
//! "Token Mint" notice). The unit tests in `presets/spl_token/tests.rs`
//! exercise the visualizer in isolation; these tests prove the dispatch +
//! conversion chain works for real Transaction inputs.

mod common;

use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use spl_token::instruction as token_instruction;
use visualsign_solana::transaction_to_visual_sign;

use common::{find_text, instruction_fields, options_no_idl};

fn build_tx(instruction: solana_sdk::instruction::Instruction) -> SolanaTransaction {
    let fee_payer = Pubkey::new_unique();
    SolanaTransaction::new_unsigned(Message::new(&[instruction], Some(&fee_payer)))
}

#[test]
fn pipeline_initialize_mint_surfaces_mint_decimals_and_authorities() {
    let mint = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();
    let freeze_authority = Pubkey::new_unique();

    let ix = token_instruction::initialize_mint(
        &spl_token::id(),
        &mint,
        &mint_authority,
        Some(&freeze_authority),
        9,
    )
    .unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let layouts = instruction_fields(&payload);
    assert_eq!(layouts.len(), 1);
    let expanded = layouts[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(find_text(&expanded.fields, "Decimals"), Some("9".into()));
    assert_eq!(
        find_text(&expanded.fields, "Mint Authority"),
        Some(mint_authority.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Freeze Authority"),
        Some(freeze_authority.to_string())
    );
}

#[test]
fn pipeline_initialize_mint2_surfaces_mint_decimals_and_authorities() {
    let mint = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();

    let ix = token_instruction::initialize_mint2(&spl_token::id(), &mint, &mint_authority, None, 6)
        .unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let layouts = instruction_fields(&payload);
    let expanded = layouts[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(find_text(&expanded.fields, "Decimals"), Some("6".into()));
    assert_eq!(
        find_text(&expanded.fields, "Mint Authority"),
        Some(mint_authority.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Freeze Authority"),
        Some("None".into())
    );
}

#[test]
fn pipeline_initialize_account_surfaces_account_mint_and_owner() {
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix =
        token_instruction::initialize_account(&spl_token::id(), &account, &mint, &owner).unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Account"),
        Some(account.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Owner"),
        Some(owner.to_string())
    );
}

#[test]
fn pipeline_initialize_account3_surfaces_mint_and_owner_from_instruction_data() {
    // InitializeAccount3 carries the owner in instruction data rather than
    // as an account, so it tests the "owner from data" code path.
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix =
        token_instruction::initialize_account3(&spl_token::id(), &account, &mint, &owner).unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Account"),
        Some(account.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Owner"),
        Some(owner.to_string())
    );
}

#[test]
fn pipeline_freeze_account_surfaces_account_mint_and_authority() {
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let freeze_authority = Pubkey::new_unique();

    let ix = token_instruction::freeze_account(
        &spl_token::id(),
        &account,
        &mint,
        &freeze_authority,
        &[],
    )
    .unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Account"),
        Some(account.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Freeze Authority"),
        Some(freeze_authority.to_string())
    );
}

#[test]
fn pipeline_thaw_account_surfaces_account_mint_and_authority() {
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let freeze_authority = Pubkey::new_unique();

    let ix =
        token_instruction::thaw_account(&spl_token::id(), &account, &mint, &freeze_authority, &[])
            .unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Account"),
        Some(account.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Freeze Authority"),
        Some(freeze_authority.to_string())
    );
}

#[test]
fn pipeline_close_account_surfaces_account_destination_and_owner() {
    // CloseAccount cannot carry the mint in its instruction. The rendered output
    // should surface the three account fields cleanly and omit the mint field
    // entirely (mint absence is reported via tracing::debug! for operators).
    let account = Pubkey::new_unique();
    let dest = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix =
        token_instruction::close_account(&spl_token::id(), &account, &dest, &owner, &[]).unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Account"),
        Some(account.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Destination"),
        Some(dest.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Owner"),
        Some(owner.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        None,
        "CloseAccount should not render a Token Mint field"
    );
}

#[test]
fn pipeline_revoke_surfaces_source_and_owner() {
    // Revoke cannot carry the mint. Source + Owner surface; no Token Mint field.
    let source = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix = token_instruction::revoke(&spl_token::id(), &source, &owner, &[]).unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Source"),
        Some(source.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Owner"),
        Some(owner.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        None,
        "Revoke should not render a Token Mint field"
    );
}

#[test]
fn pipeline_unchecked_transfer_renders_without_mint_field() {
    // Transfer (unchecked) cannot carry the mint in its instruction. The
    // rendered output surfaces Source/Destination/Owner and omits the mint
    // field entirely (a tracing::debug! line flags the unchecked variant for
    // operators, but it does not pollute the wallet view).
    let source = Pubkey::new_unique();
    let dest = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix =
        token_instruction::transfer(&spl_token::id(), &source, &dest, &owner, &[], 1_000).unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Source"),
        Some(source.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Destination"),
        Some(dest.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Owner"),
        Some(owner.to_string())
    );
    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        None,
        "unchecked Transfer should not render a Token Mint field"
    );
}

#[test]
fn pipeline_transfer_checked_surfaces_mint_explicitly() {
    // TransferChecked is the "happy path" — mint is in the instruction
    // accounts and surfaces directly.
    let source = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let dest = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let ix = token_instruction::transfer_checked(
        &spl_token::id(),
        &source,
        &mint,
        &dest,
        &owner,
        &[],
        1_000,
        6,
    )
    .unwrap();

    let payload = transaction_to_visual_sign(build_tx(ix), options_no_idl()).unwrap();
    let expanded = instruction_fields(&payload)[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "Token Mint"),
        Some(mint.to_string()),
        "TransferChecked should surface the literal mint pubkey"
    );
    assert_eq!(find_text(&expanded.fields, "Decimals"), Some("6".into()));
}
