//! Compiled-in preset IDLs, decoded and hashed once per process.
//!
//! Presets render from IDLs embedded with `include_str!`. Decoding one, and
//! hashing it for `SolanaParsedInstructionData::idl_hash`, is the same work on
//! every request, so both are done once here and shared by reference.

use solana_parser::{Idl, SolanaParsedInstructionData};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, PoisonError};

/// Decodes a compiled-in IDL once per process.
///
/// Keyed by the JSON's address and length: a `&'static str` is never freed,
/// so the key can't be reused for different JSON. The decoded IDL is leaked to
/// get a `'static` reference; there is one per compiled-in IDL.
pub(crate) fn compiled_in_idl(json: &'static str) -> Result<&'static Idl, String> {
    type Decoded = &'static Result<Idl, String>;
    static DECODED: OnceLock<Mutex<BTreeMap<(usize, usize), Decoded>>> = OnceLock::new();

    let key = (json.as_ptr() as usize, json.len());
    let mut decoded = DECODED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let entry = *decoded.entry(key).or_insert_with(|| {
        Box::leak(Box::new(
            solana_parser::decode_idl_data(json).map_err(|e| e.to_string()),
        ))
    });
    entry.as_ref().map_err(Clone::clone)
}

/// [`solana_parser::parse_instruction_with_idl`] for an IDL that lives for the
/// whole process, with its `idl_hash` computed once rather than on every call.
///
/// Keyed by the IDL's address: a `&'static Idl` is never freed, so the key
/// always refers to the same IDL. The reported hash is
/// [`solana_parser::idl_hash_of`], exactly what `parse_instruction_with_idl`
/// computes per call.
pub(crate) fn parse_preset_instruction(
    instruction_data: &[u8],
    program_id: &str,
    idl: &'static Idl,
) -> Result<SolanaParsedInstructionData, Box<dyn std::error::Error>> {
    static HASHES: OnceLock<Mutex<BTreeMap<usize, Result<String, String>>>> = OnceLock::new();

    let key = std::ptr::from_ref(idl) as usize;
    let hash = HASHES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key)
        .or_insert_with(|| solana_parser::idl_hash_of(idl).map_err(|e| e.to_string()))
        .clone();
    match hash {
        Ok(hash) => {
            solana_parser::parse_instruction_with_idl_hash(instruction_data, program_id, idl, &hash)
        }
        // Hashing can only fail by failing to serialize the IDL; let the
        // uncached path report that exactly as it always has.
        Err(_) => solana_parser::parse_instruction_with_idl(instruction_data, program_id, idl),
    }
}
