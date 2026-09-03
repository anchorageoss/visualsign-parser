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
//!   (`program_call_args_json`), as is the RPC's own decode (`parsed_json`),
//!   because `serde_json::Value` does not implement
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
    self as parser, AccountAddress, IdlParseError, IdlSource, SolanaMetadata,
    SolanaParsedInstructionData,
};
use solana_parser::{CustomIdlConfig, parse_transaction_with_idl_records};
use visualsign::errors::VisualSignError;
use visualsign::vsptrait::TransactionParseError;

use crate::idl::IdlRegistry;

/// Version of the `SolanaIntermediateOutput` Borsh schema. Bump on ANY change
/// to the shape below. Mirrored decoders assert this value, so a bump makes a
/// schema drift fail loudly instead of silently misparsing.
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
    pub simulated_instructions: Vec<SolanaSimulatedInstruction>,
    /// Why the caller's `simulated_transaction_result` could not be read.
    /// `None` means it was read or none was sent, and `simulated_instructions`
    /// is authoritative -- empty there means the simulation genuinely had no
    /// inner instructions.
    pub simulation_error: Option<SolanaSimulationError>,
}

/// Why a caller-supplied `simulateTransaction` result could not be read. The
/// detail is logged at WARN; these variants are the wire contract.
///
/// Discriminants start at 1 so 0 is not a value this type can hold: a decoder
/// that renders `Option::None` as the zero value (borsh-go v0.3.1 does) can
/// then tell an absent error from `InvalidBase64`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
pub enum SolanaSimulationError {
    InvalidBase64 = 1,
    /// Usually the whole JSON-RPC envelope where the bare `result` was expected.
    InvalidJson = 2,
    /// `value.err` was set, so the trace is partial and was dropped.
    SimulationFailed = 3,
    CallerIdlRecordsUnusable = 4,
    /// An inner instruction arrived compiled. `simulateTransaction` parses inner
    /// instructions whatever the transaction encoding, so the input was not one
    /// of its results.
    CompiledInstruction = 5,
    /// An inner instruction's data was not valid base58, which the RPC never
    /// emits.
    InvalidInstructionData = 6,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaIntermediateInstruction {
    pub program_key: String,
    pub accounts: Vec<SolanaAccount>,
    pub instruction_data_hex: String,
    pub address_table_lookups: Vec<SolanaSingleAddressTableLookup>,
    /// `None` when the parser could not match an IDL for this instruction.
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
    pub idl_parse_error: Option<SolanaIdlParseError>,
    /// Where `program_key` was registered, if at all -- see [`RegisteredSource`].
    pub registered_source: RegisteredSource,
}

/// Where a program ID was recognized. Decodability is a separate question --
/// `system`, `spl_token`, `token_2022`, `compute_budget`,
/// `associated_token_account`, `stakepool` and `swig_wallet` are all registered
/// and ship no IDL -- so read `parsed_instruction_data`,
/// `solana_rpc_parsed_data` and `idl_parse_error` for that.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisteredSource {
    /// Matched `idl::builtin_programs`'s `NATIVE_PROGRAM_NAMES` list (native
    /// runtime / core SPL programs).
    Native,
    /// Matched an in-crate preset visualizer's program ID
    Preset,
    /// Matched a program known only via `solana_parser::ProgramType`
    ThirdParty,
    /// Matched only via caller-provided `idl_mappings`.
    CallerSupplied,
    /// Matched none of the above; nothing was found at all.
    Unregistered,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaSimulatedInstruction {
    /// Index of the outer instruction this was invoked under. A grouping key,
    /// not a unique one: every instruction in a CPI group shares it.
    pub index: u32,
    /// `0` when the RPC omitted it. Inner instructions are CPIs, so a real
    /// value is always >= 2.
    pub stack_height: u32,
    pub program_key: String,
    /// Empty when `solana_rpc_parsed_data` is set: that response shape carries
    /// its account keys inside `parsed`, under per-program field names.
    pub accounts: Vec<String>,
    /// Empty when `solana_rpc_parsed_data` is set: the RPC consumes the
    /// instruction data to produce `parsed` and does not return it.
    pub instruction_data_hex: String,
    pub registered_source: RegisteredSource,
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
    /// The RPC's own jsonParsed decode, for the recognized programs it returns
    /// that way (System/Token and friends). `None` for partially-decoded
    /// instructions, which we IDL-decode into `parsed_instruction_data` instead.
    pub solana_rpc_parsed_data: Option<SolanaRpcParsedInstructionDataIo>,
    pub idl_parse_error: Option<SolanaIdlParseError>,
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
/// recognized programs.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaRpcParsedInstructionDataIo {
    pub program: String,
    pub parsed_json: String,
}

/// Why IDL decode failed for an instruction, when it was attempted at all.
/// Mirrors `solana_parser::solana::structs::IdlParseError`, flattened for
/// Borsh (that upstream type carries no Borsh derive). `None` on
/// `parsed_instruction_data`/`solana_rpc_parsed_data`'s siblings means either decode
/// succeeded or no IDL was available to attempt against in the first place --
/// distinct from an attempt that ran and failed, which this type identifies.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum SolanaIdlParseError {
    /// The instruction data could not be decoded into the IDL's argument
    /// types (e.g. an unknown enum variant in the data).
    DataParseError {
        instruction_name: String,
        error: String,
    },
    /// The accounts list could not be mapped to the IDL's named accounts.
    AccountsMapError {
        instruction_name: String,
        error: String,
    },
    /// No instruction in the IDL matched the discriminator bytes -- the
    /// program is known, but this specific call isn't one of its documented
    /// instructions (e.g. an Anchor event-log self-CPI).
    DiscriminatorNotFound(String),
    /// The IDL itself could not be resolved (missing, malformed, etc.).
    IdlResolutionError(String),
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
        IdlSource::Preset => "Preset".to_string(),
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

impl From<&IdlParseError> for SolanaIdlParseError {
    fn from(value: &IdlParseError) -> Self {
        match value {
            IdlParseError::DataParseError {
                instruction_name,
                error,
            } => Self::DataParseError {
                instruction_name: instruction_name.clone(),
                error: error.clone(),
            },
            IdlParseError::AccountsMapError {
                instruction_name,
                error,
            } => Self::AccountsMapError {
                instruction_name: instruction_name.clone(),
                error: error.clone(),
            },
            IdlParseError::DiscriminatorNotFound(msg) => Self::DiscriminatorNotFound(msg.clone()),
            IdlParseError::IdlResolutionError(msg) => Self::IdlResolutionError(msg.clone()),
        }
    }
}

/// Builds a [`SolanaIntermediateInstruction`] from `solana_parser`'s own
/// top-level decode output. Not a `From` impl because `registered_source`
/// needs `caller_idl_program_ids` (the keys of `IdlRegistry::get_all_configs()`),
/// which `parser::SolanaInstruction` doesn't carry.
fn build_intermediate_instruction(
    value: &parser::SolanaInstruction,
    caller_idl_program_ids: &std::collections::BTreeMap<String, CustomIdlConfig>,
) -> SolanaIntermediateInstruction {
    SolanaIntermediateInstruction {
        program_key: value.program_key.clone(),
        accounts: value.accounts.iter().map(SolanaAccount::from).collect(),
        instruction_data_hex: value.instruction_data_hex.clone(),
        address_table_lookups: value
            .address_table_lookups
            .iter()
            .map(SolanaSingleAddressTableLookup::from)
            .collect(),
        registered_source: crate::idl::builtin_programs::registered_source(
            &value.program_key,
            caller_idl_program_ids,
        ),
        parsed_instruction_data: value
            .parsed_instruction
            .as_ref()
            .map(SolanaParsedInstructionDataIo::from),
        idl_parse_error: value
            .idl_parse_error
            .as_ref()
            .map(SolanaIdlParseError::from),
    }
}

/// Unmarshals the raw `simulateTransaction` RPC bytes and IDL-decodes every inner
/// instruction across all groups in one pass, returning a flat, borsh-ready list.
///
/// On any problem the list comes back empty with a [`SolanaSimulationError`]
/// saying why, so "we could not read this" stays distinguishable from "there was
/// nothing to find". A simulation with no `innerInstructions` is the latter.
pub(crate) fn parse_and_decode_simulated_instructions(
    raw_json: &[u8],
    idl_registry: &IdlRegistry,
) -> (
    Vec<SolanaSimulatedInstruction>,
    Option<SolanaSimulationError>,
) {
    let response: solana_rpc_client_types::response::Response<
        solana_rpc_client_types::response::RpcSimulateTransactionResult,
    > = match serde_json::from_slice(raw_json) {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "simulated_transaction_result is not a simulateTransaction result"
            );
            return (Vec::new(), Some(SolanaSimulationError::InvalidJson));
        }
    };

    if let Some(err) = &response.value.err {
        tracing::warn!(
            error = ?err,
            "simulated transaction reverted; dropping its partial inner-instruction trace"
        );
        return (Vec::new(), Some(SolanaSimulationError::SimulationFailed));
    }

    let Some(inner_instructions) = response.value.inner_instructions else {
        return (Vec::new(), None);
    };
    decode_inner_instructions(inner_instructions, idl_registry)
}

