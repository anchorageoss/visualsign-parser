//! Property-based fuzz tests for IDL instruction parsing.
//!
//! These tests verify that `decode_idl_data` and `parse_instruction_with_idl`
//! (from `solana_parser`) never panic regardless of:
//!
//! - IDL shape: varying instruction counts, argument counts, and argument types
//! - Instruction data bytes: fully random, correct-discriminator prefix, empty, overlong
//!
//! Run: `cargo test --test fuzz_idl_parsing`
//! More iterations: `PROPTEST_CASES=5000 cargo test --test fuzz_idl_parsing`

use proptest::prelude::*;
use solana_parser::{decode_idl_data, parse_instruction_with_idl};

const TEST_PROGRAM_ID: &str = "11111111111111111111111111111111";

// ── Strategies ───────────────────────────────────────────────────────────────

/// All primitive IDL types in their JSON wire format (as expected by `decode_idl_data`).
fn arb_primitive_type() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![
        Just(serde_json::json!("bool")),
        Just(serde_json::json!("u8")),
        Just(serde_json::json!("u16")),
        Just(serde_json::json!("u32")),
        Just(serde_json::json!("u64")),
        Just(serde_json::json!("u128")),
        Just(serde_json::json!("i8")),
        Just(serde_json::json!("i16")),
        Just(serde_json::json!("i32")),
        Just(serde_json::json!("i64")),
        Just(serde_json::json!("i128")),
        Just(serde_json::json!("f32")),
        Just(serde_json::json!("f64")),
        Just(serde_json::json!("publicKey")),
        Just(serde_json::json!("string")),
        Just(serde_json::json!("bytes")),
    ]
}

/// IDL type: a primitive or a container (Vec, Option, Array) wrapping a primitive.
fn arb_idl_type() -> impl Strategy<Value = serde_json::Value> {
    arb_primitive_type().prop_flat_map(|prim| {
        let p_vec = prim.clone();
        let p_opt = prim.clone();
        let p_arr = prim.clone();
        prop_oneof![
            // Weighted 4:1:1:1 — most fields are primitives, containers less frequent.
            4 => Just(prim),
            1 => Just(serde_json::json!({"vec": p_vec})),
            1 => Just(serde_json::json!({"option": p_opt})),
            1 => (1usize..=4).prop_map(move |n| serde_json::json!({"array": [p_arr.clone(), n]})),
        ]
    })
}

/// Valid identifier: starts with [a-z], followed by 1–15 lowercase alphanumeric chars.
fn arb_identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{1,15}"
}

/// Random IDL instruction: a name + 0–20 args of randomly-chosen types.
fn arb_idl_instruction() -> impl Strategy<Value = serde_json::Value> {
    (
        arb_identifier(),
        prop::collection::vec(
            (arb_identifier(), arb_idl_type())
                .prop_map(|(name, ty)| serde_json::json!({"name": name, "type": ty})),
            0..=20,
        ),
    )
        .prop_map(|(name, args)| {
            serde_json::json!({
                "name": name,
                "accounts": [],
                "args": args,
            })
        })
}

/// Full IDL JSON string with 1–16 randomly-structured instructions.
fn arb_idl_json() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_idl_instruction(), 1..=16).prop_map(|instructions| {
        serde_json::json!({
            "instructions": instructions,
            "types": [],
        })
        .to_string()
    })
}

// ── Crash-safety property tests ──────────────────────────────────────────────

proptest! {
    // Default is 256 cases. Override with PROPTEST_CASES=5000 for deeper fuzzing.
    #![proptest_config(ProptestConfig::default())]

    /// Core crash-safety test: a random IDL paired with random instruction bytes
    /// must never cause a panic — only `Ok` or a clean `Err`.
    #[test]
    fn fuzz_idl_parsing_never_panics(
        idl_json in arb_idl_json(),
        data in prop::collection::vec(any::<u8>(), 0..200usize),
    ) {
        // If the IDL itself fails to decode, that's fine; we only care about panics.
        if let Ok(idl) = decode_idl_data(&idl_json) {
            let _ = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
        }
    }

    /// `decode_idl_data` must not panic on completely arbitrary string input.
    #[test]
    fn fuzz_decode_idl_data_arbitrary_input(s in any::<String>()) {
        let _ = decode_idl_data(&s);
    }

    /// Take a valid 8-byte discriminator from a randomly-selected instruction
    /// (not always the first) and append random arg bytes up to MAX_CURSOR_LENGTH
    /// (1232).  The parser must return `Ok` or a clean `Err` — never a panic.
    #[test]
    fn fuzz_valid_discriminator_random_args(
        idl_json in arb_idl_json(),
        inst_idx in any::<usize>(),
        arg_bytes in prop::collection::vec(any::<u8>(), 0..1300usize),
    ) {
        if let Ok(idl) = decode_idl_data(&idl_json) {
            if !idl.instructions.is_empty() {
                let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                if let Some(disc) = &inst.discriminator {
                    let mut data = disc.clone();
                    data.extend_from_slice(&arg_bytes);
                    let _ = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
                }
            }
        }
    }
}

