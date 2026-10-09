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
//!   (`program_call_args_json`), as is each `jsonParsed` decode (`parsed_json`),
//!   because `serde_json::Value` does not implement
//!   `BorshSerialize`. Keys are alphabetized at *every* nesting level: the
//!   `serde_json::Value` tree is walked and re-keyed into sorted order, so the
//!   output is independent of `serde_json`'s `preserve_order` build feature
//!   (which is enabled transitively elsewhere in the workspace and would
//!   otherwise serialize nested objects in insertion order).

use std::collections::BTreeMap;
use std::str::FromStr;

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
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::message::v0::LoadedAddresses;
use solana_sdk::message::{AccountKeys, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::VersionedTransaction;
use visualsign::canonical_json;
use visualsign::errors::VisualSignError;
use visualsign::vsptrait::TransactionParseError;

use crate::idl::IdlRegistry;

mod native_instruction_data;

use native_instruction_data::NativeDecode;

/// How an account read through a lookup table is named in `named_accounts`,
/// on the IDL path (`solana_parser`) and the native one alike.
const ADDRESS_TABLE_LOOKUP: &str = "ADDRESS_TABLE_LOOKUP";

/// Why a `Native` instruction's program cannot be decoded at all. Shared by
/// the outer and inner paths so the HSM sees one marker for it.
const UNSUPPORTED_NATIVE_PROGRAM: &str = "program has no native decoder";

/// Version of the `SolanaIntermediateOutput` Borsh schema. Bump on ANY change
/// to the shape below. Mirrored decoders assert this value, so a bump makes a
/// schema drift fail loudly instead of silently misparsing.
pub const SOLANA_INTERMEDIATE_SCHEMA_VERSION: u16 = 4;

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
    /// The IDL decode, or for a `Native` program the native decode (empty
    /// `idl_source`). `None` when neither applies; the `*_error` fields say why.
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
    pub idl_parse_error: Option<SolanaIdlParseError>,
    /// Where `program_key` was registered, if at all -- see [`RegisteredSource`].
    pub registered_source: RegisteredSource,
    /// Solana's `jsonParsed` decode of a `Native` instruction, or the parser's
    /// own for Compute Budget. `None` for other programs or on error.
    pub solana_json_parsed_data: Option<SolanaJsonParsedInstructionDataIo>,
    /// Why a `Native` instruction has no `parsed_instruction_data`: unsupported
    /// program, decoder rejection, or an unmapped decode. `None` otherwise.
    pub solana_json_parse_error: Option<String>,
    /// Position in `accounts` of each `named_accounts` entry: the first account
    /// with that key. Absent for lookup-table accounts (not in `accounts`).
    pub named_account_indices: BTreeMap<String, u32>,
}

/// Where a program ID was recognized. Decodability is a separate question --
/// `system`, `spl_token`, `token_2022`, `compute_budget`,
/// `associated_token_account`, `stakepool` and `swig_wallet` are all registered
/// and ship no IDL -- so read `parsed_instruction_data`, `solana_json_parsed_data`,
/// `solana_rpc_parsed_data`, `idl_parse_error` and `solana_json_parse_error` for
/// that.
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
    /// The IDL decode, or for a `Native` program the same native decode as a
    /// top-level instruction gets. `None` when neither applies.
    pub parsed_instruction_data: Option<SolanaParsedInstructionDataIo>,
    /// The RPC's own jsonParsed decode, for the programs it returns that way.
    /// `None` for partially-decoded instructions.
    pub solana_rpc_parsed_data: Option<SolanaRpcParsedInstructionDataIo>,
    pub idl_parse_error: Option<SolanaIdlParseError>,
    /// Why a `Native` instruction has no `parsed_instruction_data`, as on
    /// [`SolanaIntermediateInstruction`]. `None` otherwise.
    pub solana_json_parse_error: Option<String>,
    /// Position in `accounts` of each `named_accounts` entry, as on
    /// [`SolanaIntermediateInstruction`]. Empty when `accounts` is.
    pub named_account_indices: BTreeMap<String, u32>,
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

/// A top-level instruction decoded by Solana's own `jsonParsed` decoder, run
/// by the parser.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaJsonParsedInstructionDataIo {
    /// The decoder's program name, e.g. `system`, `spl-token`.
    pub program: String,
    /// Canonical JSON, keys alphabetized at every level. `{"info":..,"type":..}`
    /// for a typed instruction; a bare string for SPL Memo.
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
    // Canonicalized at every nesting level by the shared helper. Serializing a
    // Map<String, Value> never fails; on the off-chance it does we fall back to
    // an empty object so the surrounding borsh encoding stays well-formed.
    canonical_json::to_canonical_string(&Value::Object(args.clone()))
        .unwrap_or_else(|_| "{}".to_string())
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

/// Not a `From` impl: `registered_source` needs the caller IDL keys and the
/// native decode the compiled form, which `parser::SolanaInstruction` lacks.
fn build_intermediate_instruction(
    value: &parser::SolanaInstruction,
    compiled: Option<(&CompiledMessage<'_>, &CompiledInstruction)>,
    caller_idl_program_ids: &BTreeMap<String, CustomIdlConfig>,
) -> SolanaIntermediateInstruction {
    let registered_source =
        crate::idl::builtin_programs::registered_source(&value.program_key, caller_idl_program_ids);
    let idl_decode = value
        .parsed_instruction
        .as_ref()
        .map(SolanaParsedInstructionDataIo::from);
    let (solana_json_parsed_data, parsed_instruction_data, solana_json_parse_error) =
        if registered_source == RegisteredSource::Native {
            decode_native_outer(value, compiled, idl_decode)
        } else {
            (None, idl_decode, None)
        };
    SolanaIntermediateInstruction {
        program_key: value.program_key.clone(),
        accounts: value.accounts.iter().map(SolanaAccount::from).collect(),
        instruction_data_hex: value.instruction_data_hex.clone(),
        address_table_lookups: value
            .address_table_lookups
            .iter()
            .map(SolanaSingleAddressTableLookup::from)
            .collect(),
        registered_source,
        idl_parse_error: value
            .idl_parse_error
            .as_ref()
            .map(SolanaIdlParseError::from),
        named_account_indices: index_named_accounts(
            parsed_instruction_data.as_ref(),
            value
                .accounts
                .iter()
                .map(|account| account.account_key.as_str()),
        ),
        solana_json_parsed_data,
        solana_json_parse_error,
        parsed_instruction_data,
    }
}

/// Maps each named account to the first position in `keys` holding its key.
/// Solana dedups keys per message, so every position of one key has the same
/// signer and writable flags; the first is as good as any.
fn index_named_accounts<'a>(
    parsed: Option<&SolanaParsedInstructionDataIo>,
    keys: impl Iterator<Item = &'a str>,
) -> BTreeMap<String, u32> {
    let mut first_position: BTreeMap<&str, u32> = BTreeMap::new();
    for (position, key) in keys.enumerate() {
        if let Ok(position) = u32::try_from(position) {
            first_position.entry(key).or_insert(position);
        }
    }
    parsed
        .map(|parsed| &parsed.named_accounts)
        .into_iter()
        .flatten()
        .filter_map(|(name, key)| Some((name.clone(), *first_position.get(key.as_str())?)))
        .collect()
}

