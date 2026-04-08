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
    ///         u8 address_table_lookups count + lookups
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
            let data_len = read_u8(&mut pos)? as usize;
            let instruction_data = read_bytes(&mut pos, data_len)?.to_vec();
            instructions.push(MultisigCompiledInstruction {
                program_id_index,
                account_indexes,
                data: instruction_data,
            });
        }

        // Address table lookups: u8 count (we skip parsing the contents)
        // They're not needed for instruction reconstruction without ALT resolution

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
                build_parsed_fields(&parsed, &instruction.program_id.to_string(), context)
            }
            Err(_) => build_fallback_fields(&instruction.program_id.to_string()),
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        let expanded_with_raw =
            append_raw_data(expanded_fields, &instruction.data, &instruction_data_hex);
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

fn get_squads_idl() -> Option<Idl> {
    decode_idl_data(SQUADS_IDL_JSON).ok()
}

fn parse_squads_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<SquadsParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_squads_idl().ok_or("Squads Multisig IDL not available")?;
    let parsed = parse_instruction_with_idl(data, SQUADS_MULTISIG_PROGRAM_ID, &idl)?;

    let named_accounts = build_named_accounts(data, &idl, accounts);

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

fn build_parsed_fields(
    instruction: &SquadsParsedInstruction,
    program_id: &str,
    context: &VisualizerContext,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let parsed = &instruction.parsed;

    // Special case: decode nested transaction message for vaultTransactionCreate
    if parsed.instruction_name == "vaultTransactionCreate" {
        if let Some(fields) = try_build_vault_transaction_fields(
            parsed,
            &instruction.named_accounts,
            program_id,
            context,
        ) {
            return fields;
        }
    }

    build_generic_fields(parsed, &instruction.named_accounts, program_id)
}

/// Try to decode the nested transaction message inside vaultTransactionCreate.
/// Returns None if decoding fails, falling through to generic display.
fn try_build_vault_transaction_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
    program_id: &str,
    context: &VisualizerContext,
) -> Option<(
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
)> {
    // Extract transactionMessage hex from the nested args struct
    let args_value = parsed.program_call_args.get("args")?;
    let tx_msg_hex = args_value.get("transactionMessage")?.as_str()?;
    let tx_msg_bytes = hex::decode(tx_msg_hex).ok()?;
    let vault_msg = VaultTransactionMessage::deserialize(&tx_msg_bytes).ok()?;

    // Reconstruct full Instructions from the compiled instructions
    let inner_instructions = reconstruct_instructions(&vault_msg);

    // Visualize inner instructions using the full visualizer framework
    let inner_fields = visualize_inner_instructions(&inner_instructions, context);

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
    let mut condensed_fields = vec![];
    if let Ok(f) = create_text_field("Program", "Squads Multisig") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", "vaultTransactionCreate") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Vault Index", &vault_index.to_string()) {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field(
        "Inner Instructions",
        &format!("{inner_count} instruction(s)"),
    ) {
        condensed_fields.push(f);
    }

    // Expanded: full details + decoded inner instructions
    let mut expanded_fields = vec![];
    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", "vaultTransactionCreate") {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Discriminator", &parsed.discriminator) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Vault Index", &vault_index.to_string()) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Memo", memo) {
        expanded_fields.push(f);
    }

    // Named accounts from the outer instruction
    for (account_name, account_address) in named_accounts {
        if let Ok(f) = create_text_field(account_name, account_address) {
            expanded_fields.push(f);
        }
    }

    // Decoded inner instructions
    expanded_fields.extend(inner_fields);

    Some((title, condensed_fields, expanded_fields))
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
fn visualize_inner_instructions(
    inner_instructions: &[Instruction],
    context: &VisualizerContext,
) -> Vec<AnnotatedPayloadField> {
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

    instructions_vec
        .iter()
        .enumerate()
        .filter_map(|(idx, _)| {
            let inner_context =
                VisualizerContext::new(&sender, idx, &instructions_vec, &idl_registry);

            visualize_with_any(&visualizers_refs, &inner_context)
                .and_then(|result| result.ok())
                .map(|viz_result| viz_result.field)
        })
        .collect()
}