/// Builds one [`SolanaSimulatedInstruction`] per inner instruction across every
/// outer instruction's `UiInnerInstructions` entry, from the raw `simulateTransaction`
/// RPC shape
fn decode_inner_instructions(
    inner_instructions: Vec<solana_transaction_status::UiInnerInstructions>,
    idl_registry: &IdlRegistry,
) -> (
    Vec<SolanaSimulatedInstruction>,
    Option<SolanaSimulationError>,
) {
    use solana_transaction_status::{UiInstruction, UiParsedInstruction};

    let configs = idl_registry.get_all_configs();
    // Caller records only. Presets come from the process-wide cache and the
    // builtins from `solana_parser`'s own; `lookup_idl_record` layers the three
    // per program rather than merging them into one map, so neither the ~2.0 MB
    // of preset IDLs nor the builtins are cloned per request.
    //
    // Defensive: the static path rejects the request over the same records
    // before this runs.
    let Some(caller_records) = caller_idl_records(configs) else {
        tracing::warn!("caller-supplied IDLs could not be built into records");
        return (
            Vec::new(),
            Some(SolanaSimulationError::CallerIdlRecordsUnusable),
        );
    };

    let mut simulated_instructions = Vec::new();

    for entry in inner_instructions {
        let outer_index = u32::from(entry.index);

        for ui_instruction in entry.instructions {
            let UiInstruction::Parsed(parsed) = ui_instruction else {
                tracing::warn!(
                    "unexpected compiled inner instruction in simulated_transaction_result; \
                     simulateTransaction does not return this shape"
                );
                return (Vec::new(), Some(SolanaSimulationError::CompiledInstruction));
            };

            match parsed {
                UiParsedInstruction::PartiallyDecoded(decoded) => {
                    let accounts = decoded.accounts;
                    let Ok(data) = bs58::decode(&decoded.data).into_vec() else {
                        tracing::warn!(
                            program_id = %decoded.program_id,
                            "inner instruction data is not valid base58"
                        );
                        return (
                            Vec::new(),
                            Some(SolanaSimulationError::InvalidInstructionData),
                        );
                    };
                    let (parsed_instruction_data, idl_parse_error) =
                        parse_partially_decoded_instruction_idl(
                            &decoded.program_id,
                            &decoded.data,
                            &accounts,
                            &caller_records,
                        );
                    let registered_source = crate::idl::builtin_programs::registered_source(
                        &decoded.program_id,
                        configs,
                    );

                    simulated_instructions.push(SolanaSimulatedInstruction {
                        index: outer_index,
                        stack_height: decoded.stack_height.unwrap_or(0),
                        program_key: decoded.program_id,
                        accounts,
                        instruction_data_hex: hex::encode(&data),
                        registered_source,
                        parsed_instruction_data,
                        solana_rpc_parsed_data: None,
                        idl_parse_error,
                    });
                }
                UiParsedInstruction::Parsed(rpc_parsed) => {
                    let parsed_json = canonicalize_value(&rpc_parsed.parsed).to_string();
                    let program = rpc_parsed.program.clone();
                    let registered_source = crate::idl::builtin_programs::registered_source(
                        &rpc_parsed.program_id,
                        configs,
                    );

                    simulated_instructions.push(SolanaSimulatedInstruction {
                        index: outer_index,
                        stack_height: rpc_parsed.stack_height.unwrap_or(0),
                        program_key: rpc_parsed.program_id,
                        accounts: Vec::new(),
                        instruction_data_hex: String::new(),
                        registered_source,
                        parsed_instruction_data: None,
                        solana_rpc_parsed_data: Some(SolanaRpcParsedInstructionDataIo {
                            program,
                            parsed_json,
                        }),
                        idl_parse_error: None,
                    });
                }
            }
        }
    }

    (simulated_instructions, None)
}

