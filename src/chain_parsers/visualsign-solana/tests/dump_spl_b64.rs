#![allow(clippy::unwrap_used)]
//! Print base64-encoded SPL Token transactions to stdout, one per instruction
//! variant the preset claims to handle. Run with `cargo test --test
//! dump_spl_b64 -- --nocapture --include-ignored` to capture the output and
//! feed it to `parser_cli`.
//!
//! Marked `#[ignore]` so it doesn't run in normal CI; this is a dev tool for
//! generating reproducible base64 examples without depending on mainnet RPC.

use base64::Engine;
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use spl_token::instruction as token_instruction;

fn b64_of(ix: solana_sdk::instruction::Instruction) -> String {
    let fee_payer = Pubkey::new_unique();
    let tx = SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&fee_payer)));
    let bytes = bincode::serialize(&tx).unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn print_case(name: &str, ix: solana_sdk::instruction::Instruction) {
    println!("\n=== {name} ===");
    println!("{}", b64_of(ix));
}

#[test]
#[ignore]
fn dump_all_spl_token_variants() {
    // Deterministic stand-in pubkeys so the same `cargo test` invocation
    // always produces the same base64 (until the rng is initialized).
    let mint = Pubkey::from([1u8; 32]);
    let account = Pubkey::from([2u8; 32]);
    let source = Pubkey::from([3u8; 32]);
    let dest = Pubkey::from([4u8; 32]);
    let owner = Pubkey::from([5u8; 32]);
    let mint_authority = Pubkey::from([6u8; 32]);
    let freeze_authority = Pubkey::from([7u8; 32]);
    let delegate = Pubkey::from([8u8; 32]);

    print_case(
        "InitializeMint (decimals=9, freeze authority set)",
        token_instruction::initialize_mint(
            &spl_token::id(),
            &mint,
            &mint_authority,
            Some(&freeze_authority),
            9,
        )
        .unwrap(),
    );

    print_case(
        "InitializeMint2 (decimals=6, no freeze authority)",
        token_instruction::initialize_mint2(&spl_token::id(), &mint, &mint_authority, None, 6)
            .unwrap(),
    );

    print_case(
        "InitializeAccount",
        token_instruction::initialize_account(&spl_token::id(), &account, &mint, &owner).unwrap(),
    );

    print_case(
        "InitializeAccount3 (owner in instruction data)",
        token_instruction::initialize_account3(&spl_token::id(), &account, &mint, &owner).unwrap(),
    );

    print_case(
        "MintTo (amount=1230000000)",
        token_instruction::mint_to(
            &spl_token::id(),
            &mint,
            &account,
            &mint_authority,
            &[],
            1_230_000_000,
        )
        .unwrap(),
    );

    print_case(
        "MintToChecked (amount=1230000000, decimals=6)",
        token_instruction::mint_to_checked(
            &spl_token::id(),
            &mint,
            &account,
            &mint_authority,
            &[],
            1_230_000_000,
            6,
        )
        .unwrap(),
    );

    print_case(
        "Transfer (unchecked, amount=1000)",
        token_instruction::transfer(&spl_token::id(), &source, &dest, &owner, &[], 1_000).unwrap(),
    );

    print_case(
        "TransferChecked (amount=1000, decimals=6)",
        token_instruction::transfer_checked(
            &spl_token::id(),
            &source,
            &mint,
            &dest,
            &owner,
            &[],
            1_000,
            6,
        )
        .unwrap(),
    );

    print_case(
        "Burn (amount=500)",
        token_instruction::burn(&spl_token::id(), &account, &mint, &owner, &[], 500).unwrap(),
    );

    print_case(
        "BurnChecked (amount=500, decimals=6)",
        token_instruction::burn_checked(&spl_token::id(), &account, &mint, &owner, &[], 500, 6)
            .unwrap(),
    );

    print_case(
        "Approve (unchecked, amount=10000)",
        token_instruction::approve(&spl_token::id(), &source, &delegate, &owner, &[], 10_000)
            .unwrap(),
    );

    print_case(
        "ApproveChecked (amount=10000, decimals=6)",
        token_instruction::approve_checked(
            &spl_token::id(),
            &source,
            &mint,
            &delegate,
            &owner,
            &[],
            10_000,
            6,
        )
        .unwrap(),
    );

    print_case(
        "Revoke",
        token_instruction::revoke(&spl_token::id(), &source, &owner, &[]).unwrap(),
    );

    print_case(
        "FreezeAccount",
        token_instruction::freeze_account(
            &spl_token::id(),
            &account,
            &mint,
            &freeze_authority,
            &[],
        )
        .unwrap(),
    );

    print_case(
        "ThawAccount",
        token_instruction::thaw_account(&spl_token::id(), &account, &mint, &freeze_authority, &[])
            .unwrap(),
    );

    print_case(
        "CloseAccount",
        token_instruction::close_account(&spl_token::id(), &account, &dest, &owner, &[]).unwrap(),
    );
}
