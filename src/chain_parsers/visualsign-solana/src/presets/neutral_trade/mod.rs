//! Neutral Trade program preset for Solana
//!
//! Parses Neutral Trade instructions using the bundled Anchor IDL.

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::NeutralTradeConfig;
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

pub(crate) const NEUTRAL_TRADE_PROGRAM_ID: &str = "BUNDDh4P5XviMm1f3gCvnq2qKx6TGosAGnoUK12e7cXU";
const NEUTRAL_TRADE_IDL_JSON: &str = include_str!("neutral_trade.json");
const NEUTRAL_TRADE_DISPLAY_NAME: &str = "Neutral Trade";

static NEUTRAL_TRADE_CONFIG: NeutralTradeConfig = NeutralTradeConfig;

pub struct NeutralTradeVisualizer;

impl InstructionVisualizer for NeutralTradeVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        if instruction.data.len() < 8 {
            return Err(VisualSignError::DecodeError(
                "Instruction data too short for Anchor discriminator".into(),
            ));
        }

        let idl = load_idl()?;
        let parsed = parse_instruction_with_idl(&instruction.data, NEUTRAL_TRADE_PROGRAM_ID, &idl)
            .map_err(|e| {
                VisualSignError::DecodeError(format!("Neutral Trade IDL parse failed: {e}"))
            })?;

        let named_accounts = build_named_accounts(instruction, &idl);

        let program_id_str = instruction.program_id.to_string();
        let instruction_data_hex = hex::encode(&instruction.data);
        let instruction_title =
            format!("{NEUTRAL_TRADE_DISPLAY_NAME}: {}", parsed.instruction_name);

        let condensed = SignablePayloadFieldListLayout {
            fields: build_condensed_fields(&instruction_title, &parsed)?,
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: build_parsed_fields(
                &program_id_str,
                &parsed,
                &named_accounts,
                &instruction.data,
            )?,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: instruction_title.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: NEUTRAL_TRADE_DISPLAY_NAME.to_string(),
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
                    fallback_text: format!(
                        "Program ID: {program_id_str}\nData: {instruction_data_hex}"
                    ),
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&NEUTRAL_TRADE_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(NEUTRAL_TRADE_DISPLAY_NAME)
    }
}

fn load_idl() -> Result<Idl, VisualSignError> {
    decode_idl_data(NEUTRAL_TRADE_IDL_JSON)
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid Neutral Trade IDL: {e}")))
}

fn build_named_accounts(
    instruction: &solana_sdk::instruction::Instruction,
    idl: &Idl,
) -> HashMap<String, String> {
    let mut named_accounts = HashMap::new();

    let matching_idl_instruction = idl.instructions.iter().find(|inst| {
        if let Some(disc) = inst.discriminator.as_ref() {
            instruction.data.len() >= 8 && &instruction.data[0..8] == disc.as_slice()
        } else {
            false
        }
    });

    if let Some(idl_instruction) = matching_idl_instruction {
        for (index, account_meta) in instruction.accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
            }
        }
    }

    named_accounts
}

fn build_condensed_fields(
    title: &str,
    parsed: &SolanaParsedInstructionData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![create_text_field("Instruction", title)?];
    for (key, value) in &parsed.program_call_args {
        fields.push(create_text_field(key, &format_arg_value(value))?);
    }
    Ok(fields)
}

fn build_parsed_fields(
    program_id: &str,
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program", NEUTRAL_TRADE_DISPLAY_NAME)?,
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    for (name, address) in named_accounts {
        fields.push(create_text_field(name, address)?);
    }

    for (key, value) in &parsed.program_call_args {
        fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    append_raw_data(&mut fields, data)?;
    Ok(fields)
}

fn append_raw_data(
    fields: &mut Vec<AnnotatedPayloadField>,
    data: &[u8],
) -> Result<(), VisualSignError> {
    fields.push(create_raw_data_field(data, Some(hex::encode(data)))?);
    Ok(())
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_neutral_trade_idl_loads() {
        let idl = load_idl().expect("IDL should load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL must contain instructions"
        );
    }

    #[test]
    fn test_neutral_trade_idl_has_discriminators() {
        let idl = load_idl().expect("IDL should load");
        for inst in &idl.instructions {
            let disc = inst
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("instruction '{}' has no discriminator", inst.name));
            assert_eq!(
                disc.len(),
                8,
                "instruction '{}' discriminator must be 8 bytes",
                inst.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let idl = load_idl().expect("IDL should load");
        let garbage = vec![0xFFu8; 9];
        let result = parse_instruction_with_idl(&garbage, NEUTRAL_TRADE_PROGRAM_ID, &idl);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let idl = load_idl().expect("IDL should load");
        let short = vec![0x01u8, 0x02, 0x03];
        let result = parse_instruction_with_idl(&short, NEUTRAL_TRADE_PROGRAM_ID, &idl);
        assert!(result.is_err(), "Short data should return error");
    }
}
