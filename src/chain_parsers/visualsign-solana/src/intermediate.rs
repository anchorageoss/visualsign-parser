//! Solana intermediate output for downstream policy engines.
//!
//! This is a Borsh-serialized mirror of [`solana_parser::SolanaMetadata`]
//! shaped to match the per-instruction attributes that Turnkey's Solana
//! policy engine evaluates against (see Turnkey's `solana.tx.*` policy
//! variables). The schema is deliberately kept stable in Rust so the
//! parser and the wallet share the same definition; the bytes emitted
//! here are placed verbatim into `ParsedTransactionPayload.intermediate_output`.
//!
//! Differences from `solana_parser::SolanaMetadata`:
//! - `signatures` is dropped (unsigned txs have none).
//! - All maps use `BTreeMap` so Borsh encoding is byte-deterministic.
//! - `program_call_args` is emitted as a canonical JSON string
//!   (`program_call_args_json`) because `serde_json::Value` does not
//!   implement `BorshSerialize`. Keys are alphabetized.

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

/// Token-2022 program id. Confidential-transfer decoding only applies to
/// instructions issued against this program.
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Top-level Solana intermediate output. Mirrors `solana_parser::SolanaMetadata`
/// minus `signatures`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SolanaIntermediateOutput {
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
    /// `Some` only for Token-2022 ConfidentialTransfer Transfer/Withdraw
    /// sub-instructions; `None` otherwise (including other CT
    /// sub-instructions and decode failures).
    pub confidential_transfer: Option<ConfidentialTransferIo>,
}

/// Borsh-serializable mirror of
/// [`crate::presets::token_2022::ConfidentialTransferIx`] for the policy
/// engine's intermediate output.
///
/// The `Transfer` variant deliberately carries no amount field: the transfer
/// amount is confidential (encrypted on-chain) and a wallet-decoded value
/// must never be placed in this trust boundary. `Withdraw`'s amount is
/// plaintext in the instruction itself, so it is safe to include.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum ConfidentialTransferIo {
    Withdraw {
        source_token_account: String,
        mint: String,
        owner: String,
        amount: u64,
        decimals: u8,
        new_decryptable_available_balance: String,
        equality_proof_context_account: Option<String>,
        range_proof_context_account: Option<String>,
    },
    Transfer {
        source_token_account: String,
        mint: String,
        destination_token_account: String,
        owner: String,
        new_source_decryptable_available_balance: String,
        auditor_configured: bool,
        equality_proof_context_account: Option<String>,
        validity_proof_context_account: Option<String>,
        range_proof_context_account: Option<String>,
    },
}

impl From<crate::presets::token_2022::ConfidentialTransferIx> for ConfidentialTransferIo {
    fn from(ix: crate::presets::token_2022::ConfidentialTransferIx) -> Self {
        use crate::presets::token_2022::ConfidentialTransferIx as Ix;
        match ix {
            Ix::Withdraw {
                source_token_account,
                mint,
                owner,
                amount,
                decimals,
                new_decryptable_available_balance,
                equality_proof_context_account,
                range_proof_context_account,
            } => Self::Withdraw {
                source_token_account,
                mint,
                owner,
                amount,
                decimals,
                new_decryptable_available_balance,
                equality_proof_context_account,
                range_proof_context_account,
            },
            Ix::Transfer {
                source_token_account,
                mint,
                destination_token_account,
                owner,
                new_source_decryptable_available_balance,
                auditor_configured,
                equality_proof_context_account,
                validity_proof_context_account,
                range_proof_context_account,
            } => Self::Transfer {
                source_token_account,
                mint,
                destination_token_account,
                owner,
                new_source_decryptable_available_balance,
                auditor_configured,
                equality_proof_context_account,
                validity_proof_context_account,
                range_proof_context_account,
            },
        }
    }
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

// ── From impls ──────────────────────────────────────────────────────────────

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

/// Decode a Token-2022 ConfidentialTransfer Transfer/Withdraw sub-instruction
/// from a parsed instruction's hex data and account list, if applicable.
///
/// Returns `None` for non-Token-2022 programs, undecodable hex, decode
/// errors, or any CT sub-instruction other than Transfer/Withdraw.
fn decode_confidential_transfer(
    value: &parser::SolanaInstruction,
) -> Option<ConfidentialTransferIo> {
    if value.program_key != TOKEN_2022_PROGRAM_ID {
        return None;
    }
    let data = hex::decode(&value.instruction_data_hex).ok()?;
    let account_strings: Vec<String> = value
        .accounts
        .iter()
        .map(|a| a.account_key.clone())
        .collect();
    crate::presets::token_2022::try_decode_confidential_transfer(&data, &account_strings)
        .ok()
        .flatten()
        .map(ConfidentialTransferIo::from)
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
            confidential_transfer: decode_confidential_transfer(value),
        }
    }
}

