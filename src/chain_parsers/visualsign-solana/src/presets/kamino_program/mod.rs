//! Kamino Program preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::KaminoProgramConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use solana_sdk::instruction::AccountMeta;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const KAMINO_PROGRAM_PROGRAM_ID: &str = "6LtLpnUFNByNXLyCoK9wA2MykKAmQNZKBdY8s47dehDc";

const KAMINO_PROGRAM_IDL_JSON: &str = include_str!("kamino_program.json");

static KAMINO_PROGRAM_CONFIG: KaminoProgramConfig = KaminoProgramConfig;

pub struct KaminoProgramVisualizer;

impl InstructionVisualizer for KaminoProgramVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let program_id = instruction.program_id.to_string();
        let instruction_data_hex = hex::encode(&instruction.data);
        let fallback_text = format!("Program ID: {program_id}\nData: {instruction_data_hex}");

        let parsed = parse_kamino_program_instruction(&instruction.data, &instruction.accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &program_id)?,
            Err(_) => build_fallback_fields(&program_id)?,
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

        let index = context.instruction_index() + 1;
        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {index}"),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&KAMINO_PROGRAM_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Kamino Program")
    }
}

fn get_kamino_program_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(KAMINO_PROGRAM_IDL_JSON).ok())
        .as_ref()
}

fn parse_kamino_program_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<KaminoProgramParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_kamino_program_idl().ok_or("Kamino Program IDL not available")?;
    let parsed = parse_instruction_with_idl(data, KAMINO_PROGRAM_PROGRAM_ID, idl)?;

    let named_accounts = build_named_accounts(data, idl, accounts);

    Ok(KaminoProgramParsedInstruction {
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
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
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

struct KaminoProgramParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
}

fn build_parsed_fields(
    instruction: &KaminoProgramParsedInstruction,
    program_id: &str,
) -> Result<
    (
        String,
        Vec<AnnotatedPayloadField>,
        Vec<AnnotatedPayloadField>,
    ),
    VisualSignError,
> {
    let parsed = &instruction.parsed;
    let instruction_name = &parsed.instruction_name;
    let title = format!("Kamino Program: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Kamino Program")?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Instruction", instruction_name)?);
    expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    Ok((title, condensed_fields, expanded_fields))
}

fn build_fallback_fields(
    program_id: &str,
) -> Result<
    (
        String,
        Vec<AnnotatedPayloadField>,
        Vec<AnnotatedPayloadField>,
    ),
    VisualSignError,
> {
    let title = "Kamino Program: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Kamino Program")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

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

    #[test]
    fn test_kamino_program_idl_loads() {
        let idl = get_kamino_program_idl();
        assert!(idl.is_some(), "Kamino Program IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_kamino_program_idl_has_discriminators() {
        let idl = get_kamino_program_idl().unwrap();
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
        let result = parse_kamino_program_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_kamino_program_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
