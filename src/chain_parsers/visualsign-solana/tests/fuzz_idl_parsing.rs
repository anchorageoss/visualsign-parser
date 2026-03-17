//! Property-based fuzz tests for IDL instruction parsing.
//!
//! These tests verify that `decode_idl_data` and `parse_instruction_with_idl`
//! (from `solana_parser`) never panic regardless of:
//!
//! - IDL shape: varying instruction counts, argument counts, and argument types
//! - Instruction data bytes: fully random, correct-discriminator prefix, empty, overlong
//! - Defined types (structs) referenced from instruction args
//! - Nested container types: `Vec<Option<T>>`, `Option<Vec<T>>`
//! - SizeGuard boundary: large Vec/String length prefixes with little backing data
//!
//! Run: `cargo test --test fuzz_idl_parsing`
//! More iterations: `PROPTEST_CASES=5000 cargo test --test fuzz_idl_parsing`

use proptest::prelude::*;
use solana_parser::solana::structs::{
    EnumFields, IdlInstruction, IdlType, IdlTypeDefinition, IdlTypeDefinitionType,
};
use solana_parser::{decode_idl_data, parse_instruction_with_idl};
use std::sync::Arc;

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

/// IDL type: a primitive or a container (Vec, Option, Array, or nested combo)
/// wrapping a primitive.
///
/// Weights: 4 primitive : 1 Vec : 1 Option : 1 Array : 1 Vec<Option<T>> : 1 Option<Vec<T>>
fn arb_idl_type() -> impl Strategy<Value = serde_json::Value> {
    arb_primitive_type().prop_flat_map(|prim| {
        let p_vec = prim.clone();
        let p_opt = prim.clone();
        let p_arr = prim.clone();
        let p_vec_opt = prim.clone(); // Vec<Option<T>>
        let p_opt_vec = prim.clone(); // Option<Vec<T>>
        prop_oneof![
            // Most fields are primitives; containers and nested types less frequent.
            4 => Just(prim),
            1 => Just(serde_json::json!({"vec": p_vec})),
            1 => Just(serde_json::json!({"option": p_opt})),
            1 => (1usize..=4).prop_map(move |n| serde_json::json!({"array": [p_arr.clone(), n]})),
            1 => Just(serde_json::json!({"vec": {"option": p_vec_opt}})),
            1 => Just(serde_json::json!({"option": {"vec": p_opt_vec}})),
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

/// Full IDL JSON string with 1–16 randomly-structured instructions (primitive + container types).
fn arb_idl_json() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_idl_instruction(), 1..=16).prop_map(|instructions| {
        serde_json::json!({
            "instructions": instructions,
            "types": [],
        })
        .to_string()
    })
}

/// IDL JSON string where one instruction references a randomly-generated defined struct.
///
/// This exercises the `Defined` type resolution path through `types`.
fn arb_defined_struct_idl_json() -> impl Strategy<Value = String> {
    (
        arb_identifier(), // struct name
        prop::collection::vec(
            (arb_identifier(), arb_primitive_type())
                .prop_map(|(n, t)| serde_json::json!({"name": n, "type": t})),
            1..=8, // struct fields (primitives only — avoids Defined-in-Defined depth limit)
        ),
        arb_identifier(), // instruction name
        // extra instructions that use primitive args, not the defined type
        prop::collection::vec(arb_idl_instruction(), 0..=4),
    )
        .prop_map(|(struct_name, fields, inst_name, mut extra_insts)| {
            let main_inst = serde_json::json!({
                "name": inst_name,
                "accounts": [],
                "args": [{"name": "data", "type": {"defined": struct_name}}]
            });
            extra_insts.push(main_inst);
            serde_json::json!({
                "instructions": extra_insts,
                "types": [{
                    "name": struct_name,
                    "type": {"kind": "struct", "fields": fields}
                }]
            })
            .to_string()
        })
}

/// IDL JSON string where every instruction has at least one `Vec` arg.
///
/// Used to stress-test the SizeGuard, which guards against large length-prefix
/// attacks (e.g. claiming a Vec of 10,000,000 u8 when the cursor has 4 bytes).
fn arb_vec_arg_idl_json() -> impl Strategy<Value = String> {
    (arb_identifier(), arb_idl_type()).prop_map(|(inst_name, elem_type)| {
        serde_json::json!({
            "instructions": [{
                "name": inst_name,
                "accounts": [],
                "args": [{"name": "data", "type": {"vec": elem_type}}]
            }],
            "types": []
        })
        .to_string()
    })
}

// ── Valid-input byte-generation strategies ────────────────────────────────────
//
// These strategies produce borsh-correct bytes for a given IdlType so that the
// parser can be asserted to return Ok — not merely "didn't panic".
//
// Size constraints keep every generated payload ≤ MAX_CURSOR_LENGTH (1232 bytes):
//   Vec: 0..=2 elements, String/Bytes: 0..=16 bytes of content.
// With 20 args per instruction (the max from arb_idl_instruction), worst-case:
//   Vec<Option<String<16>>> × 2 elements × 20 args + 8-byte disc = ~620 bytes.

/// Generate borsh-correct bytes for `ty`, resolving Defined types against `types`.
///
/// Returns a `BoxedStrategy` so the function can recurse for container types.
fn arb_bytes_for_type(ty: IdlType, types: Arc<Vec<IdlTypeDefinition>>) -> BoxedStrategy<Vec<u8>> {
    match ty {
        IdlType::Bool =>
            any::<bool>().prop_map(|b| vec![b as u8]).boxed(),
        IdlType::U8 =>
            any::<u8>().prop_map(|v| vec![v]).boxed(),
        IdlType::U16 =>
            any::<u16>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::U32 =>
            any::<u32>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::U64 =>
            any::<u64>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::U128 =>
            any::<u128>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::I8 =>
            any::<i8>().prop_map(|v| vec![v as u8]).boxed(),
        IdlType::I16 =>
            any::<i16>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::I32 =>
            any::<i32>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::I64 =>
            any::<i64>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::I128 =>
            any::<i128>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        // Use raw bit patterns to avoid NaN/inf — parser calls read_f32/f64 which accept any bits.
        IdlType::F32 =>
            any::<u32>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        IdlType::F64 =>
            any::<u64>().prop_map(|v| v.to_le_bytes().to_vec()).boxed(),
        // PublicKey: exactly 32 bytes, no length prefix.
        IdlType::PublicKey =>
            prop::collection::vec(any::<u8>(), 32).boxed(),
        // String: borsh u32-length-prefixed valid UTF-8.
        IdlType::String =>
            "[a-z0-9]{0,16}".prop_map(|s| {
                let b = s.as_bytes();
                let mut out = (b.len() as u32).to_le_bytes().to_vec();
                out.extend_from_slice(b);
                out
            }).boxed(),
        // Bytes: borsh u32-length-prefixed raw bytes.
        IdlType::Bytes =>
            prop::collection::vec(any::<u8>(), 0..=16).prop_map(|bytes| {
                let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
                out.extend(bytes);
                out
            }).boxed(),
        // Option: 1-byte tag (0=None, 1=Some) + inner bytes when Some.
        IdlType::Option(inner) => {
            let some_strat = arb_bytes_for_type(*inner, types);
            prop_oneof![
                1 => Just(vec![0u8]),
                1 => some_strat.prop_map(|b| { let mut out = vec![1u8]; out.extend(b); out }),
            ].boxed()
        }
        // Vec: u32 length prefix + N encoded elements (N ≤ 2 to bound total size).
        IdlType::Vec(inner) => {
            let inner_strat = arb_bytes_for_type(*inner, types);
            prop::collection::vec(inner_strat, 0..=2).prop_map(|items| {
                let mut out = (items.len() as u32).to_le_bytes().to_vec();
                for item in items { out.extend(item); }
                out
            }).boxed()
        }
        // Array: exactly N encoded elements, no length prefix.
        IdlType::Array(inner, n) => {
            let inner_strat = arb_bytes_for_type(*inner, types);
            prop::collection::vec(inner_strat, n..=n)
                .prop_map(|items| items.into_iter().flatten().collect())
                .boxed()
        }
        // Defined: look up the struct/enum/alias in `types` and encode accordingly.
        IdlType::Defined(defined) => {
            let name = defined.to_string();
            match types.iter().find(|t| t.name == name).map(|t| t.r#type.clone()) {
                Some(IdlTypeDefinitionType::Struct { fields }) => {
                    fields.into_iter()
                        .map(|f| arb_bytes_for_type(f.r#type, types.clone()))
                        .fold(Just(Vec::new()).boxed(), |acc, strat| {
                            (acc, strat)
                                .prop_map(|(mut a, b)| { a.extend(b); a })
                                .boxed()
                        })
                }
                Some(IdlTypeDefinitionType::Enum { variants }) => {
                    // borsh: 1-byte variant index + optional fields for that variant.
                    let n = variants.len();
                    if n == 0 {
                        return Just(vec![]).boxed();
                    }
                    let variants_owned = variants.clone();
                    let types_inner = types.clone();
                    (0..n)
                        .prop_flat_map(move |idx| {
                            let variant = variants_owned[idx].clone();
                            let types_v = types_inner.clone();
                            let idx_byte = idx as u8; // borsh enum index is u8
                            let fields_strat: BoxedStrategy<Vec<u8>> =
                                match variant.fields {
                                    None => Just(vec![]).boxed(),
                                    Some(EnumFields::Named(fields)) => fields
                                        .into_iter()
                                        .map(|f| arb_bytes_for_type(f.r#type, types_v.clone()))
                                        .fold(Just(vec![]).boxed(), |acc, s| {
                                            (acc, s)
                                                .prop_map(|(mut a, b)| { a.extend(b); a })
                                                .boxed()
                                        }),
                                    Some(EnumFields::Tuple(tys)) => tys
                                        .into_iter()
                                        .map(|t| arb_bytes_for_type(t, types_v.clone()))
                                        .fold(Just(vec![]).boxed(), |acc, s| {
                                            (acc, s)
                                                .prop_map(|(mut a, b)| { a.extend(b); a })
                                                .boxed()
                                        }),
                                };
                            fields_strat.prop_map(move |f| {
                                let mut out = vec![idx_byte];
                                out.extend(f);
                                out
                            })
                        })
                        .boxed()
                }
                Some(IdlTypeDefinitionType::Alias { value }) =>
                    arb_bytes_for_type(value, types),
                None =>
                    // Unknown defined type — fall back to empty bytes.
                    Just(vec![]).boxed(),
            }
        }
    }
}

/// Generate the discriminator + borsh-correct arg bytes for one instruction.
fn arb_valid_instruction_bytes(
    inst: &IdlInstruction,
    types: Arc<Vec<IdlTypeDefinition>>,
) -> BoxedStrategy<Vec<u8>> {
    let disc = match &inst.discriminator {
        Some(d) => d.clone(),
        None => return Just(vec![]).boxed(),
    };
    inst.args.iter()
        .map(|field| arb_bytes_for_type(field.r#type.clone(), types.clone()))
        .fold(Just(disc).boxed(), |acc, strat| {
            (acc, strat).prop_map(|(mut a, b)| { a.extend(b); a }).boxed()
        })
}

/// Strategy that produces `(idl_json, instruction_index, valid_borsh_bytes)`.
///
/// The bytes are always correctly encoded for the selected instruction's arg
/// layout — so `parse_instruction_with_idl` is expected to return `Ok`.
fn arb_idl_and_valid_bytes() -> impl Strategy<Value = (String, usize, Vec<u8>)> {
    arb_idl_json().prop_flat_map(|idl_json| {
        match decode_idl_data(&idl_json) {
            Err(_) => {
                // arb_idl_json generates well-formed IDLs; this branch is rare.
                Just((idl_json, 0usize, vec![])).boxed()
            }
            Ok(idl) => {
                let n = idl.instructions.len();
                let types = Arc::new(idl.types.clone());
                let instructions = idl.instructions.clone();
                let idl_json_owned = idl_json.clone();
                (0..n)
                    .prop_flat_map(move |inst_idx| {
                        let byte_strat = arb_valid_instruction_bytes(
                            &instructions[inst_idx],
                            types.clone(),
                        );
                        let j = idl_json_owned.clone();
                        byte_strat.prop_map(move |bytes| (j.clone(), inst_idx, bytes))
                    })
                    .boxed()
            }
        }
    })
}

// ── Crash-safety property tests ──────────────────────────────────────────────

proptest! {
    // Default is 256 cases. Override with PROPTEST_CASES=5000 for deeper fuzzing.
    #![proptest_config(ProptestConfig::default())]

    /// Core crash-safety test: a random IDL paired with instruction data that is
    /// either (a) fully random bytes or (b) a valid discriminator prefix followed
    /// by random arg bytes — 50/50 split.
    ///
    /// Using a valid discriminator for half of all inputs ensures the argument-
    /// decoding code paths are covered, not just the discriminator-matching paths.
    ///
    /// On the valid-discriminator branch: if parsing returns `Ok`, the instruction
    /// name must be non-empty — confirming that the parse code path was taken, not
    /// just that an `Err` was returned silently.
    #[test]
    fn fuzz_idl_parsing_never_panics(
        idl_json in arb_idl_json(),
        use_valid_disc in any::<bool>(),
        inst_idx in any::<usize>(),
        data in prop::collection::vec(any::<u8>(), 0..200usize),
    ) {
        if let Ok(idl) = decode_idl_data(&idl_json) {
            if use_valid_disc && !idl.instructions.is_empty() {
                let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                if let Some(disc) = &inst.discriminator {
                    let mut d = disc.clone();
                    d.extend_from_slice(&data);
                    if let Ok(result) = parse_instruction_with_idl(&d, TEST_PROGRAM_ID, &idl) {
                        prop_assert!(!result.instruction_name.is_empty(),
                            "Ok result must have a non-empty instruction name");
                    }
                    // Err is also acceptable — random arg bytes may be too short or malformed
                }
            } else {
                // Random bytes: only crash-safety matters, not the Ok/Err outcome
                let _ = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
            }
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
    ///
    /// On `Ok`: the instruction name must match the selected instruction, confirming
    /// that discriminator dispatch routed to the correct handler.
    #[test]
    fn fuzz_valid_discriminator_random_args(
        idl_json in arb_idl_json(),
        inst_idx in any::<usize>(),
        arg_bytes in prop::collection::vec(any::<u8>(), 0..1300usize),
    ) {
        if let Ok(idl) = decode_idl_data(&idl_json) {
            if !idl.instructions.is_empty() {
                let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                let expected_name = inst.name.clone();
                if let Some(disc) = &inst.discriminator {
                    let mut data = disc.clone();
                    data.extend_from_slice(&arg_bytes);
                    if let Ok(result) = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl) {
                        prop_assert_eq!(&result.instruction_name, &expected_name,
                            "discriminator must dispatch to the correct instruction");
                    }
                    // Err is acceptable — random arg bytes may be too short or malformed
                }
            }
        }
    }

    /// IDLs with defined struct types must not panic regardless of instruction bytes.
    /// Uses the same 50/50 valid-discriminator mix as the core test.
    ///
    /// On the valid-discriminator branch: if parsing returns `Ok`, the instruction
    /// name must match the selected instruction, confirming that defined-type
    /// resolution was attempted (not short-circuited before dispatch).
    #[test]
    fn fuzz_defined_struct_types_never_panics(
        idl_json in arb_defined_struct_idl_json(),
        use_valid_disc in any::<bool>(),
        inst_idx in any::<usize>(),
        data in prop::collection::vec(any::<u8>(), 0..200usize),
    ) {
        if let Ok(idl) = decode_idl_data(&idl_json) {
            if use_valid_disc && !idl.instructions.is_empty() {
                let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                let expected_name = inst.name.clone();
                if let Some(disc) = &inst.discriminator {
                    let mut d = disc.clone();
                    d.extend_from_slice(&data);
                    if let Ok(result) = parse_instruction_with_idl(&d, TEST_PROGRAM_ID, &idl) {
                        prop_assert_eq!(&result.instruction_name, &expected_name,
                            "defined-type instruction must dispatch to the correct handler");
                    }
                    // Err is acceptable — random arg bytes may not satisfy struct field layout
                }
            } else {
                let _ = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
            }
        }
    }

    /// Valid input must always parse successfully.
    ///
    /// Unlike the other crash-safety tests, this one asserts `result.is_ok()` —
    /// not merely "didn't panic". The bytes are generated by `arb_idl_and_valid_bytes`,
    /// which produces a correctly borsh-encoded payload for every instruction layout.
    ///
    /// A failure here indicates a genuine parser bug: the parser rejected data
    /// that it should have accepted according to its own IDL contract.
    ///
    /// On `Ok`: instruction name must match the selected instruction, confirming
    /// discriminator dispatch and arg decoding both succeeded.
    #[test]
    fn fuzz_valid_data_always_parses_ok(
        (idl_json, inst_idx, bytes) in arb_idl_and_valid_bytes(),
    ) {
        let Ok(idl) = decode_idl_data(&idl_json) else { return Ok(()); };
        if idl.instructions.is_empty() || bytes.is_empty() { return Ok(()); }
        let expected_name = idl.instructions[inst_idx].name.clone();
        let result = parse_instruction_with_idl(&bytes, TEST_PROGRAM_ID, &idl);
        prop_assert!(result.is_ok(),
            "parser rejected correctly-encoded input for instruction '{expected_name}': {:?}", result);
        prop_assert_eq!(&result.unwrap().instruction_name, &expected_name);
    }

    /// SizeGuard stress: a Vec arg instruction with a valid discriminator followed
    /// by an arbitrary u32 length prefix and a short trailing payload.
    ///
    /// The SizeGuard must prevent the parser from allocating memory proportional
    /// to the claimed length when the cursor contains far fewer bytes
    /// (budget = MAX_CURSOR_LENGTH × MAX_ALLOC_PER_CURSOR_LENGTH = 1232 × 24 = 29 568 bytes).
    #[test]
    fn fuzz_size_guard_vec_length_prefix(
        idl_json in arb_vec_arg_idl_json(),
        length_prefix in any::<u32>(),
        trailing in prop::collection::vec(any::<u8>(), 0..=8usize),
    ) {
        if let Ok(idl) = decode_idl_data(&idl_json) {
            if !idl.instructions.is_empty() {
                // There is exactly one instruction in arb_vec_arg_idl_json
                let inst = &idl.instructions[0];
                if let Some(disc) = &inst.discriminator {
                    let mut data = disc.clone();
                    data.extend_from_slice(&length_prefix.to_le_bytes());
                    data.extend_from_slice(&trailing);
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

// ── Defined type (struct) roundtrip tests ────────────────────────────────────

/// An instruction whose single arg is a defined struct with primitive fields
/// is decoded correctly end-to-end.
#[test]
fn roundtrip_defined_struct_arg() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "createOrder", "accounts": [], "args": [
            {"name": "params", "type": {"defined": "OrderParams"}}
        ]}],
        "types": [{
            "name": "OrderParams",
            "type": {"kind": "struct", "fields": [
                {"name": "price",    "type": "u64"},
                {"name": "quantity", "type": "u32"},
                {"name": "side",     "type": "bool"},
            ]}
        }]
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.extend_from_slice(&5000u64.to_le_bytes()); // price
    data.extend_from_slice(&10u32.to_le_bytes());   // quantity
    data.push(1u8);                                  // side = buy

    // Must parse and return Ok with the struct contents.
    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl).unwrap();
    assert_eq!(result.instruction_name, "createOrder");
    // Struct fields are nested under the "params" key.
    let params = &result.program_call_args["params"];
    assert_eq!(params["price"],    serde_json::json!(5000));
    assert_eq!(params["quantity"], serde_json::json!(10));
    assert_eq!(params["side"],     serde_json::json!(true));
}

// ── SizeGuard boundary tests ──────────────────────────────────────────────────

/// A Vec<u8> arg with a length prefix that vastly exceeds the backing data
/// must be rejected cleanly (Err), not panic or over-allocate.
///
/// SizeGuard budget = MAX_CURSOR_LENGTH (1232) × MAX_ALLOC_PER_CURSOR_LENGTH (24) = 29 568 bytes.
#[test]
fn size_guard_huge_vec_length_prefix_is_rejected_cleanly() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "writeData", "accounts": [], "args": [
            {"name": "payload", "type": {"vec": "u8"}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    // Claim 10 000 000 elements but provide zero backing bytes.
    let mut data = disc.clone();
    data.extend_from_slice(&10_000_000u32.to_le_bytes());

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
    // Must be Err, not a panic or OOM.
    assert!(result.is_err(), "expected Err for over-budget Vec length, got Ok");
}

/// Same as above but with a Vec<u64> (8 bytes/element) — smaller element count
/// is still enough to exceed the budget relative to cursor length.
#[test]
fn size_guard_vec_u64_over_budget() {
    let idl_json = serde_json::json!({
        "instructions": [{"name": "setRates", "accounts": [], "args": [
            {"name": "rates", "type": {"vec": "u64"}}
        ]}],
        "types": []
    })
    .to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    // 100 000 × 8 bytes = 800 000 bytes, far exceeds the 29 568-byte budget.
    let mut data = disc.clone();
    data.extend_from_slice(&100_000u32.to_le_bytes());

    let result = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
    assert!(result.is_err(), "expected Err for over-budget Vec<u64> length");
}

// ── Real-IDL property tests (driven by IDL_FILE env var) ─────────────────────
//
// These tests are skipped when IDL_FILE is unset, so CI passes without it.
//
// Usage:
//   IDL_FILE=/path/to/jupiter.json cargo test --test fuzz_idl_parsing real_idl
//   IDL_FILE=/path/to/drift.json PROPTEST_CASES=1000 cargo test --test fuzz_idl_parsing real_idl
//
// See scripts/fuzz_all_idls.sh to run against all embedded IDLs in one pass.

fn load_idl_from_env() -> Option<(String, solana_parser::solana::structs::Idl)> {
    let path = std::env::var("IDL_FILE").ok()?;
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("IDL_FILE={path}: {e}"));
    match decode_idl_data(&json) {
        Ok(idl) => Some((json, idl)),
        Err(e) => {
            // IDL failed validation (e.g. duplicate type names, cyclic references).
            // Skip these tests — they are not valid inputs for real_idl_* tests.
            eprintln!("IDL_FILE={path}: skipping — decode failed: {e}");
            None
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    /// Crash-safety test against a real IDL loaded from IDL_FILE.
    ///
    /// Uses the same 50/50 valid/random discriminator mix as
    /// `fuzz_idl_parsing_never_panics`. On `Ok` with a valid discriminator,
    /// asserts the instruction name matches the selected instruction.
    #[test]
    fn real_idl_never_panics(
        use_valid_disc in any::<bool>(),
        inst_idx in any::<usize>(),
        data in prop::collection::vec(any::<u8>(), 0..1300usize),
    ) {
        let Some((_, idl)) = load_idl_from_env() else { return Ok(()); };
        if use_valid_disc && !idl.instructions.is_empty() {
            let inst = &idl.instructions[inst_idx % idl.instructions.len()];
            let expected_name = inst.name.clone();
            if let Some(disc) = &inst.discriminator {
                let mut d = disc.clone();
                d.extend_from_slice(&data);
                if let Ok(result) = parse_instruction_with_idl(&d, TEST_PROGRAM_ID, &idl) {
                    prop_assert_eq!(&result.instruction_name, &expected_name,
                        "discriminator must dispatch to the correct instruction");
                }
            }
        } else {
            let _ = parse_instruction_with_idl(&data, TEST_PROGRAM_ID, &idl);
        }
    }
}

/// Valid-data parse test against a real IDL loaded from IDL_FILE.
///
/// Uses TestRunner::run directly so the strategy can be built from the
/// runtime-loaded IDL (not possible with the proptest! macro, which requires
/// strategies to be fully determined at compile time).
///
/// For every instruction in the IDL, generates correctly borsh-encoded bytes
/// (discriminator + all args) and asserts the parser returns Ok with the
/// expected instruction name.
#[test]
fn real_idl_valid_data_always_parses_ok() {
    let Some((_, idl)) = load_idl_from_env() else { return; };
    let n = idl.instructions.len();
    if n == 0 { return; }

    let types = Arc::new(idl.types.clone());
    let instructions = idl.instructions.clone();

    let strategy = (0..n).prop_flat_map(move |inst_idx| {
        arb_valid_instruction_bytes(&instructions[inst_idx], types.clone())
            .prop_map(move |bytes| (inst_idx, bytes))
    });

    let config = ProptestConfig::default();
    let mut runner = proptest::test_runner::TestRunner::new(config);
    let idl_ref = idl.clone();
    runner
        .run(&strategy, move |(inst_idx, bytes)| {
            let expected = &idl_ref.instructions[inst_idx].name;
            let result = parse_instruction_with_idl(&bytes, TEST_PROGRAM_ID, &idl_ref);
            prop_assert!(
                result.is_ok(),
                "instruction '{expected}' rejected correctly-encoded input: {:?}", result
            );
            prop_assert_eq!(&result.unwrap().instruction_name, expected);
            Ok(())
        })
        .expect("real_idl_valid_data_always_parses_ok failed");
}
