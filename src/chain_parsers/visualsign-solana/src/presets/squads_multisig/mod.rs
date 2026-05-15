//! Squads v4 Multisig preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    available_visualizers, visualize_with_any,
};
use config::SquadsMultisigConfig;
use solana_parser::solana::structs::SolanaAccount;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const SQUADS_MULTISIG_PROGRAM_ID: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";

const SQUADS_IDL_JSON: &str = include_str!("squads_multisig_program.json");

static SQUADS_MULTISIG_CONFIG: SquadsMultisigConfig = SquadsMultisigConfig;

// -- VaultTransactionMessage uses Solana's compact wire format (u8 lengths), not borsh --

struct VaultTransactionMessage {
    /// Total number of signer accounts (the first `num_signers` entries of `account_keys`).
    num_signers: u8,
    /// Number of writable signer accounts (the first `num_writable_signers`
    /// entries of `account_keys`).
    num_writable_signers: u8,
    /// Number of writable non-signer accounts (immediately following the signer
    /// block in `account_keys`).
    num_writable_non_signers: u8,
    account_keys: Vec<Pubkey>,
    instructions: Vec<MultisigCompiledInstruction>,
}

struct MultisigCompiledInstruction {
    program_id_index: u8,
    account_indexes: Vec<u8>,
    data: Vec<u8>,
}

impl VaultTransactionMessage {
    /// Parse from Squads' compact wire format (mimics Solana Message serialization).
    /// Format: 3×u8 header, u8 account_keys count + pubkeys,
    ///         u8 instructions count + compiled instructions,
    ///         u8 address_table_lookups count + lookups.
    /// Within compiled instructions, data length is encoded as u16 LE
    /// (matching the Squads v4 on-chain format).
    fn deserialize(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut pos = 0;

        let read_u8 = |pos: &mut usize| -> Result<u8, Box<dyn std::error::Error>> {
            if *pos >= data.len() {
                return Err("unexpected end of data".into());
            }
            let val = data[*pos];
            *pos += 1;
            Ok(val)
        };

        let read_u16_le = |pos: &mut usize| -> Result<u16, Box<dyn std::error::Error>> {
            if *pos + 2 > data.len() {
                return Err("unexpected end of data".into());
            }
            let val = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
            *pos += 2;
            Ok(val)
        };

        let read_bytes =
            |pos: &mut usize, len: usize| -> Result<&[u8], Box<dyn std::error::Error>> {
                if *pos + len > data.len() {
                    return Err("unexpected end of data".into());
                }
                let slice = &data[*pos..*pos + len];
                *pos += len;
                Ok(slice)
            };

        // Header: 3 u8s (numSigners, numWritableSigners, numWritableNonSigners)
        let num_signers = read_u8(&mut pos)?;
        let num_writable_signers = read_u8(&mut pos)?;
        let num_writable_non_signers = read_u8(&mut pos)?;

        // Account keys: u8 count + N × 32-byte pubkeys
        let num_keys = read_u8(&mut pos)? as usize;
        let mut account_keys = Vec::with_capacity(num_keys);
        for _ in 0..num_keys {
            let key_bytes = read_bytes(&mut pos, 32)?;
            account_keys.push(Pubkey::new_from_array(key_bytes.try_into()?));
        }

        // Instructions: u8 count + N × compiled instructions
        let num_instructions = read_u8(&mut pos)? as usize;
        let mut instructions = Vec::with_capacity(num_instructions);
        for _ in 0..num_instructions {
            let program_id_index = read_u8(&mut pos)?;
            let num_account_indexes = read_u8(&mut pos)? as usize;
            let account_indexes = read_bytes(&mut pos, num_account_indexes)?.to_vec();
            let data_len = read_u16_le(&mut pos)? as usize;
            let instruction_data = read_bytes(&mut pos, data_len)?.to_vec();
            instructions.push(MultisigCompiledInstruction {
                program_id_index,
                account_indexes,
                data: instruction_data,
            });
        }

        // Address table lookups: u8 count + N × { 32-byte pubkey, u8 count + writable
        // indexes, u8 count + readonly indexes }. We don't need the lookup contents to
        // reconstruct top-level instructions, but we still consume them so a malformed
        // (truncated or padded) message is rejected rather than silently accepted.
        let num_lookups = read_u8(&mut pos)? as usize;
        for _ in 0..num_lookups {
            let _account_key = read_bytes(&mut pos, 32)?;
            let num_writable = read_u8(&mut pos)? as usize;
            let _writable_indexes = read_bytes(&mut pos, num_writable)?;
            let num_readonly = read_u8(&mut pos)? as usize;
            let _readonly_indexes = read_bytes(&mut pos, num_readonly)?;
        }

        if pos != data.len() {
            return Err(format!(
                "trailing bytes after VaultTransactionMessage: pos={pos}, len={}",
                data.len()
            )
            .into());
        }

        Ok(Self {
            num_signers,
            num_writable_signers,
            num_writable_non_signers,
            account_keys,
            instructions,
        })
    }
}

