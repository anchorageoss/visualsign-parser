//! DFlow Aggregator preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::DflowAggregatorConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::HashMap;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const DFLOW_AGGREGATOR_PROGRAM_ID: &str = "DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH";

const DFLOW_AGGREGATOR_IDL_JSON: &str = include_str!("dflow_aggregator.json");

static DFLOW_AGGREGATOR_CONFIG: DflowAggregatorConfig = DflowAggregatorConfig;

pub struct DflowAggregatorVisualizer;

impl InstructionVisualizer for DflowAggregatorVisualizer {
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

        let parsed = parse_dflow_aggregator_instruction(&instruction.data, &instruction.accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &instruction.program_id.to_string()),
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
        Some(&DFLOW_AGGREGATOR_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("DFlow Aggregator")
    }
}

fn get_dflow_aggregator_idl() -> Option<Idl> {
    decode_idl_data(DFLOW_AGGREGATOR_IDL_JSON).ok()
}

fn parse_dflow_aggregator_instruction(
    data: &[u8],
    accounts: &[solana_sdk::instruction::AccountMeta],
) -> Result<DflowAggregatorParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_dflow_aggregator_idl().ok_or("DFlow Aggregator IDL not available")?;
    let parsed = parse_instruction_with_idl(data, DFLOW_AGGREGATOR_PROGRAM_ID, &idl)?;

    let named_accounts = build_named_accounts(data, &idl, accounts);

    Ok(DflowAggregatorParsedInstruction {
        parsed,
        named_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[solana_sdk::instruction::AccountMeta],
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

struct DflowAggregatorParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: HashMap<String, String>,
}

fn build_parsed_fields(
    instruction: &DflowAggregatorParsedInstruction,
    program_id: &str,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let parsed = &instruction.parsed;
    let title = format!("DFlow Aggregator: {}", parsed.instruction_name);

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    if let Ok(f) = create_text_field("Program", "DFlow Aggregator") {
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

    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", &parsed.instruction_name) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Discriminator", &parsed.discriminator) {
        expanded_fields.push(f);
    }

    for (account_name, account_address) in &instruction.named_accounts {
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
    let title = "DFlow Aggregator: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    if let Ok(f) = create_text_field("Program", "DFlow Aggregator") {
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_dflow_aggregator_idl_loads() {
        let idl = get_dflow_aggregator_idl();
        assert!(
            idl.is_some(),
            "DFlow Aggregator IDL should load successfully"
        );
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_dflow_aggregator_idl_has_discriminators() {
        let idl = get_dflow_aggregator_idl().unwrap();
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
        let result = parse_dflow_aggregator_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_dflow_aggregator_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