/// Resolves one program's `IdlRecord` across the three sources, in the same
/// precedence order the old single merged map encoded: a caller-supplied record
/// wins, then a preset, then a `solana_parser` builtin.
///
/// Layered rather than merged so the preset records (~2.0 MB) and the builtins
/// are borrowed from their process-wide caches instead of being cloned into a
/// fresh map on every request.
fn lookup_idl_record<'a>(
    program_id: &str,
    caller_records: &'a BTreeMap<String, solana_parser::solana::structs::IdlRecord>,
) -> Option<&'a solana_parser::solana::structs::IdlRecord> {
    if let Some(record) = caller_records.get(program_id) {
        return Some(record);
    }
    if let Some(record) = crate::idl::builtin_programs::preset_idl_records().get(program_id) {
        return Some(record);
    }
    builtin_idl_records().get(program_id)
}

/// Builds `IdlRecord`s for the caller-supplied IDLs alone.
///
/// `construct_idl_records_map` always prepends `solana_parser`'s builtins, so
/// the builtin entries it returns are dropped here -- [`builtin_idl_records`]
/// already holds them, cached. `None` signals that a caller IDL failed to parse.
#[allow(clippy::disallowed_types)]
fn caller_idl_records(
    configs: &BTreeMap<String, CustomIdlConfig>,
) -> Option<BTreeMap<String, solana_parser::solana::structs::IdlRecord>> {
    if configs.is_empty() {
        return Some(BTreeMap::new());
    }
    let records = construct_idl_records_map(Some(
        configs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
    .ok()?;
    Some(
        records
            .into_iter()
            .filter(|(program_id, _)| configs.contains_key(program_id))
            .collect(),
    )
}

/// The full `IdlRecord` map for one request: builtins and presets from their
/// process-wide caches, caller-supplied IDLs layered on top.
///
/// `solana_parser::parse_transaction_with_idl_records` takes the map by value,
/// so the static path has to materialize one. The saving over passing configs
/// is the parse-and-re-serialize of every preset IDL, which
/// `construct_idl_records_map` would otherwise redo on every request.
///
/// Precedence matches [`lookup_idl_record`]: caller, then preset, then builtin.
#[allow(clippy::disallowed_types)]
fn build_idl_record_map(
    caller_records: &BTreeMap<String, solana_parser::solana::structs::IdlRecord>,
) -> std::collections::HashMap<String, solana_parser::solana::structs::IdlRecord> {
    let mut records: std::collections::HashMap<_, _> = builtin_idl_records()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (program_id, record) in crate::idl::builtin_programs::preset_idl_records() {
        records.insert(program_id.clone(), record.clone());
    }
    for (program_id, record) in caller_records {
        records.insert(program_id.clone(), record.clone());
    }
    records
}

/// `solana_parser`'s own builtin records, built once per process.
///
/// `construct_custom_idl_records_map` takes no arguments and returns the same
/// 21 records every call, so there is nothing per-request about it.
fn builtin_idl_records() -> &'static BTreeMap<String, solana_parser::solana::structs::IdlRecord> {
    static BUILTIN_IDL_RECORDS: std::sync::OnceLock<
        BTreeMap<String, solana_parser::solana::structs::IdlRecord>,
    > = std::sync::OnceLock::new();
    BUILTIN_IDL_RECORDS.get_or_init(|| {
        solana_parser::construct_custom_idl_records_map()
            .map(|records| records.into_iter().collect())
            .unwrap_or_default()
    })
}

/// IDL-decodes one `PartiallyDecoded` instruction's raw data, using the exact same
/// resolution chain the top-level static decoder's private `parse_idl` uses internally.
/// The record is resolved by [`lookup_idl_record`], mirroring `parse_idl`'s
/// `custom_idls.get(program_key)`. Returns `(None, None)` when `program_id` has
/// no `IdlRecord` at all -- nothing was available to attempt against, distinct
/// from an attempt that ran and failed (`(None, Some(err))`).
fn parse_partially_decoded_instruction_idl(
    program_id: &str,
    data_base58: &str,
    accounts: &[String],
    caller_records: &BTreeMap<String, solana_parser::solana::structs::IdlRecord>,
) -> (
    Option<SolanaParsedInstructionDataIo>,
    Option<SolanaIdlParseError>,
) {
    let Some(idl_record) = lookup_idl_record(program_id, caller_records) else {
        return (None, None);
    };
    let (idl, idl_json, idl_source) = match resolve_idl_for_record(idl_record, program_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                Some(SolanaIdlParseError::IdlResolutionError(e.to_string())),
            );
        }
    };
    // A malformed base58 payload isn't an IDL-resolution problem (the IDL
    // resolved fine); treat it as "no instruction could be matched" since
    // there's no byte data to check a discriminator against.
    let Ok(data) = bs58::decode(data_base58).into_vec() else {
        return (
            None,
            Some(SolanaIdlParseError::DiscriminatorNotFound(
                "instruction data is not valid base58".to_string(),
            )),
        );
    };
    let instruction = match find_instruction_by_discriminator(&data, idl.instructions.clone()) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                Some(SolanaIdlParseError::DiscriminatorNotFound(e.to_string())),
            );
        }
    };
    let program_call_args = match parse_data_into_args(&data, &instruction, &idl) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                Some(SolanaIdlParseError::DataParseError {
                    instruction_name: instruction.name,
                    error: e.to_string(),
                }),
            );
        }
    };
    // signer/writable are unavailable for a simulated PartiallyDecoded
    // instruction; create_accounts_map only reads the account key (via
    // AccountAddress's Display impl), so the flags below are unused filler
    // required only by parser::SolanaAccount's shape.
    let account_addresses: Vec<AccountAddress> = accounts
        .iter()
        .map(|account_key| {
            AccountAddress::Static(parser::SolanaAccount {
                account_key: account_key.clone(),
                signer: false,
                writable: false,
            })
        })
        .collect();
    let named_accounts = match create_accounts_map(&account_addresses, &instruction) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                Some(SolanaIdlParseError::AccountsMapError {
                    instruction_name: instruction.name,
                    error: e.to_string(),
                }),
            );
        }
    };
    let Some(discriminator) = instruction.discriminator.clone() else {
        // We only reach here after matching by discriminator above, so this
        // is unreachable in practice; report it the same way solana_parser's
        // own parse_idl does for the analogous case.
        return (
            None,
            Some(SolanaIdlParseError::DiscriminatorNotFound(
                "matched instruction has no discriminator".to_string(),
            )),
        );
    };

    (
        Some(SolanaParsedInstructionDataIo {
            instruction_name: instruction.name,
            discriminator: hex::encode(discriminator),
            named_accounts: named_accounts.into_iter().collect(),
            program_call_args_json: canonical_args_json(&program_call_args),
            idl_source: idl_source_string(&idl_source),
            idl_hash: compute_idl_hash(&idl_json),
        }),
        None,
    )
}