pub struct SquadsMultisigVisualizer;

impl InstructionVisualizer for SquadsMultisigVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let instruction_data_hex = hex::encode(&instruction.data);
        let fallback_text = format!(
            "Program ID: {}\nData: {instruction_data_hex}",
            instruction.program_id,
        );

        let parsed = parse_squads_instruction(&instruction.data, &instruction.accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => {
                build_parsed_fields(&parsed, &instruction.program_id.to_string(), context)?
            }
            Err(_) => build_fallback_fields(&instruction.program_id.to_string())?,
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        let expanded_with_raw =
            append_raw_data(expanded_fields, &instruction.data, &instruction_data_hex)?;
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_with_raw,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {}", context.instruction_index() + 1),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SQUADS_MULTISIG_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("SquadsMultisig")
    }
}

fn get_squads_idl() -> Option<&'static Idl> {
    static IDL: std::sync::LazyLock<Option<Idl>> =
        std::sync::LazyLock::new(|| decode_idl_data(SQUADS_IDL_JSON).ok());
    IDL.as_ref()
}

fn parse_squads_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<SquadsParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_squads_idl().ok_or("Squads Multisig IDL not available")?;
    let parsed = parse_instruction_with_idl(data, SQUADS_MULTISIG_PROGRAM_ID, idl)?;

    let named_accounts = build_named_accounts(data, idl, accounts);

    Ok(SquadsParsedInstruction {
        parsed,
        named_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[AccountMeta],
) -> BTreeMap<String, String> {
    let mut named_accounts = BTreeMap::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.len() >= disc.len() && data[..disc.len()] == *disc)
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_meta) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
            }
        }
    }

    named_accounts
}

struct SquadsParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
}

/// `(title, condensed_fields, expanded_fields)` returned by the various `build_*` helpers.
type SquadsPreviewFields = (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
);

fn build_parsed_fields(
    instruction: &SquadsParsedInstruction,
    program_id: &str,
    context: &VisualizerContext,
) -> Result<SquadsPreviewFields, VisualSignError> {
    let parsed = &instruction.parsed;

    // Special case: decode nested transaction message for vaultTransactionCreate
    if parsed.instruction_name == "vaultTransactionCreate" {
        if let Some(fields) = try_build_vault_transaction_fields(
            parsed,
            &instruction.named_accounts,
            program_id,
            context,
        )? {
            return Ok(fields);
        }
    }

    build_generic_fields(parsed, &instruction.named_accounts, program_id)
}

