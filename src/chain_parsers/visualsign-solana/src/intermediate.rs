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
//!   `BorshSerialize`. Keys are alphabetized.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde_json::Value;
use solana_parser::solana::structs::{
    self as parser, IdlSource, SolanaMetadata, SolanaParsedInstructionData,
};
use solana_parser::{CustomIdlConfig, parse_transaction_with_idls};
use visualsign::errors::VisualSignError;
use visualsign::vsptrait::TransactionParseError;

use crate::idl::IdlRegistry;

/// Version of the `SolanaIntermediateOutput` Borsh schema. Bump on ANY change
/// to the shape below. Mirrored decoders assert this value, so a bump makes a
/// schema drift fail loudly instead of silently misparsing.
pub const SOLANA_INTERMEDIATE_SCHEMA_VERSION: u16 = 1;

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
    /// Canonical JSON string with alphabetized keys. Built from a `BTreeMap`
    /// view so byte-identical inputs produce byte-identical encodings.
    pub program_call_args_json: String,
    /// `"BuiltIn"` (with the inner program-type discriminant collapsed) or
    /// `"Custom"`. Empty when no IDL was used.
    pub idl_source: String,
    pub idl_hash: String,
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
    // Re-key into a BTreeMap so JSON output is alphabetized regardless of
    // upstream insertion order. We hold references to avoid cloning Values.
    let ordered: BTreeMap<&String, &Value> = args.iter().collect();
    // serde_json::to_string never fails on a Map<String, Value>; on the
    // off-chance it does we fall back to an empty object so the surrounding
    // borsh encoding stays well-formed.
    serde_json::to_string(&ordered).unwrap_or_else(|_| "{}".to_string())
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
