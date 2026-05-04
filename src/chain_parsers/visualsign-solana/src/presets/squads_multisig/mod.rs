//! Squads v4 Multisig preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    available_visualizers, visualize_with_any,
};
use crate::idl::IdlRegistry;
use config::SquadsMultisigConfig;
use solana_parser::solana::structs::SolanaAccount;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
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
        let _num_signers = read_u8(&mut pos)?;
        let _num_writable_signers = read_u8(&mut pos)?;
        let _num_writable_non_signers = read_u8(&mut pos)?;

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
) -> HashMap<String, String> {
    let mut named_accounts = HashMap::new();

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
    named_accounts: HashMap<String, String>,
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
/// Returns `Ok(None)` when the embedded transaction message is missing or unparseable
/// (callers fall through to the generic display). Returns `Err` only when field-builder
/// or downstream visualization errors occur — those propagate up so the caller can decide
/// whether to surface them.
fn try_build_vault_transaction_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
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

    // Reconstruct full Instructions from the compiled instructions
    let inner_instructions = reconstruct_instructions(&vault_msg);

    // Visualize inner instructions using the full visualizer framework
    let inner_fields = visualize_inner_instructions(&inner_instructions, context)?;

    let vault_index = args_value
        .get("vaultIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
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

/// Reconstruct Instruction objects from VaultTransactionMessage compiled instructions.
/// Same pattern as core/instructions.rs:39-66.
fn reconstruct_instructions(vault_msg: &VaultTransactionMessage) -> Vec<Instruction> {
    let account_keys = &vault_msg.account_keys;

    vault_msg
        .instructions
        .iter()
        .filter_map(|ci| {
            let program_id_idx = ci.program_id_index as usize;
            if program_id_idx >= account_keys.len() {
                return None;
            }

            let accounts: Vec<AccountMeta> = ci
                .account_indexes
                .iter()
                .filter_map(|&i| {
                    let idx = i as usize;
                    if idx < account_keys.len() {
                        Some(AccountMeta::new_readonly(account_keys[idx], false))
                    } else {
                        None
                    }
                })
                .collect();

            Some(Instruction {
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
fn visualize_inner_instructions(
    inner_instructions: &[Instruction],
    context: &VisualizerContext,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect();

    let idl_registry = IdlRegistry::new();
    let sender = SolanaAccount {
        account_key: context.sender().account_key.clone(),
        signer: false,
        writable: false,
    };

    let instructions_vec: Vec<Instruction> = inner_instructions.to_vec();
    let mut fields = Vec::with_capacity(instructions_vec.len());

    for (idx, _) in instructions_vec.iter().enumerate() {
        let inner_context = VisualizerContext::new(&sender, idx, &instructions_vec, &idl_registry);

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
    named_accounts: &HashMap<String, String>,
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

        let instructions = reconstruct_instructions(&vault_msg);
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
}