fn build_generic_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
    program_id: &str,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let title = format!("Squads Multisig: {}", parsed.instruction_name);

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    // Condensed: program name + instruction name + key args
    if let Ok(f) = create_text_field("Program", "Squads Multisig") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", &parsed.instruction_name) {
        condensed_fields.push(f);
    }
    for (key, value) in &parsed.program_call_args {
        if let Ok(f) = create_text_field(key, &format_arg_value(value)) {
            condensed_fields.push(f);
        }
    }

    // Expanded: full details
    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", &parsed.instruction_name) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Discriminator", &parsed.discriminator) {
        expanded_fields.push(f);
    }

    for (account_name, account_address) in named_accounts {
        if let Ok(f) = create_text_field(account_name, account_address) {
            expanded_fields.push(f);
        }
    }

    for (key, value) in &parsed.program_call_args {
        if let Ok(f) = create_text_field(key, &format_arg_value(value)) {
            expanded_fields.push(f);
        }
    }

    (title, condensed_fields, expanded_fields)
}

fn build_fallback_fields(
    program_id: &str,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let title = "Squads Multisig: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    if let Ok(f) = create_text_field("Program", "Squads Multisig") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Status", "Unknown instruction type") {
        condensed_fields.push(f);
    }

    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Status", "Unknown instruction type") {
        expanded_fields.push(f);
    }

    (title, condensed_fields, expanded_fields)
}

fn append_raw_data(
    mut fields: Vec<AnnotatedPayloadField>,
    data: &[u8],
    hex_str: &str,
) -> Vec<AnnotatedPayloadField> {
    if let Ok(f) = create_raw_data_field(data, Some(hex_str.to_string())) {
        fields.push(f);
    }
    fields
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

    #[test]
    fn test_squads_idl_loads() {
        let idl = get_squads_idl();
        assert!(idl.is_some(), "Squads IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_squads_idl_has_discriminators() {
        let idl = get_squads_idl().unwrap();
        for instruction in &idl.instructions {
            assert!(
                instruction.discriminator.is_some(),
                "Instruction '{}' should have a computed discriminator",
                instruction.name
            );
            let disc = instruction.discriminator.as_ref().unwrap();
            assert_eq!(
                disc.len(),
                8,
                "Discriminator for '{}' should be 8 bytes",
                instruction.name
            );
        }
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
    fn test_vault_transaction_message_deserialization() {
        // The transactionMessage hex from the sample transaction
        let tx_msg_hex = "01010103904fc8953dcfc9f3b5179893ee12fc9c445cad889a957d61cfb8dbcc172f6a4f4a3eef4b03c82a71599ea07a16ee4bcf6dce31357d8460b2ac1bd4c3a9860c9d0954dbbe9ec960c98a7a293fe21336966fe180d151ae4b8179561f89854a53f601020200012800a1b028d53cb8b3e4ef5e27e0961546aad46749acc092e03c1a8c7c1187e887cc245db9cf2bca9a9900";
        let tx_msg_bytes = hex::decode(tx_msg_hex).unwrap();
        let vault_msg = VaultTransactionMessage::deserialize(&tx_msg_bytes).unwrap();

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

        let inner = &vault_msg.instructions[0];
        assert!(
            (inner.program_id_index as usize) < vault_msg.account_keys.len(),
            "program_id_index should be valid"
        );

        let instructions = reconstruct_instructions(&vault_msg);
        assert_eq!(instructions.len(), 1, "Should reconstruct 1 instruction");
        // The inner instruction's program_id should be one of the account keys
        assert!(
            vault_msg.account_keys.contains(&instructions[0].program_id),
            "Inner instruction program_id should be in account_keys"
        );
    }
}