// ── Valid-data roundtrip tests ────────────────────────────────────────────────
//
// These tests construct an IDL, extract the computed discriminator, then build
// correctly-serialized instruction data and assert that parsing succeeds.

#[test]
fn roundtrip_no_args() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "initialize", "accounts": [], "args": []}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let result = parse_instruction_with_idl(disc, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.instruction_name, "initialize");
    assert!(result.program_call_args.is_empty());
}

#[test]
fn roundtrip_single_u64_arg() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "deposit", "accounts": [], "args": [
            {"name": "amount", "type": "u64"}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.extend_from_slice(&42u64.to_le_bytes());

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.instruction_name, "deposit");
    assert_eq!(result.program_call_args["amount"], serde_json::json!(42));
}

#[test]
fn roundtrip_mixed_primitive_args() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "swap", "accounts": [], "args": [
            {"name": "amountIn",  "type": "u64"},
            {"name": "minOut",    "type": "u64"},
            {"name": "slippage",  "type": "u16"},
            {"name": "isExact",   "type": "bool"},
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.extend_from_slice(&1000u64.to_le_bytes()); // amountIn
    data.extend_from_slice(&900u64.to_le_bytes());  // minOut
    data.extend_from_slice(&50u16.to_le_bytes());   // slippage
    data.push(1u8);                                  // isExact = true

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.instruction_name, "swap");
    assert_eq!(result.program_call_args["amountIn"], serde_json::json!(1000));
    assert_eq!(result.program_call_args["minOut"],   serde_json::json!(900));
    assert_eq!(result.program_call_args["slippage"], serde_json::json!(50));
    assert_eq!(result.program_call_args["isExact"],  serde_json::json!(true));
}

#[test]
fn roundtrip_option_some() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "setFee", "accounts": [], "args": [
            {"name": "feeBps", "type": {"option": "u16"}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.push(1u8);                               // Some
    data.extend_from_slice(&300u16.to_le_bytes());

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.program_call_args["feeBps"], serde_json::json!(300));
}

#[test]
fn roundtrip_option_none() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "setFee", "accounts": [], "args": [
            {"name": "feeBps", "type": {"option": "u16"}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.push(0u8); // None

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.program_call_args["feeBps"], serde_json::Value::Null);
}

#[test]
fn roundtrip_vec_u8() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "writeData", "accounts": [], "args": [
            {"name": "payload", "type": {"vec": "u8"}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let elements: [u8; 3] = [10, 20, 30];
    let mut data = disc.clone();
    data.extend_from_slice(&(elements.len() as u32).to_le_bytes()); // u32 length prefix
    data.extend_from_slice(&elements);

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(
        result.program_call_args["payload"],
        serde_json::json!([10, 20, 30])
    );
}

#[test]
fn roundtrip_array_u32() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "setParams", "accounts": [], "args": [
            {"name": "limits", "type": {"array": ["u32", 3]}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    for val in [100u32, 200, 300] {
        data.extend_from_slice(&val.to_le_bytes());
    }

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(
        result.program_call_args["limits"],
        serde_json::json!([100, 200, 300])
    );
}

#[test]
fn roundtrip_multiple_instructions_distinct_dispatch() {
    // IDL with 3 instructions; verify each is dispatched by its own discriminator.
    let idl_json = serde_json::json!({
        "instructions": [
            {"name": "initialize", "accounts": [], "args": []},
            {"name": "deposit",    "accounts": [], "args": [{"name": "amount", "type": "u32"}]},
            {"name": "withdraw",   "accounts": [], "args": [
                {"name": "amount", "type": "u32"},
                {"name": "all",    "type": "bool"},
            ]},
        ],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();

    // initialize — no args
    let disc0 = idl.instructions[0].discriminator.as_ref().unwrap();
    let r = parse_instruction_with_idl(disc0, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(r.instruction_name, "initialize");
    assert!(r.program_call_args.is_empty());

    // deposit — single u32
    let disc1 = idl.instructions[1].discriminator.as_ref().unwrap();
    let mut data1 = disc1.clone();
    data1.extend_from_slice(&99u32.to_le_bytes());
    let r = parse_instruction_with_idl(&data1, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(r.instruction_name, "deposit");
    assert_eq!(r.program_call_args["amount"], serde_json::json!(99));

    // withdraw — u32 + bool
    let disc2 = idl.instructions[2].discriminator.as_ref().unwrap();
    let mut data2 = disc2.clone();
    data2.extend_from_slice(&50u32.to_le_bytes());
    data2.push(0u8); // all = false
    let r = parse_instruction_with_idl(&data2, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(r.instruction_name, "withdraw");
    assert_eq!(r.program_call_args["amount"], serde_json::json!(50));
    assert_eq!(r.program_call_args["all"],    serde_json::json!(false));
}
