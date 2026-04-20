//! Onre App preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::OnreAppConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const ONRE_APP_PROGRAM_ID: &str = "onreuGhHHgVzMWSkj2oQDLDtvvGvoepBPkqyaubFcwe";

const ONRE_APP_IDL_JSON: &str = include_str!("onre_app.json");

static ONRE_APP_CONFIG: OnreAppConfig = OnreAppConfig;

#[derive(Debug, Clone)]
pub struct OnreAppParsedInstruction {
    pub parsed: SolanaParsedInstructionData,
    pub named_accounts: Vec<(String, String)>,
}

pub struct OnreAppVisualizer;

impl InstructionVisualizer for OnreAppVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let instruction_number = context.instruction_index() + 1;
        let program_id_str = instruction.program_id.to_string();
        let instruction_data_hex = hex::encode(&instruction.data);

        let decoded = parse_onre_app_instruction(&instruction.data, &instruction.accounts)
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        let instruction_name = decoded.parsed.instruction_name.clone();
        let summary = format!("Onre App: {instruction_name}");

        let mut condensed_fields = vec![
            create_text_field("Instruction", &summary)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        ];
        for (key, value) in &decoded.parsed.program_call_args {
            condensed_fields.push(
                create_text_field(key, &format_arg_value(value))
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );
        }

        let mut expanded_fields = vec![
            create_text_field("Program ID", &program_id_str)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            create_text_field("Instruction", &instruction_name)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            create_text_field("Discriminator", &decoded.parsed.discriminator)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        ];
        for (account_name, account_address) in &decoded.named_accounts {
            expanded_fields.push(
                create_text_field(account_name, account_address)
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );
        }
        for (key, value) in &decoded.parsed.program_call_args {
            expanded_fields.push(
                create_text_field(key, &format_arg_value(value))
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );
        }
        expanded_fields.push(
            create_raw_data_field(&instruction.data, Some(instruction_data_hex.clone()))
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        );

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: summary.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: "Onre App".to_string(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        let fallback_text = format!("Program ID: {program_id_str}\nData: {instruction_data_hex}");

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {instruction_number}"),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&ONRE_APP_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Onre App")
    }
}

fn get_onre_app_idl() -> Result<Idl, Box<dyn std::error::Error>> {
    decode_idl_data(ONRE_APP_IDL_JSON)
}

fn parse_onre_app_instruction(
    data: &[u8],
    accounts: &[solana_sdk::instruction::AccountMeta],
) -> Result<OnreAppParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_onre_app_idl()?;
    let parsed = parse_instruction_with_idl(data, ONRE_APP_PROGRAM_ID, &idl)?;

    let mut named_accounts: Vec<(String, String)> = Vec::new();
    if let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        if let Some(ref disc) = inst.discriminator {
            data.len() >= 8 && &data[0..8] == disc.as_slice()
        } else {
            false
        }
    }) {
        for (index, account_meta) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.push((idl_account.name.clone(), account_meta.pubkey.to_string()));
            }
        }
    }

    Ok(OnreAppParsedInstruction {
        parsed,
        named_accounts,
    })
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
    fn test_onre_app_idl_loads() {
        let idl = get_onre_app_idl().expect("IDL should decode");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should contain instructions"
        );
    }

    #[test]
    fn test_onre_app_idl_has_discriminators() {
        let idl = get_onre_app_idl().expect("IDL should decode");
        for inst in &idl.instructions {
            let disc = inst
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("Instruction {} missing discriminator", inst.name));
            assert_eq!(
                disc.len(),
                8,
                "Instruction {} discriminator should be 8 bytes",
                inst.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage: [u8; 9] = [0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x00];
        let result = parse_onre_app_instruction(&garbage, &[]);
        assert!(
            result.is_err(),
            "Unknown discriminator should produce an error"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let short: [u8; 3] = [0x01, 0x02, 0x03];
        let result = parse_onre_app_instruction(&short, &[]);
        assert!(result.is_err(), "Short data should produce an error");
    }
}
