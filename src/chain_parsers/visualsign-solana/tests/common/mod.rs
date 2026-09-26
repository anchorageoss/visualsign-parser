#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Shared test helpers for IDL-based fuzz and integration tests.
#![allow(dead_code)]

use std::collections::BTreeMap;

use base64::Engine;
use generated::parser::{ChainMetadata, Idl as ProtoIdl, SolanaMetadata, chain_metadata};
use solana_parser::decode_idl_data;
use solana_parser::solana::structs::Idl;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};
use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldPreviewLayout,
};
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

/// Why `build_disc_data` (below) failed to produce discriminator-prefixed
/// instruction data for a given IDL.
pub(crate) enum DiscDataError {
    DecodeRejected(String),
    NoInstructions,
    NoDiscriminator(usize),
}

impl std::fmt::Display for DiscDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DecodeRejected(e) => write!(f, "decode_idl_data rejected the IDL: {e}"),
            Self::NoInstructions => write!(f, "IDL has no instructions"),
            Self::NoDiscriminator(idx) => write!(f, "instructions[{idx}] has no discriminator"),
        }
    }
}

/// Decode an IDL JSON string, extract the discriminator for the instruction at
/// `inst_idx`, and return `(idl, data)` where `data` = discriminator ++ `arg_bytes`.
pub(crate) fn try_build_disc_data(
    idl_json: &str,
    inst_idx: usize,
    arg_bytes: &[u8],
) -> Result<(Idl, Vec<u8>), DiscDataError> {
    let idl =
        decode_idl_data(idl_json).map_err(|e| DiscDataError::DecodeRejected(e.to_string()))?;
    if idl.instructions.is_empty() {
        return Err(DiscDataError::NoInstructions);
    }
    let sel_idx = inst_idx % idl.instructions.len();
    let disc = idl.instructions[sel_idx]
        .discriminator
        .as_ref()
        .ok_or(DiscDataError::NoDiscriminator(sel_idx))?;
    let mut data = disc.clone();
    data.extend_from_slice(arg_bytes);
    Ok((idl, data))
}

/// Decode an IDL JSON string, extract the discriminator for the instruction at
/// `inst_idx`, and return `(idl, data)` where `data` = discriminator ++ `arg_bytes`.
///
/// Returns `None` if decoding fails, the IDL has no instructions, or the
/// selected instruction has no discriminator. Use `try_build_disc_data` (crate-
/// internal) when the caller wants to report which failure mode occurred.
pub fn build_disc_data(
    idl_json: &str,
    inst_idx: usize,
    arg_bytes: &[u8],
) -> Option<(Idl, Vec<u8>)> {
    try_build_disc_data(idl_json, inst_idx, arg_bytes).ok()
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
    if use_valid_disc && let Some((_idl, disc_data)) = build_disc_data(idl_json, inst_idx, &data) {
        return disc_data;
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
    let mut idl_mappings = BTreeMap::new();
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
        include_intermediate_output: false,
        metadata: Some(ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                // Boundary conversion: generated proto type uses HashMap; we keep
                // BTreeMap locally per crate-wide determinism rule.
                idl_mappings: idl_mappings.into_iter().collect(),
                network_id: None,
                idl: None,
                simulated_transaction_result: None,
            })),
        }),
        ..VisualSignOptions::default()
    }
}

pub fn options_no_idl() -> VisualSignOptions {
    VisualSignOptions::default()
}

// ── Field inspection helpers ──────────────────────────────────────────────────

/// Returns the PreviewLayout for every instruction field in the payload.
/// Instruction fields are PreviewLayouts that are not "Network", "Accounts",
/// "Address Lookup Tables", or diagnostic fields.
pub fn instruction_fields(payload: &SignablePayload) -> Vec<&SignablePayloadFieldPreviewLayout> {
    let non_instruction_labels = ["Network", "Accounts", "Address Lookup Tables"];
    payload
        .fields
        .iter()
        .filter_map(|f| {
            if let SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } = f
                && !non_instruction_labels.contains(&common.label.as_str())
            {
                return Some(preview_layout);
            }
            None
        })
        .collect()
}

