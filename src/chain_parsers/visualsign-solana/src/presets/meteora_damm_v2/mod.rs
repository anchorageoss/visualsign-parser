//! Meteora DAMM V2 preset implementation for Solana
//!
//! IDL-based visualizer for Meteora's DAMM V2 (constant-product AMM) program.

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::MeteoraDammV2Config;
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

pub(crate) const METEORA_DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";

const DISPLAY_NAME: &str = "Meteora DAMM V2";

static METEORA_DAMM_V2_CONFIG: MeteoraDammV2Config = MeteoraDammV2Config;

pub struct MeteoraDammV2Visualizer;

impl InstructionVisualizer for MeteoraDammV2Visualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let data = &instruction.data;
        let program_id = instruction.program_id.to_string();
        let instruction_data_hex = hex::encode(data);

        let parsed = parse_meteora_damm_v2_instruction(data, &instruction.accounts)
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        let instruction_title = format!("{DISPLAY_NAME}: {}", parsed.instruction_name);

        let mut condensed_fields = vec![create_text_field("Instruction", &instruction_title)?];
        for (key, value) in &parsed.program_call_args {
            condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
        }

        let mut expanded_fields = vec![
            create_text_field("Program ID", &program_id)?,
            create_text_field("Instruction", &parsed.instruction_name)?,
            create_text_field("Discriminator", &parsed.discriminator)?,
        ];
        for (account_name, account_address) in &parsed.named_accounts {
            expanded_fields.push(create_text_field(account_name, account_address)?);
        }
        for (key, value) in &parsed.program_call_args {
            expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
        }
        expanded_fields.push(create_raw_data_field(
            data,
            Some(instruction_data_hex.clone()),
        )?);

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: instruction_title.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(SignablePayloadFieldListLayout {
                fields: condensed_fields,
            }),
            expanded: Some(SignablePayloadFieldListLayout {
                fields: expanded_fields,
            }),
        };

        let fallback_text = format!("Program ID: {program_id}\nData: {instruction_data_hex}");

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
        Some(&METEORA_DAMM_V2_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex(DISPLAY_NAME)
    }
}

fn load_idl() -> Result<Idl, Box<dyn std::error::Error>> {
    let json = include_str!("meteora_damm_v2.json");
    decode_idl_data(json)
}

fn parse_meteora_damm_v2_instruction(
    data: &[u8],
    accounts: &[solana_sdk::instruction::AccountMeta],
) -> Result<SolanaParsedInstructionData, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Instruction data too short for Anchor discriminator".into());
    }

    let idl = load_idl()?;
    let mut parsed = parse_instruction_with_idl(data, METEORA_DAMM_V2_PROGRAM_ID, &idl)?;

    let mut named_accounts = HashMap::new();
    if let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.len() >= 8 && &data[0..8] == disc.as_slice())
    }) {
        for (index, account_meta) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
            }
        }
    }
    parsed.named_accounts = named_accounts;

    Ok(parsed)
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_meteora_damm_v2_idl_loads() {
        let idl = load_idl().expect("IDL should load");
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_meteora_damm_v2_idl_has_discriminators() {
        let idl = load_idl().expect("IDL should load");
        for inst in &idl.instructions {
            let disc = inst
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("Instruction {} missing discriminator", inst.name));
            assert_eq!(
                disc.len(),
                8,
                "Instruction {} discriminator must be 8 bytes",
                inst.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage = [0xAA_u8; 9];
        let result = parse_meteora_damm_v2_instruction(&garbage, &[]);
        assert!(
            result.is_err(),
            "Unknown discriminator should return an error"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let short = [0x01_u8, 0x02, 0x03];
        let result = parse_meteora_damm_v2_instruction(&short, &[]);
        assert!(result.is_err(), "Data shorter than 8 bytes must error");
    }
}