/// Try to decode the nested transaction message inside vaultTransactionCreate.
///
/// Returns `Ok(None)` when the embedded transaction message is missing, unparseable,
/// references accounts via an Address Lookup Table, or otherwise fails to satisfy the
/// invariants required for safe nested visualization (callers fall through to the
/// generic display, where the user can still see the raw decoded args).
///
/// Returns `Err` only when field-builder or downstream visualization errors occur —
/// those propagate up so the caller can decide whether to surface them.
fn try_build_vault_transaction_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &BTreeMap<String, String>,
    program_id: &str,
    context: &VisualizerContext,
) -> Result<Option<SquadsPreviewFields>, VisualSignError> {
    // Extract transactionMessage hex from the nested args struct.
    // Missing/malformed embedded data is "graceful degradation" — the outer caller falls
    // back to the generic display, NOT an error.
    let Some(args_value) = parsed.program_call_args.get("args") else {
        return Ok(None);
    };
    let Some(tx_msg_hex) = args_value
        .get("transactionMessage")
        .and_then(|v| v.as_str())
    else {
        return Ok(None);
    };
    let Ok(tx_msg_bytes) = hex::decode(tx_msg_hex) else {
        return Ok(None);
    };
    let Ok(vault_msg) = VaultTransactionMessage::deserialize(&tx_msg_bytes) else {
        return Ok(None);
    };

    // Vault index must be present and fit a u8 (Squads supports vault indices 0..=255).
    // Don't default a missing or out-of-range value to 0 — vault 0 is real, so a silent
    // default is indistinguishable from an explicit selection.
    let Some(vault_index_u64) = args_value.get("vaultIndex").and_then(|v| v.as_u64()) else {
        return Ok(None);
    };
    let Ok(vault_index) = u8::try_from(vault_index_u64) else {
        return Ok(None);
    };

    // Reconstruct full Instructions from the compiled instructions. An out-of-range
    // index means an ALT-resolved key the parser doesn't have; fall back to generic
    // display rather than rendering a truncated account list.
    let Ok(inner_instructions) = reconstruct_instructions(&vault_msg) else {
        return Ok(None);
    };

    // Derive the vault PDA so downstream visualizers that compare `context.sender()`
    // attribute the inner instruction's signer to the vault, not the outer fee-payer.
    let inner_sender_account = vault_pda_account(named_accounts, vault_index);

    // Visualize inner instructions using the full visualizer framework, threading
    // `for_nested_call` so we cap at MAX_CALL_DEPTH.
    let inner_fields =
        visualize_inner_instructions(&inner_instructions, context, inner_sender_account.as_ref())?;

    let memo = args_value
        .get("memo")
        .and_then(|v| v.as_str())
        .unwrap_or("None");

    let inner_count = inner_instructions.len();
    let title = format!("Squads Multisig: Vault Transaction ({inner_count} inner instruction(s))");

    // Condensed: program, instruction, vault index, inner instruction count
    let condensed_fields = vec![
        create_text_field("Program", "Squads Multisig")?,
        create_text_field("Instruction", "vaultTransactionCreate")?,
        create_text_field("Vault Index", &vault_index.to_string())?,
        create_text_field(
            "Inner Instructions",
            &format!("{inner_count} instruction(s)"),
        )?,
    ];

    // Expanded: full details + decoded inner instructions
    let mut expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", "vaultTransactionCreate")?,
        create_text_field("Discriminator", &parsed.discriminator)?,
        create_text_field("Vault Index", &vault_index.to_string())?,
        create_text_field("Memo", memo)?,
    ];

    // Named accounts from the outer instruction
    for (account_name, account_address) in named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    // Decoded inner instructions
    expanded_fields.extend(inner_fields);

    Ok(Some((title, condensed_fields, expanded_fields)))
}

