//! Shared test helpers for IDL-based fuzz and integration tests.

use solana_parser::decode_idl_data;
use solana_parser::solana::structs::Idl;

/// Decode an IDL JSON string, extract the discriminator for the instruction at
/// `inst_idx`, and return `(idl, data)` where `data` = discriminator ++ `arg_bytes`.
///
/// Returns `None` if decoding fails, the IDL has no instructions, or the
/// selected instruction has no discriminator.
pub fn build_disc_data(
    idl_json: &str,
    inst_idx: usize,
    arg_bytes: &[u8],
) -> Option<(Idl, Vec<u8>)> {
    let idl = decode_idl_data(idl_json).ok()?;
    if idl.instructions.is_empty() {
        return None;
    }
    let inst = &idl.instructions[inst_idx % idl.instructions.len()];
    let disc = inst.discriminator.as_ref()?;
    let mut data = disc.clone();
    data.extend_from_slice(arg_bytes);
    Some((idl, data))
}

/// Build instruction bytes using a 50/50 valid-discriminator / random-data split.
///
/// When `use_valid_disc` is true, attempts to prepend a real discriminator from
/// the IDL instruction at `inst_idx`. Falls back to raw `data` if decoding
/// fails, the IDL has no instructions, or the instruction has no discriminator.
pub fn build_maybe_disc_bytes(
    idl_json: &str,
    use_valid_disc: bool,
    inst_idx: usize,
    data: Vec<u8>,
) -> Vec<u8> {
    if use_valid_disc {
        if let Some((_idl, disc_data)) = build_disc_data(idl_json, inst_idx, &data) {
            return disc_data;
        }
    }
    data
}