/// Builds a [`SolanaIntermediateOutput`] from `solana_parser`'s own top-level
/// decode output. Not a `From` impl because `registered_source` on each
/// instruction needs `caller_idl_program_ids` (the keys of
/// `IdlRegistry::get_all_configs()`), which `SolanaMetadata` doesn't carry.
fn build_intermediate_output(
    value: &SolanaMetadata,
    caller_idl_program_ids: &std::collections::BTreeMap<String, CustomIdlConfig>,
) -> SolanaIntermediateOutput {
    SolanaIntermediateOutput {
        schema_version: SOLANA_INTERMEDIATE_SCHEMA_VERSION,
        account_keys: value.account_keys.clone(),
        program_keys: value.program_keys.clone(),
        instructions: value
            .instructions
            .iter()
            .map(|instruction| build_intermediate_instruction(instruction, caller_idl_program_ids))
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
        simulation_error: None,
    }
}

// -- Extraction --------------------------------------------------------------

/// Parse the transaction once via `solana_parser::parse_transaction_with_idl_records`
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
// `disallowed_types`: the `solana_parser::parse_transaction_with_idl_records`
// API requires a `HashMap` for its record argument. We build one only as a
// transient adapter from the deterministic `BTreeMap` caches; it never feeds
// serialized output, so determinism is unaffected.
#[allow(clippy::disallowed_types)]
pub(crate) fn extract_solana_intermediate_output(
    raw_message_hex: &str,
    full_transaction: bool,
    idl_registry: &IdlRegistry,
) -> Result<SolanaIntermediateOutput, VisualSignError> {
    let configs = idl_registry.get_all_configs();
    let caller_records = caller_idl_records(configs).ok_or_else(|| {
        VisualSignError::ParseError(TransactionParseError::DecodeError(
            "Failed to build IDL records from caller-supplied IDLs".to_string(),
        ))
    })?;

    let response = parse_transaction_with_idl_records(
        raw_message_hex.to_string(),
        full_transaction,
        build_idl_record_map(&caller_records),
    )
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

    Ok(build_intermediate_output(metadata, configs))
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
        assert_eq!(idl_source_string(&IdlSource::Preset), "Preset");
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
    fn unreadable_simulation_is_distinguishable_from_an_empty_one() {
        let empty =
            br#"{"context":{"slot":1},"value":{"err":null,"logs":[],"innerInstructions":[]}}"#;
        let (instructions, error) =
            parse_and_decode_simulated_instructions(empty, &IdlRegistry::new());
        assert!(instructions.is_empty());
        assert!(error.is_none(), "genuinely empty carries no error");

        let (instructions, error) =
            parse_and_decode_simulated_instructions(b"not-json", &IdlRegistry::new());
        assert!(instructions.is_empty());
        assert_eq!(error, Some(SolanaSimulationError::InvalidJson));
    }

    #[test]
    fn full_jsonrpc_envelope_reports_invalid_json() {
        let envelope = br#"{"jsonrpc":"2.0","id":1,"result":{"context":{"slot":1},"value":{"err":null,"innerInstructions":[]}}}"#;
        let (instructions, error) =
            parse_and_decode_simulated_instructions(envelope, &IdlRegistry::new());
        assert!(instructions.is_empty());
        assert_eq!(error, Some(SolanaSimulationError::InvalidJson));
    }

    #[test]
    fn reverted_simulation_drops_its_partial_trace() {
        let reverted = br#"{"context":{"slot":1},"value":{"err":{"InstructionError":[3,{"Custom":6001}]},"logs":[],"innerInstructions":[{"index":0,"instructions":[{"accounts":["D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf"],"data":"3Bxs","programId":"QuaNtZsgYRe5Z9Bk4LZ4cTD9tbkVoyCNf1R2BN9bBDv","stackHeight":2}]}]}}"#;
        let (instructions, error) =
            parse_and_decode_simulated_instructions(reverted, &IdlRegistry::new());
        assert!(
            instructions.is_empty(),
            "a partial trace must not attach as a complete one"
        );
        assert_eq!(error, Some(SolanaSimulationError::SimulationFailed));
    }

    #[test]
    fn compiled_inner_instruction_is_rejected() {
        let compiled = br#"{"context":{"slot":1},"value":{"err":null,"innerInstructions":[{"index":0,"instructions":[{"programIdIndex":4,"accounts":[1,2],"data":"3Bxs","stackHeight":2}]}]}}"#;
        let (instructions, error) =
            parse_and_decode_simulated_instructions(compiled, &IdlRegistry::new());
        assert!(instructions.is_empty());
        assert_eq!(error, Some(SolanaSimulationError::CompiledInstruction));
    }

    #[test]
    fn simulation_error_round_trips_through_borsh() {
        for (error, tag) in [
            (SolanaSimulationError::InvalidBase64, 1u8),
            (SolanaSimulationError::InvalidJson, 2),
            (SolanaSimulationError::SimulationFailed, 3),
            (SolanaSimulationError::CallerIdlRecordsUnusable, 4),
            (SolanaSimulationError::CompiledInstruction, 5),
            (SolanaSimulationError::InvalidInstructionData, 6),
        ] {
            let io = SolanaIntermediateOutput {
                schema_version: SOLANA_INTERMEDIATE_SCHEMA_VERSION,
                account_keys: vec![],
                program_keys: vec![],
                instructions: vec![],
                transfers: vec![],
                spl_transfers: vec![],
                recent_blockhash: "blockhash".to_string(),
                address_table_lookups: vec![],
                simulated_instructions: vec![],
                simulation_error: Some(error),
            };
            let bytes = borsh::to_vec(&io).expect("borsh serializes");
            let recovered: SolanaIntermediateOutput =
                borsh::from_slice(&bytes).expect("borsh deserializes");
            assert_eq!(io, recovered);
            // Trailing `01 <tag>`: Some, then the variant. No payload.
            assert_eq!(bytes[bytes.len() - 2], 1);
            assert_eq!(bytes[bytes.len() - 1], tag, "{error:?} tag");
            assert_ne!(tag, 0, "0 stays free to mean None");
        }
    }

    #[test]
    fn registered_source_classifications_from_jupiter_route_simulation() {
        let raw_json =
            include_bytes!("../tests/fixtures/simulated_instructions/jupiter_route_sim_resp.json");
        let response: solana_rpc_client_types::response::Response<
            solana_rpc_client_types::response::RpcSimulateTransactionResult,
        > = serde_json::from_slice(raw_json)
            .expect("fixture parses as a simulateTransaction result");
        let inner_instructions = response
            .value
            .inner_instructions
            .expect("fixture has innerInstructions");

        let (instructions, simulation_error) =
            decode_inner_instructions(inner_instructions, &IdlRegistry::new());
        assert!(simulation_error.is_none());
        assert_eq!(instructions.len(), 4, "fixture carries four inner calls");

        assert_eq!(
            instructions[0].program_key,
            "QuaNtZsgYRe5Z9Bk4LZ4cTD9tbkVoyCNf1R2BN9bBDv"
        );
        assert_eq!(
            instructions[0].registered_source,
            RegisteredSource::Unregistered
        );
        assert!(instructions[0].parsed_instruction_data.is_none());
        assert!(instructions[0].solana_rpc_parsed_data.is_none());

        for i in [1, 2] {
            assert_eq!(
                instructions[i].program_key,
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            );
            assert_eq!(instructions[i].registered_source, RegisteredSource::Native);
            assert!(instructions[i].parsed_instruction_data.is_none());
            assert!(instructions[i].solana_rpc_parsed_data.is_some());
            assert!(instructions[i].idl_parse_error.is_none());
        }

        assert_eq!(
            instructions[3].program_key,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
        );
        assert_eq!(instructions[3].registered_source, RegisteredSource::Preset);
        assert!(instructions[3].parsed_instruction_data.is_none());
        assert!(matches!(
            instructions[3].idl_parse_error,
            Some(SolanaIdlParseError::DiscriminatorNotFound(_))
        ));
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
        let io = build_intermediate_output(&metadata, &std::collections::BTreeMap::new());
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
