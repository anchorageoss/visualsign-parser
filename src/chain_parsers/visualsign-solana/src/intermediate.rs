//! Solana intermediate output for downstream policy engines.
//!
//! This is a Borsh-serialized mirror of [`solana_parser::SolanaMetadata`]
//! shaped to match the per-instruction attributes that a downstream policy
//! engine evaluates against (account keys, program keys, transfers, and the
//! decoded instruction args). The schema is deliberately kept stable in Rust
//! so the parser and the consumer share one definition; the bytes emitted here
//! are placed verbatim into `ParsedTransactionPayload.intermediate_output`.
//!
//! Consumers (e.g. the Anchorage HSM) mirror these types and decode the bytes;
//! [`SOLANA_INTERMEDIATE_SCHEMA_VERSION`] is the first field so a shape change
//! is a single, reviewable signal that forces the mirrored decoder to update.
//!
//! Differences from `solana_parser::SolanaMetadata`:
//! - `signatures` is dropped (unsigned txs have none).
//! - All maps use `BTreeMap` so Borsh encoding is byte-deterministic.
//! - `program_call_args` is emitted as a canonical JSON string
//!   (`program_call_args_json`) because `serde_json::Value` does not implement
//!   `BorshSerialize`. Keys are alphabetized at *every* nesting level: the
//!   `serde_json::Value` tree is walked and re-keyed into sorted order, so the
//!   output is independent of `serde_json`'s `preserve_order` build feature
//!   (which is enabled transitively elsewhere in the workspace and would
//!   otherwise serialize nested objects in insertion order).

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde_json::Value;
use solana_parser::solana::idl_parser::{
    compute_idl_hash, construct_idl_records_map, create_accounts_map,
    find_instruction_by_discriminator, parse_data_into_args, resolve_idl_for_record,
};
use solana_parser::solana::structs::{
    self as parser, AccountAddress, IdlSource, SolanaMetadata, SolanaParsedInstructionData,
};
use solana_parser::{CustomIdlConfig, parse_transaction_with_idls};
use visualsign::errors::VisualSignError;
use visualsign::vsptrait::TransactionParseError;

use crate::idl::IdlRegistry;

/// Version of the `SolanaIntermediateOutput` Borsh schema. Bump on any change
/// to the shape below that ships to a consumer that already understands a
/// prior version -- mirrored decoders assert this value, so a bump makes a
/// schema drift fail loudly instead of silently misparsing. Emission is gated
/// behind an opt-in flag with no live consumers yet, so `simulated_instructions`
/// was added under this same version: every decoder is updated to the new
/// shape before the flag is ever enabled.
pub const SOLANA_INTERMEDIATE_SCHEMA_VERSION: u16 = 2;

/// Top-level Solana intermediate output. Mirrors `solana_parser::SolanaMetadata`
/// minus `signatures`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaIntermediateOutput {
    /// Always [`SOLANA_INTERMEDIATE_SCHEMA_VERSION`]. First field so decoders
    /// can gate on it before reading the rest.
    pub schema_version: u16,
    pub account_keys: Vec<String>,
    pub program_keys: Vec<String>,
    pub instructions: Vec<SolanaIntermediateInstruction>,
    pub transfers: Vec<SolTransfer>,
    pub spl_transfers: Vec<SplTransfer>,
    pub recent_blockhash: String,
    pub address_table_lookups: Vec<SolanaAddressTableLookup>,
    /// Every call (top-level and inner/CPI, flattened) a caller-supplied
    /// transaction simulation observed. Always empty when no simulation was
    /// supplied. Separate from `instructions` (static decode): this is the
    /// only place `is_unregistered` is computed.
    pub simulated_instructions: Vec<SolanaSimulatedInstruction>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaIntermediateInstruction {
    pub program_key: String,
    pub accounts: Vec<SolanaAccount>,
    pub instruction_data_hex: String,
    pub address_table_lookups: Vec<SolanaSingleAddressTableLookup>,
    /// `None` when the parser could not match an IDL for this instruction.
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
}