/// Derive the Squads v4 vault PDA from the multisig pubkey + vault index.
///
/// Seeds: `[b"multisig", multisig.as_ref(), b"vault", &[vault_index]]`,
/// program = `SQUADS_MULTISIG_PROGRAM_ID`.
///
/// Returns `None` when `multisig` isn't present in `named_accounts`, isn't a parseable
/// pubkey, or when the program-id constant fails to parse (the last is unreachable in
/// practice but we don't panic).
fn vault_pda_account(
    named_accounts: &BTreeMap<String, String>,
    vault_index: u8,
) -> Option<SolanaAccount> {
    let multisig_str = named_accounts.get("multisig")?;
    let multisig = Pubkey::from_str(multisig_str).ok()?;
    let program_id = Pubkey::from_str(SQUADS_MULTISIG_PROGRAM_ID).ok()?;
    let (vault_pda, _bump) = Pubkey::find_program_address(
        &[b"multisig", multisig.as_ref(), b"vault", &[vault_index]],
        &program_id,
    );
    Some(SolanaAccount {
        account_key: vault_pda.to_string(),
        signer: true,
        writable: false,
    })
}

/// Reconstruct Instruction objects from VaultTransactionMessage compiled instructions.
///
/// Returns `Err` if any program-id index or account index references an out-of-range
/// position in `account_keys`. The latter is a real signal that the transaction would
/// resolve missing keys via an Address Lookup Table at execution time; we don't have
/// the ALT contents here, so we refuse to reconstruct rather than silently dropping
/// accounts (which can hide a transfer destination from the user).
///
/// `is_signer` / `is_writable` are derived from the header counts per Solana's
/// `MessageHeader` convention:
///   - `[0, num_writable_signers)` are signer + writable
///   - `[num_writable_signers, num_signers)` are signer + readonly
///   - `[num_signers, num_signers + num_writable_non_signers)` are non-signer + writable
///   - everything else is non-signer + readonly
fn reconstruct_instructions(
    vault_msg: &VaultTransactionMessage,
) -> Result<Vec<Instruction>, &'static str> {
    let account_keys = &vault_msg.account_keys;
    let num_keys = account_keys.len();
    let num_signers = vault_msg.num_signers as usize;
    let num_writable_signers = vault_msg.num_writable_signers as usize;
    let num_writable_non_signers = vault_msg.num_writable_non_signers as usize;

    // Validate the header invariants up-front: a malformed input that claims
    // more writable signers than signers, more signers than keys, or more
    // writable non-signers than non-signer slots would silently mis-label
    // accounts as signers/writable downstream. Refuse instead.
    if num_writable_signers > num_signers {
        return Err("num_writable_signers > num_signers");
    }
    if num_signers > num_keys {
        return Err("num_signers > account_keys.len()");
    }
    if num_writable_non_signers > num_keys - num_signers {
        return Err("num_writable_non_signers > non-signer slots");
    }

    let account_meta_for_index = |idx: usize| -> Option<AccountMeta> {
        let pubkey = *account_keys.get(idx)?;
        let is_signer = idx < num_signers;
        let is_writable = if is_signer {
            idx < num_writable_signers
        } else {
            let non_signer_idx = idx - num_signers;
            non_signer_idx < num_writable_non_signers
        };
        Some(if is_writable {
            AccountMeta::new(pubkey, is_signer)
        } else {
            AccountMeta::new_readonly(pubkey, is_signer)
        })
    };

    vault_msg
        .instructions
        .iter()
        .map(|ci| {
            let program_id_idx = ci.program_id_index as usize;
            if program_id_idx >= num_keys {
                return Err("program_id index out of range (likely an ALT-resolved key)");
            }

            let accounts: Vec<AccountMeta> = ci
                .account_indexes
                .iter()
                .map(|&i| {
                    account_meta_for_index(i as usize).ok_or(
                        "inner-instruction account index out of range \
                         (likely an ALT-resolved key)",
                    )
                })
                .collect::<Result<_, _>>()?;

            Ok(Instruction {
                program_id: account_keys[program_id_idx],
                accounts,
                data: ci.data.clone(),
            })
        })
        .collect()
}

