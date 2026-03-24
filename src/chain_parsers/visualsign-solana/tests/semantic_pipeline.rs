//! Semantic pipeline tests: real embedded IDLs with realistic instruction data.
//!
//! Each test uses a real embedded IDL and constructs valid Borsh-serialized
//! instruction data for a characteristic instruction. The narrative reflects
//! what a real user of the protocol would do. We verify:
//! - The IDL code path is taken (title contains "(IDL)")
//! - The instruction name is correctly identified
//! - Decoded arg values match what was serialized
//!
//! Contrast with pipeline_integration.rs, which tests the pipeline with
//! handcrafted minimal IDLs and proptest-generated random IDLs.

mod common;

use solana_parser::decode_idl_data;
use solana_parser::solana::embedded_idls;
use solana_sdk::pubkey::Pubkey;
use visualsign::SignablePayload;
use visualsign_solana::transaction_to_visual_sign;

use common::{build_transaction, find_text, instruction_fields, options_with_idl};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Look up an instruction by name in a parsed IDL and return its discriminator.
fn disc_for(idl: &solana_parser::solana::structs::Idl, name: &str) -> Vec<u8> {
    idl.instructions
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("instruction '{name}' not found in IDL"))
        .discriminator
        .clone()
        .unwrap_or_else(|| panic!("no discriminator for '{name}'"))
}

/// Assert the pipeline decoded an instruction with the expected name and arg values.
fn assert_decoded(
    payload: &SignablePayload,
    expected_instruction: &str,
    expected_args: &[(&str, &str)],
) {
    let inst_fields = instruction_fields(payload);
    assert!(!inst_fields.is_empty(), "no instruction fields in payload");
    let layout = inst_fields[0];

    let title = layout.title.as_ref().unwrap().text.as_str();
    assert!(
        title.contains("(IDL)"),
        "expected IDL path, got title: {title}"
    );

    let condensed = layout.condensed.as_ref().unwrap();
    assert_eq!(
        find_text(&condensed.fields, "Instruction"),
        Some(expected_instruction.to_string()),
        "wrong instruction name decoded"
    );
    for (label, value) in expected_args {
        assert_eq!(
            find_text(&condensed.fields, label),
            Some(value.to_string()),
            "wrong value for arg '{label}'"
        );
    }
}

/// Run a semantic test: parse the IDL, look up the instruction, build instruction
/// data from the discriminator + arg bytes, run the pipeline, and assert the
/// decoded instruction name and arg values match.
fn assert_semantic(
    idl_json: &str,
    instruction_name: &str,
    build_args: impl FnOnce(&mut Vec<u8>),
    expected_args: &[(&str, &str)],
) {
    let idl = decode_idl_data(idl_json).unwrap();
    let mut data = disc_for(&idl, instruction_name);
    build_args(&mut data);

    let program_id = Pubkey::new_unique();
    let payload = transaction_to_visual_sign(
        build_transaction(program_id, vec![], data),
        options_with_idl(&program_id, idl_json, "Test"),
    )
    .unwrap();

    assert_decoded(&payload, instruction_name, expected_args);
}

// ── Semantic tests ───────────────────────────────────────────────────────────

/// Drift: a trader deposits 5 USDC into spot market 0.
/// Drift is a derivatives protocol; deposit is the entry point for margin trading.
#[test]
fn semantic_drift_deposit() {
    assert_semantic(
        embedded_idls::DRIFT_IDL,
        "deposit",
        |data| {
            // Args: marketIndex: u16, amount: u64, reduceOnly: bool
            data.extend_from_slice(&0u16.to_le_bytes()); // marketIndex = 0 (USDC spot)
            data.extend_from_slice(&5_000_000u64.to_le_bytes()); // amount = 5 USDC (6 decimals)
            data.push(0u8); // reduceOnly = false
        },
        &[
            ("marketIndex", "0"),
            ("amount", "5000000"),
            ("reduceOnly", "false"),
        ],
    );
}

/// Lifinity: a user swaps 1 SOL for at least 150 USDC through the oracle-based AMM.
/// Lifinity is a proactive market maker that uses oracle prices.
#[test]
fn semantic_lifinity_swap() {
    assert_semantic(
        embedded_idls::LIFINITY_IDL,
        "swap",
        |data| {
            // Args: amountIn: u64, minimumAmountOut: u64
            data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL (9 decimals)
            data.extend_from_slice(&150_000_000u64.to_le_bytes()); // min 150 USDC (6 decimals)
        },
        &[
            ("amountIn", "1000000000"),
            ("minimumAmountOut", "150000000"),
        ],
    );
}