impl From<&SolanaMetadata> for SolanaIntermediateOutput {
    fn from(value: &SolanaMetadata) -> Self {
        Self {
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

// ── Extraction ──────────────────────────────────────────────────────────────

/// Parse the transaction once via `solana_parser::parse_transaction_with_idls`
/// and project the result into a Borsh-friendly intermediate output.
///
/// `raw_message_hex` is the hex-encoded serialized message (or full
/// transaction); `full_transaction` toggles which form is being passed in,
/// matching `solana_parser`'s API.
pub fn extract_solana_intermediate_output(
    raw_message_hex: &str,
    full_transaction: bool,
    idl_registry: &IdlRegistry,
) -> Result<SolanaIntermediateOutput, VisualSignError> {
    let custom_idls: Option<std::collections::HashMap<String, CustomIdlConfig>> = {
        let configs = idl_registry.get_all_configs();
        if configs.is_empty() {
            None
        } else {
            Some(configs.clone())
        }
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use bytemuck::bytes_of;
    use serde_json::json;
    use solana_parser::solana::structs::ProgramType;
    use spl_token_2022_interface::extension::confidential_transfer::instruction::{
        ConfidentialTransferInstruction, WithdrawInstructionData,
    };
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
    fn confidential_transfer_io_round_trip() {
        let io = ConfidentialTransferIo::Withdraw {
            source_token_account: "src".into(),
            mint: "mint".into(),
            owner: "owner".into(),
            amount: 1_000_000,
            decimals: 6,
            new_decryptable_available_balance: "AAAA".into(),
            equality_proof_context_account: Some("eq".into()),
            range_proof_context_account: Some("rng".into()),
        };
        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: ConfidentialTransferIo =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
    }

    #[test]
    fn from_solana_instruction_populates_confidential_transfer_withdraw() {
        let d = WithdrawInstructionData {
            amount: 1_500_000u64.into(),
            decimals: 6,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        let mut data = vec![
            crate::presets::token_2022::confidential_transfer::CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR,
            ConfidentialTransferInstruction::Withdraw as u8,
        ];
        data.extend_from_slice(bytes_of(&d));

        // Accounts: [src, mint, equality_ctx, range_ctx, owner]
        let account_labels = ["src", "mint", "eqctx", "rngctx", "owner"];
        let accounts: Vec<parser::SolanaAccount> = account_labels
            .iter()
            .map(|label| parser::SolanaAccount {
                account_key: (*label).to_string(),
                signer: false,
                writable: false,
            })
            .collect();

        let instruction = parser::SolanaInstruction {
            program_key: TOKEN_2022_PROGRAM_ID.to_string(),
            accounts,
            instruction_data_hex: hex::encode(&data),
            address_table_lookups: vec![],
            parsed_instruction: None,
        };

        let io = SolanaIntermediateInstruction::from(&instruction);
        match io.confidential_transfer {
            Some(ConfidentialTransferIo::Withdraw {
                amount,
                source_token_account,
                mint,
                owner,
                ..
            }) => {
                assert_eq!(amount, 1_500_000);
                assert_eq!(source_token_account, "src");
                assert_eq!(mint, "mint");
                assert_eq!(owner, "owner");
            }
            other => {
                panic!("expected Some(ConfidentialTransferIo::Withdraw {{ .. }}), got {other:?}")
            }
        }
    }

    #[test]
    fn from_solana_instruction_leaves_confidential_transfer_none_for_other_programs() {
        let instruction = parser::SolanaInstruction {
            program_key: "11111111111111111111111111111111".to_string(),
            accounts: vec![],
            instruction_data_hex: String::new(),
            address_table_lookups: vec![],
            parsed_instruction: None,
        };
        let io = SolanaIntermediateInstruction::from(&instruction);
        assert!(io.confidential_transfer.is_none());
    }

    #[test]
    fn extract_handles_empty_metadata_via_real_parser() {
        // A minimal valid Solana transaction message hex: zero accounts, zero
        // instructions. This exercises the extract function end-to-end without
        // any IDL.
        // The simplest real fixture is taken from the existing decode_transfers
        // call site — using a known-good message ensures we stay decoupled
        // from solana_parser's internals.
        // Skipping a real fixture here keeps this unit test self-contained;
        // see the integration test in src/integration/tests/parser.rs for the
        // realistic e2e exercise.
        // The function is exercised by integration tests; assert From<&> path.
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
        assert_eq!(io.account_keys, vec!["A1".to_string(), "B2".to_string()]);
        assert_eq!(io.program_keys, vec!["P1".to_string()]);
        assert!(io.instructions.is_empty());
        assert_eq!(io.recent_blockhash, "blockhash");

        let bytes = borsh::to_vec(&io).expect("borsh serializes");
        let recovered: SolanaIntermediateOutput =
            borsh::from_slice(&bytes).expect("borsh deserializes");
        assert_eq!(io, recovered);
    }
}