/// Visualize reconstructed inner instructions using the full visualizer framework.
///
/// `visualize_with_any` selects the first visualizer whose `can_handle` returns true and
/// invokes it. If that visualizer's `visualize_tx_commands` then fails — or no visualizer
/// matches — we explicitly fall back to `UnknownProgramVisualizer` (a catch-all). This
/// guarantees every inner instruction produces *some* output: a failure in a specific
/// program's visualizer never silently drops the field.
///
/// Recursion is bounded by `VisualizerContext::for_nested_call`; once `MAX_CALL_DEPTH`
/// is reached, we return a single explicit "max depth exceeded" field rather than
/// recursing further. This protects against unbounded-recursion DoS via cyclically
/// nested `vaultTransactionCreate` payloads.
fn visualize_inner_instructions(
    inner_instructions: &[Instruction],
    context: &VisualizerContext,
    inner_sender_override: Option<&SolanaAccount>,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let owned_sender = inner_sender_override
        .cloned()
        .unwrap_or_else(|| SolanaAccount {
            account_key: context.sender().account_key.clone(),
            signer: false,
            writable: false,
        });
    let instructions_vec: Vec<Instruction> = inner_instructions.to_vec();

    let Some(_probe) = context.for_nested_call(&owned_sender, 0, &instructions_vec) else {
        return Ok(vec![create_text_field(
            "Inner Instructions",
            "<truncated: maximum nested-instruction visualization depth reached>",
        )?]);
    };

    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect();

    let mut fields = Vec::with_capacity(instructions_vec.len());

    for (idx, _) in instructions_vec.iter().enumerate() {
        // Safe to unwrap-equivalent: depth was already checked above (the probe Some-arm
        // means depth + 1 <= MAX_CALL_DEPTH). We re-call so the index advances per inner.
        let Some(inner_context) = context.for_nested_call(&owned_sender, idx, &instructions_vec)
        else {
            // Should be unreachable given the probe above, but keep the safety net.
            fields.push(create_text_field(
                "Inner Instructions",
                "<truncated: maximum nested-instruction visualization depth reached>",
            )?);
            break;
        };

        let field = match visualize_with_any(&visualizers_refs, &inner_context) {
            Some(Ok(result)) => result.field,
            Some(Err(_)) | None => crate::presets::unknown_program::UnknownProgramVisualizer
                .visualize_tx_commands(&inner_context)?,
        };
        fields.push(field);
    }

    Ok(fields)
}

fn build_generic_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &BTreeMap<String, String>,
    program_id: &str,
) -> Result<SquadsPreviewFields, VisualSignError> {
    let title = format!("Squads Multisig: {}", parsed.instruction_name);

    // Condensed: program name + instruction name + key args
    let mut condensed_fields = vec![
        create_text_field("Program", "Squads Multisig")?,
        create_text_field("Instruction", &parsed.instruction_name)?,
    ];
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    // Expanded: full details
    let mut expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    for (account_name, account_address) in named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    Ok((title, condensed_fields, expanded_fields))
}

fn build_fallback_fields(program_id: &str) -> Result<SquadsPreviewFields, VisualSignError> {
    let title = "Squads Multisig: Unknown Instruction".to_string();

    let condensed_fields = vec![
        create_text_field("Program", "Squads Multisig")?,
        create_text_field("Status", "Unknown instruction type")?,
    ];

    let expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Status", "Unknown instruction type")?,
    ];

    Ok((title, condensed_fields, expanded_fields))
}

