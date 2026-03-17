//! PropTest strategies that generate borsh-correct byte sequences for Anchor IDL
//! instruction layouts.
//!
//! Extracted from `visualsign-solana/tests/fuzz_idl_parsing.rs` to serve as the
//! single source of truth used by both the existing fuzz tests and the new
//! surfpool integration tests.
//!
//! Size constraints keep every generated payload within MAX_CURSOR_LENGTH (1232 bytes):
//!   Vec: 0–2 elements, String/Bytes: 0–16 bytes of content.

use proptest::prelude::*;
use solana_parser::solana::structs::{
    EnumFields, IdlInstruction, IdlType, IdlTypeDefinition, IdlTypeDefinitionType,
};
use std::sync::Arc;

/// Generate borsh-correct bytes for `ty`, resolving `Defined` types against `types`.
///
/// Returns a `BoxedStrategy` so the function can recurse for container types.
pub fn arb_bytes_for_type(ty: IdlType, types: Arc<Vec<IdlTypeDefinition>>) -> BoxedStrategy<Vec<u8>> {
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
                            let idx_byte = idx as u8;
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
///
/// Returns an empty-byte strategy if the instruction has no discriminator
/// (should not happen for well-formed Anchor IDLs).
pub fn arb_valid_instruction_bytes(
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