/// One inner/CPI call a transaction simulation observed.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaSimulatedInstruction {
    /// Position of the top-level instruction that triggered this call
    /// (0-based), matching static decode's instruction indexing.
    pub instruction_index: u32,
    /// Call depth: 2+ for inner/CPI calls, matching Solana simulation's own
    /// stackHeight semantics.
    pub stack_height: u32,
    pub program_key: String,
    pub accounts: Vec<SolanaAccount>,
    pub instruction_data_hex: String,
    /// True when `program_key` is not in `idl::builtin_programs::is_trusted_program`'s
    /// set (native/SPL programs, `solana_parser::ProgramType` built-ins, and every
    /// in-crate preset visualizer's program IDs).
    pub is_unregistered: bool,
    /// `None` when the parser could not match an IDL for this instruction.
    /// Always `None` when `rpc_parsed_data` is `Some`: a jsonParsed instruction
    /// carries no raw instruction_data_hex/accounts for the parser to IDL-decode.
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
    /// The RPC's own jsonParsed decode, when the caller's simulateTransaction
    /// result returned this instruction jsonParsed instead of raw (recognized
    /// programs, e.g. System/Token). `None` for raw/compiled instructions.
    pub rpc_parsed_data: Option<SolanaRpcParsedInstructionDataIo>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaAccount {
    pub account_key: String,
    pub signer: bool,
    pub writable: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolTransfer {
    pub from: String,
    pub to: String,
    pub amount: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SplTransfer {
    pub from: String,
    pub to: String,
    pub amount: String,
    pub owner: String,
    pub signers: Vec<String>,
    pub token_mint: Option<String>,
    pub decimals: Option<String>,
    pub fee: Option<String>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaSingleAddressTableLookup {
    pub address_table_key: String,
    pub index: i32,
    pub writable: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaAddressTableLookup {
    pub address_table_key: String,
    pub writable_indexes: Vec<i32>,
    pub readonly_indexes: Vec<i32>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaParsedInstructionDataIo {
    pub instruction_name: String,
    pub discriminator: String,
    pub named_accounts: BTreeMap<String, String>,
    /// Canonical JSON string with alphabetized keys at every nesting level.
    /// Built by recursively re-keying the `serde_json::Value` tree into sorted
    /// order, so byte-identical inputs produce byte-identical encodings
    /// regardless of `serde_json`'s `preserve_order` build feature.
    pub program_call_args_json: String,
    /// `"BuiltIn"` (with the inner program-type discriminant collapsed) or
    /// `"Custom"`. Empty when no IDL was used.
    pub idl_source: String,
    pub idl_hash: String,
}

/// The RPC's own jsonParsed decode of a simulated instruction, as returned for
/// recognized programs. Distinct from [`SolanaParsedInstructionDataIo`], which
/// is this parser's own IDL-decoded output.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaRpcParsedInstructionDataIo {
    pub instruction_type: String,
    pub info_json: String,
}

// -- From impls --------------------------------------------------------------

impl From<&parser::SolanaAccount> for SolanaAccount {
    fn from(value: &parser::SolanaAccount) -> Self {
        Self {
            account_key: value.account_key.clone(),
            signer: value.signer,
            writable: value.writable,
        }
    }
}

impl From<&parser::SolTransfer> for SolTransfer {
    fn from(value: &parser::SolTransfer) -> Self {
        Self {
            from: value.from.clone(),
            to: value.to.clone(),
            amount: value.amount.clone(),
        }
    }
}

impl From<&parser::SplTransfer> for SplTransfer {
    fn from(value: &parser::SplTransfer) -> Self {
        Self {
            from: value.from.clone(),
            to: value.to.clone(),
            amount: value.amount.clone(),
            owner: value.owner.clone(),
            signers: value.signers.clone(),
            token_mint: value.token_mint.clone(),
            decimals: value.decimals.clone(),
            fee: value.fee.clone(),
        }
    }
}

impl From<&parser::SolanaSingleAddressTableLookup> for SolanaSingleAddressTableLookup {
    fn from(value: &parser::SolanaSingleAddressTableLookup) -> Self {
        Self {
            address_table_key: value.address_table_key.clone(),
            index: value.index,
            writable: value.writable,
        }
    }
}

impl From<&parser::SolanaAddressTableLookup> for SolanaAddressTableLookup {
    fn from(value: &parser::SolanaAddressTableLookup) -> Self {
        Self {
            address_table_key: value.address_table_key.clone(),
            writable_indexes: value.writable_indexes.clone(),
            readonly_indexes: value.readonly_indexes.clone(),
        }
    }
}

fn idl_source_string(source: &IdlSource) -> String {
    match source {
        IdlSource::BuiltIn(_) => "BuiltIn".to_string(),
        IdlSource::Custom => "Custom".to_string(),
    }
}

fn canonical_args_json(args: &serde_json::Map<String, Value>) -> String {
    // Canonicalize recursively so *every* nesting level is alphabetized, not
    // just the top-level map. serde_json::to_string never fails on a
    // Map<String, Value>; on the off-chance it does we fall back to an empty
    // object so the surrounding borsh encoding stays well-formed.
    let canonical = Value::Object(canonicalize_map(args));
    serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string())
}

/// Recursively re-key every nested JSON object so its keys appear in sorted
/// order, independent of `serde_json`'s `preserve_order` build feature.
///
/// `serde_json` is built with `preserve_order` elsewhere in the workspace
/// (pulled in transitively by the Sui chain parser and the integration crate
/// via `indexmap`), which makes `serde_json::Map` an `IndexMap` that serializes
/// keys in *insertion* order. Without this walk, only the top-level map would
/// be alphabetized (by the `BTreeMap` re-key in [`canonicalize_map`]) and
/// nested objects would leak insertion order into the canonical string —
/// silently breaking byte-determinism for consumers that mirror this schema.
/// Arrays are traversed element-wise; scalars are returned unchanged.
fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(canonicalize_map(map)),
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}

/// Build a new `serde_json::Map` whose entries are those of `map`, inserted in
/// sorted key order and with values recursively canonicalized.
///
/// Inserting in sorted order makes the serialized output alphabetized
/// regardless of whether `serde_json::Map` is backed by `BTreeMap` (default) or
/// `IndexMap` (`preserve_order`): a `BTreeMap` stays sorted; an `IndexMap`
/// preserves the (sorted) insertion order we feed it.
fn canonicalize_map(map: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    let sorted: BTreeMap<&String, &Value> = map.iter().collect();
    let mut out = serde_json::Map::with_capacity(map.len());
    for (k, v) in sorted {
        out.insert(k.clone(), canonicalize_value(v));
    }
    out
}

impl From<&SolanaParsedInstructionData> for SolanaParsedInstructionDataIo {
    fn from(value: &SolanaParsedInstructionData) -> Self {
        let named_accounts: BTreeMap<String, String> = value
            .named_accounts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Self {
            instruction_name: value.instruction_name.clone(),
            discriminator: value.discriminator.clone(),
            named_accounts,
            program_call_args_json: canonical_args_json(&value.program_call_args),
            idl_source: idl_source_string(&value.idl_source),
            idl_hash: value.idl_hash.clone(),
        }
    }
}

impl From<&parser::SolanaInstruction> for SolanaIntermediateInstruction {
    fn from(value: &parser::SolanaInstruction) -> Self {
        Self {
            program_key: value.program_key.clone(),
            accounts: value.accounts.iter().map(SolanaAccount::from).collect(),
            instruction_data_hex: value.instruction_data_hex.clone(),
            address_table_lookups: value
                .address_table_lookups
                .iter()
                .map(SolanaSingleAddressTableLookup::from)
                .collect(),
            parsed_instruction_data: value
                .parsed_instruction
                .as_ref()
                .map(SolanaParsedInstructionDataIo::from),
        }
    }
}

/// Builds a [`SolanaSimulatedInstruction`] from one caller-supplied inner/CPI
/// call, IDL-decoding it the same way static decode does. `instruction_index`
/// comes from the enclosing `InnerInstructionGroup`, not the instruction
/// itself. `is_unregistered` is computed the same way as the static-decode
/// path. IDL resolution/decoding is best-effort: any failure (no IDL, bad
/// discriminator, account/arg mismatch) degrades to `parsed_instruction_data:
/// None` rather than failing the whole conversion, matching static decode's
/// `SolanaIntermediateInstruction.parsed_instruction_data` posture.
pub(crate) fn build_simulated_instruction(
    value: &generated::parser::SimulatedInstruction,
    instruction_index: u32,
    idl_registry: &IdlRegistry,
) -> SolanaSimulatedInstruction {
    let accounts: Vec<SolanaAccount> = value
        .accounts
        .iter()
        .map(|a| SolanaAccount {
            account_key: a.account_key.clone(),
            signer: a.is_signer,
            writable: a.is_writable,
        })
        .collect();

    // A jsonParsed instruction carries no raw instruction_data_hex/accounts,
    // so there's nothing for parse_simulated_instruction_idl to decode; use
    // the RPC's own parsed data instead.
    let (parsed_instruction_data, rpc_parsed_data) = match &value.rpc_parsed_data {
        Some(rpc_parsed) => (
            None,
            Some(SolanaRpcParsedInstructionDataIo {
                instruction_type: rpc_parsed.instruction_type.clone(),
                info_json: rpc_parsed.info_json.clone(),
            }),
        ),
        None => (
            parse_simulated_instruction_idl(
                &value.program_key,
                &accounts,
                &value.instruction_data_hex,
                idl_registry,
            ),
            None,
        ),
    };

    SolanaSimulatedInstruction {
        instruction_index,
        stack_height: value.stack_height,
        program_key: value.program_key.clone(),
        accounts: accounts.clone(),
        instruction_data_hex: value.instruction_data_hex.clone(),
        is_unregistered: !crate::idl::builtin_programs::is_trusted_program(&value.program_key),
        parsed_instruction_data,
        rpc_parsed_data,
    }
}

/// IDL-decodes one simulated instruction using the same public
/// `solana_parser` building blocks the static-decode path's private
/// `parse_idl` uses internally, since simulated/inner instructions never
/// appear in the raw transaction message and so can't go through
/// `parse_transaction_with_idls`. Returns `None` on any resolution/decode
/// failure (no IDL for this program, unmatched discriminator, malformed
/// data/accounts) -- best-effort, never surfaced as an error.
fn parse_simulated_instruction_idl(
    program_key: &str,
    accounts: &[SolanaAccount],
    instruction_data_hex: &str,
    idl_registry: &IdlRegistry,
) -> Option<SolanaParsedInstructionDataIo> {
    let data = hex::decode(instruction_data_hex).ok()?;
    let account_addresses: Vec<AccountAddress> = accounts
        .iter()
        .map(|a| {
            AccountAddress::Static(parser::SolanaAccount {
                account_key: a.account_key.clone(),
                signer: a.signer,
                writable: a.writable,
            })
        })
        .collect();

    let configs = idl_registry.get_all_configs();
    let custom_idls = if configs.is_empty() {
        None
    } else {
        Some(
            configs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    };
    let idl_records = construct_idl_records_map(custom_idls).ok()?;
    let idl_record = idl_records.get(program_key)?;
    let (idl, idl_json, idl_source) = resolve_idl_for_record(idl_record, program_key).ok()?;
    let instruction = find_instruction_by_discriminator(&data, idl.instructions.clone()).ok()?;
    let program_call_args = parse_data_into_args(&data, &instruction, &idl).ok()?;
    let named_accounts = create_accounts_map(&account_addresses, &instruction).ok()?;
    let discriminator = instruction.discriminator.clone()?;

    Some(SolanaParsedInstructionDataIo {
        instruction_name: instruction.name,
        discriminator: hex::encode(discriminator),
        named_accounts: named_accounts.into_iter().collect(),
        program_call_args_json: canonical_args_json(&program_call_args),
        idl_source: idl_source_string(&idl_source),
        idl_hash: compute_idl_hash(&idl_json),
    })
}

impl From<&SolanaMetadata> for SolanaIntermediateOutput {
    fn from(value: &SolanaMetadata) -> Self {
        Self {
            schema_version: SOLANA_INTERMEDIATE_SCHEMA_VERSION,
            account_keys: value.account_keys.clone(),
            program_keys: value.program_keys.clone(),
            instructions: value
                .instructions
                .iter()
                .map(SolanaIntermediateInstruction::from)
                .collect(),
            transfers: value.transfers.iter().map(SolTransfer::from).collect(),
            spl_transfers: value.spl_transfers.iter().map(SplTransfer::from).collect(),
            recent_blockhash: value.recent_blockhash.clone(),
            address_table_lookups: value
                .address_table_lookups
                .iter()
                .map(SolanaAddressTableLookup::from)
                .collect(),
            simulated_instructions: Vec::new(),
        }
    }
}

// -- Extraction --------------------------------------------------------------

/// Parse the transaction once via `solana_parser::parse_transaction_with_idls`
/// and project the result into a Borsh-friendly intermediate output.
///
/// `raw_message_hex` is the hex-encoded serialized message (or full
/// transaction); `full_transaction` toggles which form is being passed in,
/// matching `solana_parser`'s API.
///
/// `pub(crate)` (not `pub`) because it takes the crate-private `IdlRegistry`;
/// the schema types above are `pub` so external consumers can still decode the
/// emitted bytes.
///
/// Eventual architecture (tracked, not yet implemented): the structured decode
/// should become the single source of truth from which the VisualSign payload
/// is generated, and these bytes should be passed through as-is rather than
/// re-parsed here. Today this re-parses once, best-effort, alongside the
/// existing VisualSign generation path.
// `disallowed_types`: the `solana_parser::parse_transaction_with_idls` API
// requires a `HashMap` for its custom-IDL argument. We build one only as a
// transient adapter from the deterministic `BTreeMap` registry; it never feeds
// serialized output, so determinism is unaffected.
#[allow(clippy::disallowed_types)]
pub(crate) fn extract_solana_intermediate_output(
    raw_message_hex: &str,
    full_transaction: bool,
    idl_registry: &IdlRegistry,
) -> Result<SolanaIntermediateOutput, VisualSignError> {
    // The registry stores configs in a `BTreeMap` (determinism), but the
    // parser API takes a `HashMap`; project into one, or `None` when empty.
    let configs = idl_registry.get_all_configs();
    let custom_idls: Option<std::collections::HashMap<String, CustomIdlConfig>> =
        if configs.is_empty() {
            None
        } else {
            Some(
                configs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        };

    let response =
        parse_transaction_with_idls(raw_message_hex.to_string(), full_transaction, custom_idls)
            .map_err(|e| {
                VisualSignError::ParseError(TransactionParseError::DecodeError(format!(
                    "Failed to parse transaction for intermediate output: {e}"
                )))
            })?;

    let metadata = response
        .solana_parsed_transaction
        .payload
        .as_ref()
        .and_then(|p| p.transaction_metadata.as_ref())
        .ok_or_else(|| {
            VisualSignError::ParseError(TransactionParseError::DecodeError(
                "solana_parser returned no transaction_metadata".to_string(),
            ))
        })?;

    Ok(SolanaIntermediateOutput::from(metadata))
}

#[cfg(test)]
// `disallowed_types`: the upstream `SolanaParsedInstructionData.named_accounts`
// is a `HashMap`, so tests that build a fixture value must construct one.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_types
)]
mod tests {
    use super::*;
    use serde_json::json;
    use solana_parser::solana::structs::ProgramType;
    use std::collections::HashMap;

    fn args_map(values: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        values
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn canonical_args_json_alphabetizes_keys() {
        let map_a = args_map(&[("zeta", json!(1)), ("alpha", json!(2))]);
        let map_b = args_map(&[("alpha", json!(2)), ("zeta", json!(1))]);
        // Different insertion order should produce identical canonical JSON.
        assert_eq!(canonical_args_json(&map_a), canonical_args_json(&map_b));
        assert!(
            canonical_args_json(&map_a).find("alpha").unwrap()
                < canonical_args_json(&map_a).find("zeta").unwrap()
        );
    }

    #[test]
    fn canonical_args_json_alphabetizes_nested_keys() {
        // Same content, different insertion order at the top level, inside a
        // nested object, and inside an object that is an array element. With
        // `preserve_order` enabled transitively in the workspace, serde_json
        // serializes nested objects in insertion order, so a top-level-only
        // sort would let nested insertion order leak through and the two
        // encodings would differ. Recursive canonicalization must make them
        // byte-identical.
        let map_a = args_map(&[
            ("zeta", json!({"nzeta": 1, "nalpha": 2})),
            ("alpha", json!([{"bzeta": 3, "balpha": 4}])),
        ]);
        let map_b = args_map(&[
            ("alpha", json!([{"balpha": 4, "bzeta": 3}])),
            ("zeta", json!({"nalpha": 2, "nzeta": 1})),
        ]);
        let canonical_a = canonical_args_json(&map_a);
        let canonical_b = canonical_args_json(&map_b);
        assert_eq!(
            canonical_a, canonical_b,
            "nested insertion order must not affect canonical output"
        );

        // The canonical form must have keys in sorted order at every level.
        // Parse it back and walk key order (works whether or not
        // `preserve_order` is active: the serialized string is sorted, so the
        // parsed Map is sorted by insertion/BTree either way).
        let parsed: serde_json::Value =
            serde_json::from_str(&canonical_a).expect("canonical output is valid JSON");
        let top = parsed.as_object().expect("top-level is an object");
        let top_keys: Vec<&String> = top.keys().collect();
        assert_eq!(top_keys, vec!["alpha", "zeta"]);

        let zeta_obj = top.get("zeta").unwrap().as_object().unwrap();
        let zeta_keys: Vec<&String> = zeta_obj.keys().collect();
        assert_eq!(zeta_keys, vec!["nalpha", "nzeta"]);

        let alpha_arr = top.get("alpha").unwrap().as_array().unwrap();
        let arr_obj = alpha_arr[0].as_object().unwrap();
        let arr_keys: Vec<&String> = arr_obj.keys().collect();
        assert_eq!(arr_keys, vec!["balpha", "bzeta"]);
    }

    #[test]
    fn idl_source_string_is_stable() {
        assert_eq!(
            idl_source_string(&IdlSource::BuiltIn(ProgramType::Jupiter)),
            "BuiltIn"
        );
        assert_eq!(idl_source_string(&IdlSource::Custom), "Custom");
    }

    #[test]
    fn parsed_instruction_data_io_round_trip() {
        let mut named = HashMap::new();
        named.insert("mint".to_string(), "Mint11111111111111".to_string());
        named.insert("authority".to_string(), "Auth1111111111111".to_string());

        let upstream = SolanaParsedInstructionData {
            instruction_name: "transfer".to_string(),
            discriminator: "deadbeef".to_string(),
            named_accounts: named,
            program_call_args: args_map(&[("amount", json!(42)), ("recipient", json!("abc"))]),
            idl_source: IdlSource::Custom,
            idl_hash: "cafebabe".to_string(),
        };

        let io = SolanaParsedInstructionDataIo::from(&upstream);
        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: SolanaParsedInstructionDataIo =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
        // BTreeMap-deterministic key ordering on `named_accounts`.
        let keys: Vec<_> = io.named_accounts.keys().cloned().collect();
        assert_eq!(keys, vec!["authority".to_string(), "mint".to_string()]);
        // Args JSON is alphabetized.
        assert_eq!(
            io.program_call_args_json,
            r#"{"amount":42,"recipient":"abc"}"#
        );
        assert_eq!(io.idl_source, "Custom");
    }

    #[test]
    fn is_unregistered_false_for_native_program_via_simulation() {
        // System Program is trusted regardless of IDL match -- is_unregistered
        // must come from is_trusted_program, not from an IDL decode result.
        let simulated = generated::parser::SimulatedInstruction {
            program_key: "11111111111111111111111111111111".to_string(),
            instruction_data_hex: "0200000001000000000000000000".to_string(),
            accounts: vec![],
            stack_height: 2,
            rpc_parsed_data: None,
        };
        assert!(!build_simulated_instruction(&simulated, 0, &IdlRegistry::new()).is_unregistered);
    }

    #[test]
    fn is_unregistered_true_for_unknown_program_via_simulation() {
        let simulated = generated::parser::SimulatedInstruction {
            program_key: "Unknown1111111111111111111111111111111111".to_string(),
            instruction_data_hex: "ff".to_string(),
            accounts: vec![],
            stack_height: 2,
            rpc_parsed_data: None,
        };
        assert!(build_simulated_instruction(&simulated, 0, &IdlRegistry::new()).is_unregistered);
    }

    #[test]
    fn is_unregistered_false_for_preset_covered_program_via_simulation() {
        // Squads v4 multisig: no ProgramType/IDL entry in solana_parser at
        // all, but it has an in-crate preset visualizer, so is_trusted_program
        // must cover it via preset_program_ids().
        let simulated = generated::parser::SimulatedInstruction {
            program_key: "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf".to_string(),
            instruction_data_hex: "deadbeef".to_string(),
            accounts: vec![],
            stack_height: 2,
            rpc_parsed_data: None,
        };
        assert!(!build_simulated_instruction(&simulated, 0, &IdlRegistry::new()).is_unregistered);
    }

    #[test]
    fn is_unregistered_true_for_real_unregistered_router_with_trusted_jupiter_cpi() {
        let router_program_id = "8KQG1MYXru73rqobftpFjD3hBD8Ab3jaag8wbjZG63sx";
        let router_instruction_data_hex = "f8c69e91e17587c82a000000c1209b3341d69c810402000000386400012f000064010280841e00000000000d78940000000000320000";
        let jupiter_program_id = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
        let jupiter_cpi_data_hex =
            "c1209b3341d69c810402000000386400012f000064010280841e00000000000d78940000000000320000";

        let router = generated::parser::SimulatedInstruction {
            program_key: router_program_id.to_string(),
            instruction_data_hex: router_instruction_data_hex.to_string(),
            accounts: vec![],
            stack_height: 1,
            rpc_parsed_data: None,
        };
        let jupiter_cpi = generated::parser::SimulatedInstruction {
            program_key: jupiter_program_id.to_string(),
            instruction_data_hex: jupiter_cpi_data_hex.to_string(),
            accounts: vec![],
            stack_height: 2,
            rpc_parsed_data: None,
        };

        let registry = IdlRegistry::new();
        assert!(build_simulated_instruction(&router, 0, &registry).is_unregistered);
        assert!(!build_simulated_instruction(&jupiter_cpi, 0, &registry).is_unregistered);
    }

    #[test]
    fn is_unregistered_true_survives_borsh_round_trip() {
        let simulated = generated::parser::SimulatedInstruction {
            program_key: "Unknown1111111111111111111111111111111111".to_string(),
            instruction_data_hex: "ff".to_string(),
            accounts: vec![],
            stack_height: 2,
            rpc_parsed_data: None,
        };
        let io = build_simulated_instruction(&simulated, 0, &IdlRegistry::new());

        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: SolanaSimulatedInstruction =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
        assert!(recovered.is_unregistered);
    }

    #[test]
    fn intermediate_output_round_trip_is_deterministic() {
        let metadata = SolanaMetadata {
            signatures: vec![],
            account_keys: vec!["A1".to_string(), "B2".to_string()],
            program_keys: vec!["P1".to_string()],
            instructions: vec![],
            transfers: vec![],
            spl_transfers: vec![],
            recent_blockhash: "blockhash".to_string(),
            address_table_lookups: vec![],
        };
        let io = SolanaIntermediateOutput::from(&metadata);
        assert_eq!(io.schema_version, SOLANA_INTERMEDIATE_SCHEMA_VERSION);
        assert_eq!(io.account_keys, vec!["A1".to_string(), "B2".to_string()]);
        assert_eq!(io.program_keys, vec!["P1".to_string()]);
        assert!(io.instructions.is_empty());
        assert_eq!(io.recent_blockhash, "blockhash");

        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let bytes_again = borsh::to_vec(&io).expect("borsh serializes");
        assert_eq!(bytes, bytes_again, "borsh encoding must be deterministic");
        let recovered: SolanaIntermediateOutput =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
    }
}