fn append_raw_data(
    mut fields: Vec<AnnotatedPayloadField>,
    data: &[u8],
    hex_str: &str,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    fields.push(create_raw_data_field(data, Some(hex_str.to_string()))?);
    Ok(fields)
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Test-helper error type. Boxed dyn-error so `?` accepts both `hex::FromHexError` and
    /// the `Box<dyn std::error::Error>` returned by `VaultTransactionMessage::deserialize`.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn test_squads_idl_loads() -> TestResult {
        let idl = get_squads_idl().ok_or("Squads IDL should load successfully")?;
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
        Ok(())
    }

    #[test]
    fn test_squads_idl_has_discriminators() -> TestResult {
        let idl = get_squads_idl().ok_or("Squads IDL should load successfully")?;
        for instruction in &idl.instructions {
            let disc = instruction.discriminator.as_ref().ok_or_else(|| {
                format!(
                    "Instruction '{}' should have a computed discriminator",
                    instruction.name
                )
            })?;
            assert_eq!(
                disc.len(),
                8,
                "Discriminator for '{}' should be 8 bytes",
                instruction.name
            );
        }
        Ok(())
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage_data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let accounts = vec![];
        let result = parse_squads_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_squads_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }

    #[test]
    fn test_vault_transaction_message_deserialization() -> TestResult {
        // The transactionMessage hex from the sample transaction
        let tx_msg_hex = "01010103904fc8953dcfc9f3b5179893ee12fc9c445cad889a957d61cfb8dbcc172f6a4f4a3eef4b03c82a71599ea07a16ee4bcf6dce31357d8460b2ac1bd4c3a9860c9d0954dbbe9ec960c98a7a293fe21336966fe180d151ae4b8179561f89854a53f601020200012800a1b028d53cb8b3e4ef5e27e0961546aad46749acc092e03c1a8c7c1187e887cc245db9cf2bca9a9900";
        let tx_msg_bytes = hex::decode(tx_msg_hex)?;
        let vault_msg = VaultTransactionMessage::deserialize(&tx_msg_bytes)?;

        assert_eq!(
            vault_msg.account_keys.len(),
            3,
            "Should have 3 account keys"
        );
        assert_eq!(
            vault_msg.instructions.len(),
            1,
            "Should have 1 inner instruction"
        );

        let inner = vault_msg
            .instructions
            .first()
            .ok_or("expected at least one instruction")?;
        assert!(
            (inner.program_id_index as usize) < vault_msg.account_keys.len(),
            "program_id_index should be valid"
        );

        let instructions = reconstruct_instructions(&vault_msg)?;
        assert_eq!(instructions.len(), 1, "Should reconstruct 1 instruction");
        let first_inner = instructions
            .first()
            .ok_or("expected at least one reconstructed instruction")?;
        // The inner instruction's program_id should be one of the account keys
        assert!(
            vault_msg.account_keys.contains(&first_inner.program_id),
            "Inner instruction program_id should be in account_keys"
        );
        Ok(())
    }

    fn make_pubkey(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn make_vault_msg(
        num_signers: u8,
        num_writable_signers: u8,
        num_writable_non_signers: u8,
        account_keys: Vec<Pubkey>,
        compiled: Vec<MultisigCompiledInstruction>,
    ) -> VaultTransactionMessage {
        VaultTransactionMessage {
            num_signers,
            num_writable_signers,
            num_writable_non_signers,
            account_keys,
            instructions: compiled,
        }
    }

    #[test]
    fn test_reconstruct_instructions_out_of_range_returns_err() {
        // Three keys, but the inner instruction references account index 99 (ALT-resolved
        // at execution time). We must refuse rather than silently dropping the account.
        let vault_msg = make_vault_msg(
            1,
            1,
            0,
            vec![make_pubkey(1), make_pubkey(2), make_pubkey(3)],
            vec![MultisigCompiledInstruction {
                program_id_index: 2,
                account_indexes: vec![0, 99],
                data: vec![],
            }],
        );
        assert!(
            reconstruct_instructions(&vault_msg).is_err(),
            "out-of-range account index must be rejected"
        );
    }

    #[test]
    fn test_reconstruct_instructions_rejects_inconsistent_header() {
        // num_writable_signers > num_signers — would mis-label non-signers as
        // writable. Refuse.
        let too_many_writable_signers =
            make_vault_msg(1, 2, 0, vec![make_pubkey(0), make_pubkey(1)], vec![]);
        assert!(reconstruct_instructions(&too_many_writable_signers).is_err());

        // num_signers > account_keys.len() — would treat keys past the end
        // as signers via out-of-bounds index assumptions. Refuse.
        let signers_exceed_keys =
            make_vault_msg(3, 0, 0, vec![make_pubkey(0), make_pubkey(1)], vec![]);
        assert!(reconstruct_instructions(&signers_exceed_keys).is_err());

        // num_writable_non_signers > non-signer slot count — would mark
        // entries past the end as writable. Refuse.
        let too_many_writable_non_signers =
            make_vault_msg(1, 0, 5, vec![make_pubkey(0), make_pubkey(1)], vec![]);
        assert!(reconstruct_instructions(&too_many_writable_non_signers).is_err());
    }

    #[test]
    fn test_reconstruct_instructions_writable_flags_match_header() {
        // Account layout per Solana MessageHeader convention:
        //   indices [0, num_writable_signers)            -> signer + writable
        //   indices [num_writable_signers, num_signers)  -> signer + readonly
        //   indices [num_signers, num_signers + num_writable_non_signers) -> writable non-signer
        //   indices >=                                    -> readonly non-signer
        // num_signers=2, num_writable_signers=1, num_writable_non_signers=1 means:
        //   key[0] writable signer, key[1] readonly signer, key[2] writable non-signer,
        //   key[3] readonly non-signer (program), key[4] readonly non-signer.
        let vault_msg = make_vault_msg(
            2,
            1,
            1,
            vec![
                make_pubkey(0),
                make_pubkey(1),
                make_pubkey(2),
                make_pubkey(3),
                make_pubkey(4),
            ],
            vec![MultisigCompiledInstruction {
                program_id_index: 3,
                account_indexes: vec![0, 1, 2, 4],
                data: vec![],
            }],
        );
        let reconstructed =
            reconstruct_instructions(&vault_msg).expect("reconstruction should succeed");
        let metas = &reconstructed[0].accounts;
        assert_eq!(metas.len(), 4);
        // key[0]: writable signer
        assert!(metas[0].is_writable && metas[0].is_signer);
        // key[1]: readonly signer
        assert!(!metas[1].is_writable && metas[1].is_signer);
        // key[2]: writable non-signer
        assert!(metas[2].is_writable && !metas[2].is_signer);
        // key[4]: readonly non-signer
        assert!(!metas[3].is_writable && !metas[3].is_signer);
    }

    #[test]
    fn test_build_named_accounts_returns_btreemap_for_deterministic_order() -> TestResult {
        // BTreeMap orders by key; verify two calls yield the same iteration sequence.
        let idl = get_squads_idl().ok_or("Squads IDL should load successfully")?;
        let create_disc = idl
            .instructions
            .iter()
            .find(|i| i.name == "vaultTransactionCreate")
            .and_then(|i| i.discriminator.as_ref())
            .ok_or("vaultTransactionCreate must have a discriminator")?
            .clone();
        // Build minimal "data" carrying the discriminator and arbitrary AccountMeta list.
        let data = create_disc;
        let metas: Vec<AccountMeta> = (1u8..=8)
            .map(|i| AccountMeta::new_readonly(make_pubkey(i), false))
            .collect();
        let first = build_named_accounts(&data, idl, &metas);
        let second = build_named_accounts(&data, idl, &metas);
        let first_keys: Vec<&String> = first.keys().collect();
        let second_keys: Vec<&String> = second.keys().collect();
        assert_eq!(first_keys, second_keys, "BTreeMap iteration must be stable");
        Ok(())
    }

    #[test]
    fn test_vault_pda_account_derives_pda() {
        let multisig = make_pubkey(7);
        let mut named = BTreeMap::new();
        named.insert("multisig".to_string(), multisig.to_string());
        let acct = vault_pda_account(&named, 0).expect("vault PDA should derive");
        // PDA must differ from the multisig and be a valid pubkey string.
        assert_ne!(acct.account_key, multisig.to_string());
        Pubkey::from_str(&acct.account_key).expect("vault PDA should round-trip");
        assert!(acct.signer, "inner sender should be marked as signer");
    }

    #[test]
    fn test_vault_pda_account_missing_multisig_returns_none() {
        let named: BTreeMap<String, String> = BTreeMap::new();
        assert!(vault_pda_account(&named, 0).is_none());
    }

    #[test]
    fn test_visualize_inner_instructions_truncates_when_at_max_depth() -> TestResult {
        // Build a context that is already at MAX_CALL_DEPTH so for_nested_call refuses
        // and visualize_inner_instructions emits the explicit truncation field.
        use crate::core::MAX_CALL_DEPTH;
        let sender = SolanaAccount {
            account_key: make_pubkey(9).to_string(),
            signer: false,
            writable: false,
        };
        let outer_instructions: Vec<Instruction> = vec![];
        let registry = crate::idl::IdlRegistry::new();
        let mut current = VisualizerContext::new(&sender, 0, &outer_instructions, &registry);
        for _ in 0..MAX_CALL_DEPTH {
            current = current
                .for_nested_call(&sender, 0, &outer_instructions)
                .ok_or("for_nested_call should succeed under cap")?;
        }
        // current.call_depth() == MAX_CALL_DEPTH; the next call from inside
        // visualize_inner_instructions returns None and we emit the truncation message.
        let inner_instructions = vec![Instruction {
            program_id: make_pubkey(1),
            accounts: vec![],
            data: vec![],
        }];
        let fields = visualize_inner_instructions(&inner_instructions, &current, None)?;
        assert_eq!(fields.len(), 1, "truncation must emit a single field");
        let serialized = serde_json::to_string(&fields[0])?;
        assert!(
            serialized.contains("maximum nested-instruction visualization depth reached"),
            "field must contain the truncation marker: {serialized}"
        );
        Ok(())
    }

    #[test]
    fn test_try_build_vault_transaction_fields_missing_vault_index_returns_none() -> TestResult {
        // Construct a minimal SolanaParsedInstructionData that has args but no vaultIndex.
        // The args.transactionMessage points at our existing fixture so the message itself
        // parses fine; only vaultIndex is missing.
        let tx_msg_hex = "01010103904fc8953dcfc9f3b5179893ee12fc9c445cad889a957d61cfb8dbcc172f6a4f4a3eef4b03c82a71599ea07a16ee4bcf6dce31357d8460b2ac1bd4c3a9860c9d0954dbbe9ec960c98a7a293fe21336966fe180d151ae4b8179561f89854a53f601020200012800a1b028d53cb8b3e4ef5e27e0961546aad46749acc092e03c1a8c7c1187e887cc245db9cf2bca9a9900";
        let mut args_inner = serde_json::Map::new();
        args_inner.insert(
            "transactionMessage".to_string(),
            serde_json::Value::String(tx_msg_hex.to_string()),
        );
        // Note: deliberately no "vaultIndex" key.
        let mut args_outer = serde_json::Map::new();
        args_outer.insert("args".to_string(), serde_json::Value::Object(args_inner));
        let parsed = SolanaParsedInstructionData {
            instruction_name: "vaultTransactionCreate".to_string(),
            discriminator: "00".to_string(),
            named_accounts: std::collections::HashMap::new(),
            program_call_args: args_outer,
            idl_source: solana_parser::IdlSource::Custom,
            idl_hash: String::new(),
        };
        let named: BTreeMap<String, String> = BTreeMap::new();
        let sender = SolanaAccount {
            account_key: make_pubkey(0).to_string(),
            signer: false,
            writable: false,
        };
        let instructions: Vec<Instruction> = vec![];
        let registry = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, 0, &instructions, &registry);
        let result = try_build_vault_transaction_fields(
            &parsed,
            &named,
            SQUADS_MULTISIG_PROGRAM_ID,
            &context,
        )?;
        assert!(
            result.is_none(),
            "missing vaultIndex must signal fall-back to generic display"
        );
        Ok(())
    }
}
