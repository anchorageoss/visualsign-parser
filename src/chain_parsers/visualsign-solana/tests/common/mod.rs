//! Shared test helpers for IDL-based fuzz and integration tests.
#![allow(dead_code)]

use std::collections::HashMap;

use generated::parser::{ChainMetadata, Idl as ProtoIdl, SolanaMetadata, chain_metadata};
use solana_parser::decode_idl_data;
use solana_parser::solana::structs::Idl;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField,
    SignablePayloadFieldPreviewLayout,
};

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

// ── Transaction builders ──────────────────────────────────────────────────────

pub fn build_transaction(
    program_id: Pubkey,
    extra_accounts: Vec<Pubkey>,
    data: Vec<u8>,
) -> SolanaTransaction {
    let fee_payer = Pubkey::new_unique();
    let account_metas: Vec<AccountMeta> = extra_accounts
        .iter()
        .map(|pk| AccountMeta::new_readonly(*pk, false))
        .collect();
    let ix = Instruction::new_with_bytes(program_id, &data, account_metas);
    SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&fee_payer)))
}

pub fn build_multi_instruction_transaction(pairs: Vec<(Pubkey, Vec<u8>)>) -> SolanaTransaction {
    let fee_payer = Pubkey::new_unique();
    let ixs: Vec<Instruction> = pairs
        .into_iter()
        .map(|(pid, data)| Instruction::new_with_bytes(pid, &data, vec![]))
        .collect();
    SolanaTransaction::new_unsigned(Message::new(&ixs, Some(&fee_payer)))
}

// ── VisualSignOptions builders ────────────────────────────────────────────────

pub fn options_with_idl(program_id: &Pubkey, idl_json: &str, name: &str) -> VisualSignOptions {
    let mut idl_mappings = HashMap::new();
    idl_mappings.insert(
        program_id.to_string(),
        ProtoIdl {
            value: idl_json.to_string(),
            program_name: Some(name.to_string()),
            idl_type: None,
            idl_version: None,
            signature: None,
        },
    );
    VisualSignOptions {
        metadata: Some(ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                idl_mappings,
                network_id: None,
                idl: None,
            })),
        }),
        decode_transfers: false,
        transaction_name: None,
        developer_config: None,
        abi_registry: None,
    }
}

pub fn options_no_idl() -> VisualSignOptions {
    VisualSignOptions {
        metadata: None,
        decode_transfers: false,
        transaction_name: None,
        developer_config: None,
        abi_registry: None,
    }
}

// ── Field inspection helpers ──────────────────────────────────────────────────

/// Returns the PreviewLayout for every instruction field in the payload.
pub fn instruction_fields(payload: &SignablePayload) -> Vec<&SignablePayloadFieldPreviewLayout> {
    payload
        .fields
        .iter()
        .filter_map(|f| {
            if let SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } = f
            {
                if common.label.starts_with("Instruction") {
                    return Some(preview_layout);
                }
            }
            None
        })
        .collect()
}

/// Searches a flat slice of AnnotatedPayloadFields for a TextV2 field with the given label.
pub fn find_text(fields: &[AnnotatedPayloadField], label: &str) -> Option<String> {
    fields.iter().find_map(|f| {
        if let SignablePayloadField::TextV2 { common, text_v2 } = &f.signable_payload_field {
            if common.label == label {
                return Some(text_v2.text.clone());
            }
        }
        None
    })
}