/// Raydium: a user swaps 100 USDC for at least 0.5 SOL on the concentrated liquidity pool.
/// Raydium is a leading Solana AMM with fusion pools.
#[test]
fn semantic_raydium_swap() {
    assert_semantic(
        embedded_idls::RAYDIUM_IDL,
        "swapBaseInput",
        |data| {
            // Args: amountIn: u64, minimumAmountOut: u64
            data.extend_from_slice(&100_000_000u64.to_le_bytes()); // 100 USDC
            data.extend_from_slice(&500_000_000u64.to_le_bytes()); // min 0.5 SOL
        },
        &[("amountIn", "100000000"), ("minimumAmountOut", "500000000")],
    );
}

/// Orca: a user swaps 2 SOL -> USDC on a Whirlpool concentrated liquidity pool.
/// Orca Whirlpools use tick-based concentrated liquidity similar to Uniswap V3.
#[test]
fn semantic_orca_swap() {
    assert_semantic(
        embedded_idls::ORCA_IDL,
        "swap",
        |data| {
            // Args: amount: u64, otherAmountThreshold: u64, sqrtPriceLimit: u128,
            //       amountSpecifiedIsInput: bool, aToB: bool
            data.extend_from_slice(&2_000_000_000u64.to_le_bytes()); // 2 SOL
            data.extend_from_slice(&280_000_000u64.to_le_bytes()); // min ~280 USDC
            data.extend_from_slice(&4_295_048_016u128.to_le_bytes()); // sqrtPriceLimit
            data.push(1u8); // amountSpecifiedIsInput = true
            data.push(1u8); // aToB = true (SOL -> USDC)
        },
        &[
            ("amount", "2000000000"),
            ("otherAmountThreshold", "280000000"),
            ("sqrtPriceLimit", "\"4295048016\""), // u128 values are quoted by the pipeline
            ("amountSpecifiedIsInput", "true"),
            ("aToB", "true"),
        ],
    );
}

/// Meteora: a user swaps 50 USDC through a DLMM liquidity bin pair.
/// Meteora uses a discretized liquidity model with price bins.
#[test]
fn semantic_meteora_swap() {
    assert_semantic(
        embedded_idls::METEORA_IDL,
        "swap",
        |data| {
            // Args: amountIn: u64, minAmountOut: u64
            data.extend_from_slice(&50_000_000u64.to_le_bytes()); // 50 USDC
            data.extend_from_slice(&300_000_000u64.to_le_bytes()); // min 0.3 SOL
        },
        &[("amountIn", "50000000"), ("minAmountOut", "300000000")],
    );
}

/// Kamino: a user deposits token A and token B into a liquidity strategy vault.
/// Kamino automates concentrated liquidity management on Orca/Raydium.
#[test]
fn semantic_kamino_deposit() {
    assert_semantic(
        embedded_idls::KAMINO_IDL,
        "deposit",
        |data| {
            // Args: tokenMaxA: u64, tokenMaxB: u64
            data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL
            data.extend_from_slice(&150_000_000u64.to_le_bytes()); // 150 USDC
        },
        &[("tokenMaxA", "1000000000"), ("tokenMaxB", "150000000")],
    );
}

/// Stabble: a user swaps 1 USDC for at least 0.99 USDT through the stable swap pool.
/// Stabble is a stablecoin-optimized AMM. The amount_in arg is Option<u64>
/// (Borsh: 1-byte tag + LE value for Some).
#[test]
fn semantic_stabble_swap() {
    assert_semantic(
        embedded_idls::STABBLE_IDL,
        "swap",
        |data| {
            // Args: amount_in: Option<u64>, minimum_amount_out: u64
            data.push(1u8); // Some tag
            data.extend_from_slice(&1_000_000u64.to_le_bytes()); // Some(1_000_000) = 1 USDC
            data.extend_from_slice(&990_000u64.to_le_bytes()); // min 0.99 USDT
        },
        &[("amount_in", "1000000"), ("minimum_amount_out", "990000")],
    );
}

/// OpenBook: a user deposits base and quote tokens into their open orders account.
/// OpenBook is a decentralized order book for Solana.
#[test]
fn semantic_openbook_deposit() {
    assert_semantic(
        embedded_idls::OPENBOOK_IDL,
        "deposit",
        |data| {
            // Args: baseAmount: u64, quoteAmount: u64
            data.extend_from_slice(&10_000_000_000u64.to_le_bytes()); // 10 SOL
            data.extend_from_slice(&1_500_000_000u64.to_le_bytes()); // 1500 USDC
        },
        &[("baseAmount", "10000000000"), ("quoteAmount", "1500000000")],
    );
}