/// A `Native` top-level instruction's `jsonParsed` decode and, unless an IDL
/// decoded it, its `parsed_instruction_data`; every failure is reported.
fn decode_native_outer(
    value: &parser::SolanaInstruction,
    compiled: Option<(&CompiledMessage<'_>, &CompiledInstruction)>,
    idl_decode: Option<SolanaParsedInstructionDataIo>,
) -> (
    Option<SolanaJsonParsedInstructionDataIo>,
    Option<SolanaParsedInstructionDataIo>,
    Option<String>,
) {
    // An IDL decode already explains the instruction; no error is reported beside it.
    let unexplained = |error: String| idl_decode.is_none().then_some(error);
    let data = match visualsign::encodings::decode_hex(&value.instruction_data_hex) {
        Ok(data) => data,
        Err(e) => {
            let error = unexplained(format!("invalid instruction data hex: {e}"));
            return (None, idl_decode, error);
        }
    };
    let decoded = match run_native_decoder(value, compiled, &data) {
        Ok(decoded) => decoded,
        Err(error) => {
            let error = unexplained(error);
            return (None, idl_decode, error);
        }
    };
    let json_parsed = SolanaJsonParsedInstructionDataIo {
        program: decoded.program,
        parsed_json: decoded.parsed.to_string(),
    };
    if idl_decode.is_some() {
        return (Some(json_parsed), idl_decode, None);
    }
    let mapped =
        native_instruction_data::from_json_parsed(&value.program_key, &decoded.parsed, Some(&data));
    match mapped {
        Ok(parsed) => (Some(json_parsed), Some(parsed), None),
        Err(error) => {
            tracing::warn!(program_key = %value.program_key, %error, "native instruction decode could not be mapped");
            (Some(json_parsed), None, Some(error))
        }
    }
}

/// Runs the native decoder. With the compiled form, lookup-table accounts are
/// positioned and come out as [`ADDRESS_TABLE_LOOKUP`]; without it they fail.
fn run_native_decoder(
    value: &parser::SolanaInstruction,
    compiled: Option<(&CompiledMessage<'_>, &CompiledInstruction)>,
    data: &[u8],
) -> Result<NativeDecode, String> {
    if !native_instruction_data::is_decodable(&value.program_key) {
        tracing::debug!(program_key = %value.program_key, error = UNSUPPORTED_NATIVE_PROGRAM, "native instruction not decoded");
        return Err(UNSUPPORTED_NATIVE_PROGRAM.to_string());
    }
    let program_id = match Pubkey::from_str(&value.program_key) {
        Ok(program_id) => program_id,
        // Unreachable: every decodable program ID is a valid pubkey. Reported
        // rather than dropped all the same.
        Err(e) => return Err(format!("invalid program key: {e}")),
    };

    let decoded = match compiled {
        Some((message, instruction)) => message.decode(&program_id, instruction),
        None if !value.address_table_lookups.is_empty() => {
            let error = "instruction reads an account through an address lookup table".to_string();
            tracing::debug!(program_key = %value.program_key, %error, "native instruction not decoded");
            return Err(error);
        }
        None => {
            let accounts: Vec<String> = value
                .accounts
                .iter()
                .map(|account| account.account_key.clone())
                .collect();
            decode_rebuilt(&program_id, &accounts, data.to_vec())
        }
    };
    decoded.map_err(|error| {
        tracing::warn!(program_key = %value.program_key, %error, "native instruction could not be decoded");
        error
    })
}

/// Decodes resolved account keys plus data by rebuilding the compiled form:
/// account `i` is key `i`, and the program is the key after the last account.
fn decode_rebuilt(
    program_id: &Pubkey,
    accounts: &[String],
    data: Vec<u8>,
) -> Result<NativeDecode, String> {
    let mut keys = accounts
        .iter()
        .map(|account| {
            Pubkey::from_str(account).map_err(|e| format!("invalid account key {account}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let too_many = || {
        format!(
            "{} accounts exceed the 255 a compiled instruction can index",
            keys.len()
        )
    };
    let account_indexes = (0..keys.len())
        .map(|i| u8::try_from(i).map_err(|_| too_many()))
        .collect::<Result<Vec<_>, _>>()?;
    let program_id_index = u8::try_from(keys.len()).map_err(|_| too_many())?;
    keys.push(*program_id);
    let instruction = CompiledInstruction {
        program_id_index,
        accounts: account_indexes,
        data,
    };
    native_instruction_data::decode(program_id, &instruction, &AccountKeys::new(&keys, None))
}

/// Compiled instructions with keys the decoder can index: the static keys, then
/// one placeholder per lookup-table slot (writable first, then read-only).
struct CompiledMessage<'a> {
    instructions: &'a [CompiledInstruction],
    static_keys: &'a [Pubkey],
    placeholders: LoadedAddresses,
    /// A second, different placeholder per slot: a decode is run under both,
    /// and only the strings that differ came from a lookup-table account.
    shadow_placeholders: LoadedAddresses,
}

impl<'a> CompiledMessage<'a> {
    fn new(message: &'a VersionedMessage) -> Self {
        let (writable, readonly) = message
            .address_table_lookups()
            .unwrap_or_default()
            .iter()
            .fold((0, 0), |(writable, readonly), lookup| {
                (
                    writable + lookup.writable_indexes.len(),
                    readonly + lookup.readonly_indexes.len(),
                )
            });
        let placeholders = |set: u8| {
            let placeholder = move |slot: usize| {
                Pubkey::new_from_array(
                    solana_sdk::hash::hashv(&[
                        b"visualsign:address-table-lookup",
                        &[set],
                        &slot.to_le_bytes(),
                    ])
                    .to_bytes(),
                )
            };
            LoadedAddresses {
                writable: (0..writable).map(placeholder).collect(),
                readonly: (writable..writable + readonly).map(placeholder).collect(),
            }
        };
        Self {
            instructions: message.instructions(),
            static_keys: message.static_account_keys(),
            placeholders: placeholders(0),
            shadow_placeholders: placeholders(1),
        }
    }

    /// Decodes with the lookup-table slots filled, then masks every value that
    /// changes when the slots are filled differently: the lookup-table accounts.
    fn decode(
        &self,
        program_id: &Pubkey,
        instruction: &CompiledInstruction,
    ) -> Result<NativeDecode, String> {
        let keys = AccountKeys::new(self.static_keys, Some(&self.placeholders));
        let mut decoded = native_instruction_data::decode(program_id, instruction, &keys)?;
        if self.placeholders.is_empty() {
            return Ok(decoded);
        }
        let shadow_keys = AccountKeys::new(self.static_keys, Some(&self.shadow_placeholders));
        let shadow = native_instruction_data::decode(program_id, instruction, &shadow_keys)?;
        mask_differences(&mut decoded.parsed, &shadow.parsed);
        Ok(decoded)
    }
}

/// Rewrites every string in `value` that differs from `shadow` to
/// [`ADDRESS_TABLE_LOOKUP`]; the two trees have the same shape.
fn mask_differences(value: &mut Value, shadow: &Value) {
    match (value, shadow) {
        (Value::String(key), Value::String(other)) if key != other => {
            *key = ADDRESS_TABLE_LOOKUP.to_string();
        }
        (Value::Object(map), Value::Object(other)) => {
            for (name, item) in map.iter_mut() {
                if let Some(counterpart) = other.get(name) {
                    mask_differences(item, counterpart);
                }
            }
        }
        (Value::Array(items), Value::Array(other)) => {
            for (item, counterpart) in items.iter_mut().zip(other) {
                mask_differences(item, counterpart);
            }
        }
        _ => {}
    }
}

/// Decodes a `Native` inner instruction the RPC returned with its data and
/// accounts, through the same decoder and mapping as the top-level path.
fn decode_native_inner(
    program_id: &str,
    accounts: &[String],
    data: &[u8],
) -> Result<SolanaParsedInstructionDataIo, String> {
    if !native_instruction_data::is_decodable(program_id) {
        return Err(UNSUPPORTED_NATIVE_PROGRAM.to_string());
    }
    let program = Pubkey::from_str(program_id).map_err(|e| format!("invalid program key: {e}"))?;
    let decoded = decode_rebuilt(&program, accounts, data.to_vec())?;
    native_instruction_data::from_json_parsed(program_id, &decoded.parsed, Some(data))
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
                    // An IDL decode wins when one exists, as on the top-level path.
                    let (parsed_instruction_data, solana_json_parse_error) = if registered_source
                        == RegisteredSource::Native
                        && parsed_instruction_data.is_none()
                    {
                        match decode_native_inner(&decoded.program_id, &accounts, &data) {
                            Ok(parsed) => (Some(parsed), None),
                            Err(error) => {
                                tracing::warn!(
                                    program_id = %decoded.program_id,
                                    %error,
                                    "native inner instruction could not be decoded"
                                );
                                (None, Some(error))
                            }
                        }
                    } else {
                        (parsed_instruction_data, None)
                    };

                    let named_account_indices = index_named_accounts(
                        parsed_instruction_data.as_ref(),
                        accounts.iter().map(String::as_str),
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
                        solana_json_parse_error,
                        named_account_indices,
                    });
                }
                UiParsedInstruction::Parsed(rpc_parsed) => {
                    let parsed = canonical_json::canonicalize(&rpc_parsed.parsed);
                    let parsed_json = parsed.to_string();
                    let program = rpc_parsed.program.clone();
                    let registered_source = crate::idl::builtin_programs::registered_source(
                        &rpc_parsed.program_id,
                        configs,
                    );
                    // No instruction data to check against: the RPC consumed it.
                    let (parsed_instruction_data, solana_json_parse_error) =
                        if registered_source == RegisteredSource::Native {
                            match native_instruction_data::from_json_parsed(
                                &rpc_parsed.program_id,
                                &parsed,
                                None,
                            ) {
                                Ok(parsed) => (Some(parsed), None),
                                Err(error) => {
                                    tracing::warn!(
                                        program_id = %rpc_parsed.program_id,
                                        %error,
                                        "RPC-parsed inner instruction could not be mapped"
                                    );
                                    (None, Some(error))
                                }
                            }
                        } else {
                            (None, None)
                        };

                    simulated_instructions.push(SolanaSimulatedInstruction {
                        index: outer_index,
                        stack_height: rpc_parsed.stack_height.unwrap_or(0),
                        program_key: rpc_parsed.program_id,
                        accounts: Vec::new(),
                        instruction_data_hex: String::new(),
                        registered_source,
                        parsed_instruction_data,
                        solana_rpc_parsed_data: Some(SolanaRpcParsedInstructionDataIo {
                            program,
                            parsed_json,
                        }),
                        idl_parse_error: None,
                        solana_json_parse_error,
                        // The RPC returns no account list to index into.
                        named_account_indices: BTreeMap::new(),
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
/// `message` is the compiled form for the native decode; it must line up with
/// `value.instructions` one-to-one and is dropped otherwise.
fn build_intermediate_output(
    value: &SolanaMetadata,
    message: Option<&VersionedMessage>,
    caller_idl_program_ids: &BTreeMap<String, CustomIdlConfig>,
) -> SolanaIntermediateOutput {
    let compiled = message
        .map(CompiledMessage::new)
        .filter(|compiled| compiled.instructions.len() == value.instructions.len());
    SolanaIntermediateOutput {
        schema_version: SOLANA_INTERMEDIATE_SCHEMA_VERSION,
        account_keys: value.account_keys.clone(),
        program_keys: value.program_keys.clone(),
        instructions: value
            .instructions
            .iter()
            .enumerate()
            .map(|(i, instruction)| {
                let compiled_instruction = compiled
                    .as_ref()
                    .and_then(|message| Some((message, message.instructions.get(i)?)));
                build_intermediate_instruction(
                    instruction,
                    compiled_instruction,
                    caller_idl_program_ids,
                )
            })
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

    let message = versioned_message(raw_message_hex, full_transaction);
    Ok(build_intermediate_output(
        metadata,
        message.as_ref(),
        configs,
    ))
}

/// The message in compiled form, best-effort: on a miss the native decode
/// falls back to the static accounts.
fn versioned_message(raw_hex: &str, full_transaction: bool) -> Option<VersionedMessage> {
    let bytes = visualsign::encodings::decode_hex(raw_hex).ok()?;
    if full_transaction {
        bincode::deserialize::<VersionedTransaction>(&bytes)
            .ok()
            .map(|transaction| transaction.message)
    } else {
        bincode::deserialize::<VersionedMessage>(&bytes).ok()
    }
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
    use super::native_instruction_data::SOLANA_JSON_PARSED_PROGRAMS;
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use solana_parser::solana::structs::ProgramType;
    use solana_sdk::message::{MessageHeader, v0};
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
        assert!(instructions[0].solana_json_parse_error.is_none());

        for i in [1, 2] {
            assert_eq!(
                instructions[i].program_key,
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            );
            assert_eq!(instructions[i].registered_source, RegisteredSource::Native);
            let parsed = instructions[i]
                .parsed_instruction_data
                .as_ref()
                .expect("RPC-parsed Token transfer gets parsed_instruction_data");
            assert_eq!(parsed.instruction_name, "transfer");
            assert_eq!(parsed.discriminator, "03");
            assert!(parsed.idl_source.is_empty());
            assert!(instructions[i].solana_rpc_parsed_data.is_some());
            assert!(instructions[i].idl_parse_error.is_none());
            assert!(instructions[i].solana_json_parse_error.is_none());
        }
        let first = instructions[1].parsed_instruction_data.as_ref().unwrap();
        assert_eq!(
            first.named_accounts,
            BTreeMap::from([
                (
                    "authority".to_string(),
                    "Eb1Z35D11dmR6SFYoq87YnWN2aHpsAmeLYMwMm4oz9ZB".to_string(),
                ),
                (
                    "destination".to_string(),
                    "DvFNh2UGbzaj7BBGPHv98g646EA9k5Tj6YHPnLHEZFPm".to_string(),
                ),
                (
                    "source".to_string(),
                    "GfXcftCa3AEms45sksSokT8myQKTd252JUqeq9ji8pwk".to_string(),
                ),
            ])
        );
        let args: Value = serde_json::from_str(&first.program_call_args_json).unwrap();
        assert_eq!(args["amount"], "13133749");

        assert_eq!(
            instructions[3].program_key,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
        );
        assert_eq!(instructions[3].registered_source, RegisteredSource::Preset);
        assert!(instructions[3].parsed_instruction_data.is_none());
        assert!(instructions[3].solana_json_parse_error.is_none());
        assert!(matches!(
            instructions[3].idl_parse_error,
            Some(SolanaIdlParseError::DiscriminatorNotFound(_))
        ));
    }

    fn static_instruction(
        program_key: &str,
        accounts: &[&str],
        data: &[u8],
    ) -> parser::SolanaInstruction {
        parser::SolanaInstruction {
            program_key: program_key.to_string(),
            accounts: accounts
                .iter()
                .map(|key| parser::SolanaAccount {
                    account_key: (*key).to_string(),
                    signer: false,
                    writable: true,
                })
                .collect(),
            instruction_data_hex: hex::encode(data),
            address_table_lookups: vec![],
            parsed_instruction: None,
            idl_parse_error: None,
        }
    }

    #[test]
    fn native_system_transfer_is_decoded() {
        let source = Pubkey::new_unique().to_string();
        let destination = Pubkey::new_unique().to_string();
        let mut data = vec![0x02, 0x00, 0x00, 0x00];
        data.extend_from_slice(&1001u64.to_le_bytes());
        let instruction = static_instruction(
            "11111111111111111111111111111111",
            &[&source, &destination],
            &data,
        );

        let io = build_intermediate_instruction(&instruction, None, &BTreeMap::new());

        assert_eq!(io.registered_source, RegisteredSource::Native);
        assert_eq!(
            io.parsed_instruction_data,
            Some(SolanaParsedInstructionDataIo {
                instruction_name: "transfer".to_string(),
                discriminator: "02000000".to_string(),
                named_accounts: BTreeMap::from([
                    ("destination".to_string(), destination.clone()),
                    ("source".to_string(), source.clone()),
                ]),
                program_call_args_json: format!(
                    r#"{{"destination":"{destination}","lamports":1001,"source":"{source}"}}"#
                ),
                idl_source: String::new(),
                idl_hash: String::new(),
            }),
            "no IDL: built from the jsonParsed decode"
        );
        assert!(io.idl_parse_error.is_none());
        assert_eq!(
            io.named_account_indices,
            BTreeMap::from([("destination".to_string(), 1), ("source".to_string(), 0)])
        );
        assert_eq!(
            io.solana_json_parsed_data,
            Some(SolanaJsonParsedInstructionDataIo {
                program: "system".to_string(),
                parsed_json: format!(
                    r#"{{"info":{{"destination":"{destination}","lamports":1001,"source":"{source}"}},"type":"transfer"}}"#
                ),
            })
        );
        assert!(io.solana_json_parse_error.is_none());

        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: SolanaIntermediateInstruction =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
    }

    #[test]
    fn rpc_parsed_inner_instructions_get_their_discriminator() {
        let [
            owner,
            source,
            destination,
            pool,
            mint,
            receipt_mint,
            receipt_account,
            mint_authority,
        ] = std::array::from_fn(|_| Pubkey::new_unique().to_string());
        let liquidity = Pubkey::new_unique().to_string();
        let token = spl_token::id().to_string();
        let raw_json = serde_json::to_vec(&serde_json::json!({
            "context": { "slot": 1 },
            "value": {
                "err": null,
                "logs": [],
                "innerInstructions": [{
                    "index": 1,
                    "instructions": [
                        {
                            "programId": liquidity,
                            "accounts": [pool, destination],
                            "data": "3Bxs",
                            "stackHeight": 2
                        },
                        {
                            "program": "spl-token",
                            "programId": token,
                            "parsed": {
                                "type": "transferChecked",
                                "info": {
                                    "authority": owner,
                                    "destination": destination,
                                    "mint": mint,
                                    "source": source,
                                    "tokenAmount": {
                                        "amount": "20000",
                                        "decimals": 6,
                                        "uiAmount": 0.02,
                                        "uiAmountString": "0.02"
                                    }
                                }
                            },
                            "stackHeight": 2
                        },
                        {
                            "program": "spl-token",
                            "programId": token,
                            "parsed": {
                                "type": "mintTo",
                                "info": {
                                    "account": receipt_account,
                                    "amount": "18827",
                                    "mint": receipt_mint,
                                    "mintAuthority": mint_authority
                                }
                            },
                            "stackHeight": 2
                        },
                        {
                            "program": "spl-associated-token-account",
                            "programId": "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
                            "parsed": {
                                "type": "create",
                                "info": { "wallet": owner, "mint": mint }
                            },
                            "stackHeight": 2
                        }
                    ]
                }]
            }
        }))
        .unwrap();

        let (instructions, simulation_error) =
            parse_and_decode_simulated_instructions(&raw_json, &IdlRegistry::new());
        assert!(simulation_error.is_none());
        assert_eq!(instructions.len(), 4);

        assert!(instructions[0].parsed_instruction_data.is_none());
        assert!(!instructions[0].instruction_data_hex.is_empty());
        assert!(
            instructions[0].solana_json_parse_error.is_none(),
            "not a native program, so nothing to explain"
        );

        let transfer = instructions[1]
            .parsed_instruction_data
            .as_ref()
            .expect("transferChecked is mapped");
        assert_eq!(transfer.instruction_name, "transferChecked");
        assert_eq!(transfer.discriminator, "0c");
        assert_eq!(transfer.named_accounts["source"], source);
        assert_eq!(transfer.named_accounts["authority"], owner);
        let args: Value = serde_json::from_str(&transfer.program_call_args_json).unwrap();
        assert_eq!(
            args["source"],
            source.as_str(),
            "accounts stay in the args too"
        );
        assert_eq!(args["tokenAmount"]["amount"], "20000");
        assert!(instructions[1].solana_json_parse_error.is_none());
        assert!(instructions[1].instruction_data_hex.is_empty());
        assert!(instructions[1].solana_rpc_parsed_data.is_some());

        let mint_to = instructions[2]
            .parsed_instruction_data
            .as_ref()
            .expect("mintTo is mapped");
        assert_eq!(mint_to.instruction_name, "mintTo");
        assert_eq!(mint_to.discriminator, "07");

        // ATA `create` keeps `00` whether or not the (hidden) data carried it.
        let create = instructions[3]
            .parsed_instruction_data
            .as_ref()
            .expect("create is mapped");
        assert_eq!(create.discriminator, "00");
        assert_eq!(create.named_accounts["wallet"], owner);
        assert!(instructions[3].solana_rpc_parsed_data.is_some());
        assert!(instructions[3].solana_json_parse_error.is_none());
    }

    #[test]
    fn native_spl_token_transfer_checked_is_decoded() {
        let [source, mint, destination, authority] =
            std::array::from_fn(|_| Pubkey::new_unique().to_string());
        let mut data = vec![12];
        data.extend_from_slice(&1_500_000u64.to_le_bytes());
        data.push(6);
        let instruction = static_instruction(
            &spl_token::id().to_string(),
            &[&source, &mint, &destination, &authority],
            &data,
        );

        let decoded =
            run_native_decoder(&instruction, None, &data).expect("transferChecked decodes");

        assert_eq!(decoded.program, "spl-token");
        let parsed = decoded.parsed;
        assert_eq!(parsed["type"], "transferChecked");
        assert_eq!(parsed["info"]["source"], source.as_str());
        assert_eq!(parsed["info"]["mint"], mint.as_str());
        assert_eq!(parsed["info"]["destination"], destination.as_str());
        assert_eq!(parsed["info"]["authority"], authority.as_str());
        assert_eq!(parsed["info"]["tokenAmount"]["amount"], "1500000");
        assert_eq!(parsed["info"]["tokenAmount"]["decimals"], 6);

        let io = build_intermediate_instruction(&instruction, None, &BTreeMap::new());
        let mapped = io
            .parsed_instruction_data
            .expect("transferChecked is mapped");
        assert_eq!(mapped.discriminator, "0c");
        assert_eq!(
            mapped.named_accounts,
            BTreeMap::from([
                ("authority".to_string(), authority),
                ("destination".to_string(), destination),
                ("mint".to_string(), mint),
                ("source".to_string(), source),
            ])
        );
        let args: Value = serde_json::from_str(&mapped.program_call_args_json).unwrap();
        assert_eq!(args["tokenAmount"]["amount"], "1500000");
        assert_eq!(args["authority"], mapped.named_accounts["authority"]);
    }

    #[test]
    fn native_memo_decodes_to_a_bare_string() {
        let instruction =
            static_instruction("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", &[], b"hello");

        let io = build_intermediate_instruction(&instruction, None, &BTreeMap::new());

        let decoded = io.solana_json_parsed_data.expect("memo decodes");
        assert_eq!(decoded.program, "spl-memo");
        assert_eq!(decoded.parsed_json, r#""hello""#);
        let mapped = io.parsed_instruction_data.expect("memo is mapped");
        assert_eq!(mapped.instruction_name, "memo");
        assert_eq!(mapped.discriminator, "", "memo has no discriminator");
        assert!(mapped.named_accounts.is_empty());
        assert_eq!(mapped.program_call_args_json, r#"{"memo":"hello"}"#);
        assert!(io.solana_json_parse_error.is_none());
    }

    #[test]
    fn unsupported_native_program_reports_it_and_non_native_is_untouched() {
        let stake_pool =
            static_instruction("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy", &[], &[0x0e]);
        let io = build_intermediate_instruction(&stake_pool, None, &BTreeMap::new());
        assert_eq!(io.registered_source, RegisteredSource::Native);
        assert!(io.solana_json_parsed_data.is_none());
        assert!(io.parsed_instruction_data.is_none());
        assert_eq!(
            io.solana_json_parse_error.as_deref(),
            Some(UNSUPPORTED_NATIVE_PROGRAM)
        );

        // Solana's decoder skips Compute Budget; this crate's covers it.
        let compute_budget = static_instruction(
            "ComputeBudget111111111111111111111111111111",
            &[],
            &[0x02, 0x40, 0x0d, 0x03, 0x00],
        );
        let io = build_intermediate_instruction(&compute_budget, None, &BTreeMap::new());
        assert_eq!(
            io.solana_json_parsed_data,
            Some(SolanaJsonParsedInstructionDataIo {
                program: "compute-budget".to_string(),
                parsed_json: r#"{"info":{"units":200000},"type":"setComputeUnitLimit"}"#
                    .to_string(),
            })
        );
        let mapped = io.parsed_instruction_data.expect("decoded by this crate");
        assert_eq!(mapped.instruction_name, "setComputeUnitLimit");
        assert_eq!(mapped.discriminator, "02");
        assert!(mapped.named_accounts.is_empty());
        assert_eq!(mapped.program_call_args_json, r#"{"units":200000}"#);
        assert!(io.solana_json_parse_error.is_none());

        // A well-formed System transfer under a program that is not `Native`
        // never reaches the decoder.
        let source = Pubkey::new_unique().to_string();
        let destination = Pubkey::new_unique().to_string();
        let dapp = static_instruction(
            &Pubkey::new_unique().to_string(),
            &[&source, &destination],
            &[0x02, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        );
        let io = build_intermediate_instruction(&dapp, None, &BTreeMap::new());
        assert_eq!(io.registered_source, RegisteredSource::Unregistered);
        assert!(io.solana_json_parsed_data.is_none());
        assert!(io.parsed_instruction_data.is_none());
        assert!(io.solana_json_parse_error.is_none());
    }

    /// Asks the real decoder about every `Native` program: the ones it supports
    /// must be exactly `SOLANA_JSON_PARSED_PROGRAMS`. `parse` looks the program
    /// up before reading the instruction, so an empty probe answers that alone.
    #[test]
    fn solana_json_parsed_programs_match_the_decoder() {
        use solana_transaction_status::parse_instruction::{ParseInstructionError, parse};

        let supported_by_decoder: Vec<&str> = crate::idl::builtin_programs::NATIVE_PROGRAM_NAMES
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| {
                let program_id = Pubkey::from_str(id).unwrap();
                let probe = CompiledInstruction {
                    program_id_index: 0,
                    accounts: vec![],
                    data: vec![],
                };
                let keys = AccountKeys::new(std::slice::from_ref(&program_id), None);
                !matches!(
                    parse(&program_id, &probe, &keys, None),
                    Err(ParseInstructionError::ProgramNotParsable)
                )
            })
            .collect();

        let mut listed = SOLANA_JSON_PARSED_PROGRAMS.to_vec();
        let mut supported = supported_by_decoder;
        listed.sort_unstable();
        supported.sort_unstable();
        assert_eq!(
            listed, supported,
            "SOLANA_JSON_PARSED_PROGRAMS is out of step with the jsonParsed decoder"
        );
    }

    #[test]
    fn covered_program_that_cannot_be_decoded_reports_why() {
        let account = Pubkey::new_unique().to_string();

        // The System decoder rejects unknown instruction data.
        let garbage = static_instruction("11111111111111111111111111111111", &[&account], &[0xff]);
        let io = build_intermediate_instruction(&garbage, None, &BTreeMap::new());
        assert!(io.solana_json_parsed_data.is_none());
        assert!(io.parsed_instruction_data.is_none());
        assert_eq!(
            io.solana_json_parse_error.as_deref(),
            Some("System instruction not parsable")
        );

        // A transfer with one account, where System expects two.
        let transfer = [0x02, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
        let short = static_instruction("11111111111111111111111111111111", &[&account], &transfer);
        assert_eq!(
            run_native_decoder(&short, None, &transfer).unwrap_err(),
            "System instruction key mismatch"
        );

        // Without the compiled message, an account read through a lookup
        // table can't be positioned.
        let mut via_alt = static_instruction(
            "11111111111111111111111111111111",
            &[&account, &account],
            &[0x02, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        );
        via_alt
            .address_table_lookups
            .push(parser::SolanaSingleAddressTableLookup {
                address_table_key: Pubkey::new_unique().to_string(),
                index: 0,
                writable: true,
            });
        let io = build_intermediate_instruction(&via_alt, None, &BTreeMap::new());
        assert!(io.solana_json_parsed_data.is_none());
        assert!(io.parsed_instruction_data.is_none());
        assert_eq!(
            io.solana_json_parse_error.as_deref(),
            Some("instruction reads an account through an address lookup table")
        );
    }

    /// A key passed twice indexes its first position; a named account whose
    /// key is not in the list (a lookup-table account) gets no index.
    #[test]
    fn named_accounts_index_the_first_position_of_their_key() {
        let parsed = SolanaParsedInstructionDataIo {
            instruction_name: "create".to_string(),
            discriminator: "00".to_string(),
            named_accounts: BTreeMap::from([
                ("source".to_string(), "K".to_string()),
                ("account".to_string(), "A".to_string()),
                ("wallet".to_string(), "K".to_string()),
                ("mint".to_string(), ADDRESS_TABLE_LOOKUP.to_string()),
            ]),
            program_call_args_json: "{}".to_string(),
            idl_source: String::new(),
            idl_hash: String::new(),
        };
        let indices = index_named_accounts(Some(&parsed), ["K", "A", "K"].into_iter());
        assert_eq!(
            indices,
            BTreeMap::from([
                ("account".to_string(), 1),
                ("source".to_string(), 0),
                ("wallet".to_string(), 0),
            ])
        );
        assert!(index_named_accounts(None, ["K"].into_iter()).is_empty());
    }

    /// Only values that differ between the two decodes are masked, so a static
    /// key or a data-encoded pubkey equal to a placeholder is left alone.
    #[test]
    fn masking_touches_only_lookup_backed_values() {
        let mut value = json!({
            "info": { "destination": "P0", "source": "S", "signers": ["S", "P1"], "owner": "P0" },
            "type": "transfer"
        });
        let shadow = json!({
            "info": { "destination": "Q0", "source": "S", "signers": ["S", "Q1"], "owner": "P0" },
            "type": "transfer"
        });
        mask_differences(&mut value, &shadow);
        assert_eq!(
            value,
            json!({
                "info": {
                    "destination": ADDRESS_TABLE_LOOKUP,
                    "source": "S",
                    "signers": ["S", ADDRESS_TABLE_LOOKUP],
                    "owner": "P0"
                },
                "type": "transfer"
            })
        );
    }

    /// A v0 System transfer to a lookup-table account: the compiled message
    /// positions it, and it is named `ADDRESS_TABLE_LOOKUP` as on the IDL path.
    #[test]
    fn alt_backed_native_instruction_is_decoded_from_the_compiled_message() {
        let payer = Pubkey::new_unique();
        let table = Pubkey::new_unique();
        let mut transfer = vec![2, 0, 0, 0];
        transfer.extend_from_slice(&1001u64.to_le_bytes());
        let message = VersionedMessage::V0(v0::Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![
                payer,
                Pubkey::from_str("11111111111111111111111111111111").unwrap(),
            ],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            address_table_lookups: vec![v0::MessageAddressTableLookup {
                account_key: table,
                writable_indexes: vec![7],
                readonly_indexes: vec![],
            }],
            instructions: vec![CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0, 2],
                data: transfer,
            }],
        });
        let message_hex = hex::encode(message.serialize());

        let output = extract_solana_intermediate_output(&message_hex, false, &IdlRegistry::new())
            .expect("solana_parser accepts the message");
        let instruction = &output.instructions[0];
        assert_eq!(instruction.address_table_lookups.len(), 1);
        assert!(instruction.solana_json_parse_error.is_none());
        let mapped = instruction
            .parsed_instruction_data
            .as_ref()
            .expect("decoded through the compiled message");
        assert_eq!(mapped.discriminator, "02000000");
        assert_eq!(
            mapped.named_accounts,
            BTreeMap::from([
                ("destination".to_string(), ADDRESS_TABLE_LOOKUP.to_string()),
                ("source".to_string(), payer.to_string()),
            ])
        );
        assert_eq!(
            mapped.program_call_args_json,
            format!(
                r#"{{"destination":"ADDRESS_TABLE_LOOKUP","lamports":1001,"source":"{payer}"}}"#
            )
        );
        assert_eq!(
            instruction.named_account_indices,
            BTreeMap::from([("source".to_string(), 0)]),
            "a lookup-table account is not in `accounts`, so it has no index"
        );
        let json_parsed = instruction.solana_json_parsed_data.as_ref().unwrap();
        assert!(
            json_parsed.parsed_json.contains(ADDRESS_TABLE_LOOKUP),
            "the placeholder is masked in the jsonParsed decode too: {}",
            json_parsed.parsed_json
        );
        assert_eq!(
            output.transfers[0].to, ADDRESS_TABLE_LOOKUP,
            "solana_parser's own view of the lookup agrees"
        );
    }

    /// The mainnet transaction behind the `deposit_usdc` fixture: two Compute
    /// Budget instructions, then a Jupiter Lend Earn deposit, as a v0 message.
    #[test]
    fn real_transaction_native_outer_instructions_are_decoded() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/native_programs/jupiter_earn_deposit_tx.json"
        ))
        .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(fixture["transaction_base64"].as_str().unwrap())
            .unwrap();
        let transaction: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
        let message_hex = hex::encode(transaction.message.serialize());

        let output = extract_solana_intermediate_output(&message_hex, false, &IdlRegistry::new())
            .expect("solana_parser accepts the message");
        assert_eq!(output.instructions.len(), 3);

        let limit = output.instructions[0]
            .parsed_instruction_data
            .as_ref()
            .expect("setComputeUnitLimit is mapped");
        assert_eq!(limit.instruction_name, "setComputeUnitLimit");
        assert_eq!(limit.discriminator, "02");
        assert_eq!(limit.program_call_args_json, r#"{"units":300000}"#);

        let price = output.instructions[1]
            .parsed_instruction_data
            .as_ref()
            .expect("setComputeUnitPrice is mapped");
        assert_eq!(price.instruction_name, "setComputeUnitPrice");
        assert_eq!(price.discriminator, "03");
        assert_eq!(price.program_call_args_json, r#"{"microLamports":50000}"#);

        for instruction in &output.instructions[..2] {
            assert_eq!(instruction.registered_source, RegisteredSource::Native);
            assert!(instruction.solana_json_parse_error.is_none());
            assert_eq!(
                instruction
                    .solana_json_parsed_data
                    .as_ref()
                    .unwrap()
                    .program,
                "compute-budget"
            );
        }

        for instruction in &output.instructions {
            for (name, index) in &instruction.named_account_indices {
                let key = &instruction.accounts[*index as usize].account_key;
                let named = instruction
                    .parsed_instruction_data
                    .as_ref()
                    .and_then(|parsed| parsed.named_accounts.get(name));
                assert_eq!(named, Some(key), "{name} indexes its own key");
            }
        }

        let deposit = &output.instructions[2];
        assert_eq!(
            deposit.program_key,
            "jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9"
        );
        assert_eq!(
            deposit.parsed_instruction_data.is_some(),
            !deposit.named_account_indices.is_empty(),
            "an IDL decode names accounts, so it indexes them: {:?}",
            deposit.parsed_instruction_data
        );
        assert_eq!(deposit.registered_source, RegisteredSource::Preset);
        assert!(deposit.solana_json_parsed_data.is_none());
        assert!(
            deposit.solana_json_parse_error.is_none(),
            "not native, so nothing to explain"
        );
    }

    /// One transferChecked yields identical `parsed_instruction_data` as a
    /// top-level, a partially-decoded inner, and an RPC-parsed inner instruction.
    #[test]
    fn native_instruction_decodes_identically_outer_and_inner() {
        let [source, mint, destination, authority] = std::array::from_fn(|_| Pubkey::new_unique());
        let token = spl_token::id();
        let mut data = vec![12];
        data.extend_from_slice(&1_500_000u64.to_le_bytes());
        data.push(6);
        let accounts: Vec<String> = [source, mint, destination, authority]
            .iter()
            .map(Pubkey::to_string)
            .collect();
        let account_refs: Vec<&str> = accounts.iter().map(String::as_str).collect();

        let outer_instruction = build_intermediate_instruction(
            &static_instruction(&token.to_string(), &account_refs, &data),
            None,
            &BTreeMap::new(),
        );
        let expected_indices = BTreeMap::from([
            ("authority".to_string(), 3),
            ("destination".to_string(), 2),
            ("mint".to_string(), 1),
            ("source".to_string(), 0),
        ]);
        assert_eq!(outer_instruction.named_account_indices, expected_indices);
        let outer = outer_instruction
            .parsed_instruction_data
            .expect("outer decodes");

        // The RPC-parsed form is the decoder's own output, as
        // simulateTransaction returns it.
        let rpc_parsed = decode_rebuilt(&token, &accounts, data.clone()).expect("decodes");
        let raw_json = serde_json::to_vec(&json!({
            "context": { "slot": 1 },
            "value": {
                "err": null,
                "logs": [],
                "innerInstructions": [{
                    "index": 0,
                    "instructions": [
                        {
                            "programId": token.to_string(),
                            "accounts": accounts,
                            "data": bs58::encode(&data).into_string(),
                            "stackHeight": 2
                        },
                        {
                            "program": rpc_parsed.program,
                            "programId": token.to_string(),
                            "parsed": rpc_parsed.parsed,
                            "stackHeight": 2
                        }
                    ]
                }]
            }
        }))
        .unwrap();
        let (inner, error) =
            parse_and_decode_simulated_instructions(&raw_json, &IdlRegistry::new());
        assert!(error.is_none());
        assert_eq!(inner.len(), 2);
        assert!(inner[0].solana_rpc_parsed_data.is_none());
        assert_eq!(inner[0].named_account_indices, expected_indices);
        assert!(inner[1].solana_rpc_parsed_data.is_some());
        assert!(
            inner[1].named_account_indices.is_empty(),
            "the RPC returns no account list to index into"
        );
        for instruction in &inner {
            assert_eq!(instruction.registered_source, RegisteredSource::Native);
            assert!(instruction.solana_json_parse_error.is_none());
            assert_eq!(instruction.parsed_instruction_data.as_ref(), Some(&outer));
        }

        assert_eq!(outer.discriminator, "0c");
        assert_eq!(
            outer.named_accounts,
            BTreeMap::from([
                ("authority".to_string(), authority.to_string()),
                ("destination".to_string(), destination.to_string()),
                ("mint".to_string(), mint.to_string()),
                ("source".to_string(), source.to_string()),
            ])
        );
        let args: Value = serde_json::from_str(&outer.program_call_args_json).unwrap();
        assert_eq!(args["tokenAmount"]["amount"], "1500000");
        assert_eq!(args["destination"], destination.to_string());
    }

    /// Partially-decoded inner instructions of native programs go through the
    /// same decoder as top-level ones, and each failure is marked.
    #[test]
    fn partially_decoded_native_inner_instructions_are_decoded_or_explained() {
        let account = Pubkey::new_unique().to_string();
        let raw_json = serde_json::to_vec(&json!({
            "context": { "slot": 1 },
            "value": {
                "err": null,
                "logs": [],
                "innerInstructions": [{
                    "index": 0,
                    "instructions": [
                        {
                            "programId": "ComputeBudget111111111111111111111111111111",
                            "accounts": [],
                            "data": bs58::encode([2u8, 0x40, 0x0d, 0x03, 0x00]).into_string(),
                            "stackHeight": 2
                        },
                        {
                            "programId": "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
                            "accounts": [],
                            "data": bs58::encode([0x0eu8]).into_string(),
                            "stackHeight": 2
                        },
                        {
                            "programId": "11111111111111111111111111111111",
                            "accounts": [account],
                            "data": bs58::encode([2u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]).into_string(),
                            "stackHeight": 2
                        }
                    ]
                }]
            }
        }))
        .unwrap();

        let (inner, error) =
            parse_and_decode_simulated_instructions(&raw_json, &IdlRegistry::new());
        assert!(error.is_none());
        assert_eq!(inner.len(), 3);

        let compute_budget = inner[0]
            .parsed_instruction_data
            .as_ref()
            .expect("decoded by this crate");
        assert_eq!(compute_budget.instruction_name, "setComputeUnitLimit");
        assert_eq!(compute_budget.discriminator, "02");
        assert_eq!(compute_budget.program_call_args_json, r#"{"units":200000}"#);
        assert!(inner[0].solana_json_parse_error.is_none());

        assert!(inner[1].parsed_instruction_data.is_none());
        assert_eq!(
            inner[1].solana_json_parse_error.as_deref(),
            Some(UNSUPPORTED_NATIVE_PROGRAM)
        );

        assert!(inner[2].parsed_instruction_data.is_none());
        assert!(inner[2].idl_parse_error.is_none());
        assert_eq!(
            inner[2].solana_json_parse_error.as_deref(),
            Some("System instruction key mismatch")
        );

        // The marker round-trips through the published schema.
        let io = SolanaIntermediateOutput {
            schema_version: SOLANA_INTERMEDIATE_SCHEMA_VERSION,
            account_keys: vec![],
            program_keys: vec![],
            instructions: vec![],
            transfers: vec![],
            spl_transfers: vec![],
            recent_blockhash: "blockhash".to_string(),
            address_table_lookups: vec![],
            simulated_instructions: inner,
            simulation_error: None,
        };
        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: SolanaIntermediateOutput =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
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
        let io = build_intermediate_output(&metadata, None, &BTreeMap::new());
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