/// Searches a flat slice of AnnotatedPayloadFields for a TextV2 field with the given label.
pub fn find_text(fields: &[AnnotatedPayloadField], label: &str) -> Option<String> {
    fields.iter().find_map(|f| {
        if let SignablePayloadField::TextV2 { common, text_v2 } = &f.signable_payload_field
            && common.label == label
        {
            return Some(text_v2.text.clone());
        }
        None
    })
}

// ── IDL loading helpers ───────────────────────────────────────────────────────

/// Load a real IDL from the path in the `IDL_FILE` environment variable.
///
/// Returns `None` when `IDL_FILE` is unset or the IDL fails validation,
/// allowing `real_idl_*` tests to be silently skipped in CI.
pub fn load_idl_from_env() -> Option<(String, solana_parser::solana::structs::Idl)> {
    let path = std::env::var("IDL_FILE").ok()?;
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("IDL_FILE={path}: {e}"));
    match decode_idl_data(&json) {
        Ok(idl) => Some((json, idl)),
        Err(e) => {
            eprintln!("IDL_FILE={path}: skipping — decode failed: {e}");
            None
        }
    }
}

// ── Surfpool roundtrip ────────────────────────────────────────────────────────

/// Per-IDL roundtrip: decode the IDL, build a synthetic transaction whose data
/// starts with the first instruction's discriminator, run it through the
/// visual-sign converter, and assert the payload is non-empty.
///
/// Network-bound: starts a `surfpool` mainnet fork and requires the `surfpool`
/// binary on `$PATH`. Callers are responsible for marking their tests with
/// `#[ignore]`. Use the `idl_test!` macro for the standard wrapper.
///
/// To loop over many IDLs without paying the surfpool startup cost per IDL,
/// start one `SurfpoolManager` yourself and call `run_idl_roundtrip_inner`
/// in the loop.
pub async fn run_idl_roundtrip(idl_label: &str, idl_json: &str) {
    let _manager = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("surfpool should start");
    run_idl_roundtrip_inner(idl_label, idl_json);
}

/// Body of `run_idl_roundtrip` minus the `SurfpoolManager` start. Use when
/// running many IDLs in sequence under a single shared manager.
pub fn run_idl_roundtrip_inner(idl_label: &str, idl_json: &str) {
    // A red test names the IDL and the actual cause (decode rejection from a
    // malformed IDL, empty instruction list, or a missing discriminator).
    let (_idl, data) =
        try_build_disc_data(idl_json, 0, &[0u8; 32]).unwrap_or_else(|e| panic!("{idl_label}: {e}"));

    let program_id = Pubkey::new_unique();
    let tx = build_transaction(program_id, vec![Pubkey::new_unique()], data);
    let tx_bytes = bincode::serialize(&tx).expect("tx should serialize");
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    let wrapper = SolanaTransactionWrapper::from_string(&tx_b64)
        .expect("from_string should succeed for a valid base64 transaction");

    let options = options_with_idl(&program_id, idl_json, "test_program");
    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(wrapper, options)
        .expect("converter should succeed")
        .payload;

    assert!(
        !payload.fields.is_empty(),
        "payload must contain at least one field"
    );
}

/// Generate a `#[tokio::test] #[ignore]` that runs `run_idl_roundtrip` against
/// the provided IDL string. Works for both upstream `embedded_idls` consts and
/// vsp-local IDL JSON via `include_str!`.
///
/// Any sibling test file can call this macro unqualified after `mod common;` —
/// `#[macro_export]` puts it at the test binary's crate root, so neither
/// `#[macro_use]` nor an explicit `use` is required.
#[macro_export]
macro_rules! idl_test {
    ($name:ident, $idl:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        #[ignore]
        async fn $name() {
            $crate::common::run_idl_roundtrip(stringify!($name), $idl).await;
        }
    };
}
