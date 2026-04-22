//! Orca Whirlpool preset — generic IDL-driven visualizer.

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::OrcaWhirlpoolConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

const ORCA_WHIRLPOOL_DISPLAY_NAME: &str = "Orca Whirlpool";

const ORCA_WHIRLPOOL_IDL_JSON: &str = include_str!("orca_whirlpool.json");

static ORCA_WHIRLPOOL_CONFIG: OrcaWhirlpoolConfig = OrcaWhirlpoolConfig;

pub struct OrcaWhirlpoolVisualizer;

impl InstructionVisualizer for OrcaWhirlpoolVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let account_keys: Vec<String> = instruction
            .accounts
            .iter()
            .map(|account| account.pubkey.to_string())
            .collect();

        let parsed = parse_orca_whirlpool_instruction(&instruction.data, &account_keys)?;
        let named_accounts = build_named_accounts(&parsed, &account_keys);

        let title_text = format!("{ORCA_WHIRLPOOL_DISPLAY_NAME}: {}", parsed.instruction_name);

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![
                create_text_field("Instruction", &title_text)
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            ],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: build_expanded_fields(
                &parsed,
                &named_accounts,
                &instruction.program_id.to_string(),
                &instruction.data,
            )?,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        let fallback_text = format!(
            "Program ID: {}\nData: {}",
            instruction.program_id,
            hex::encode(&instruction.data)
        );

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
        Some(&ORCA_WHIRLPOOL_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex(ORCA_WHIRLPOOL_DISPLAY_NAME)
    }
}

fn load_orca_whirlpool_idl() -> Result<&'static Idl, VisualSignError> {
    static IDL: std::sync::OnceLock<Result<Idl, String>> = std::sync::OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(ORCA_WHIRLPOOL_IDL_JSON).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| VisualSignError::DecodeError(format!("Orca Whirlpool IDL invalid: {e}")))
}

fn parse_orca_whirlpool_instruction(
    data: &[u8],
    _accounts: &[String],
) -> Result<SolanaParsedInstructionData, VisualSignError> {
    if data.len() < 8 {
        return Err(VisualSignError::DecodeError(
            "Orca Whirlpool instruction data too short (need 8-byte discriminator)".into(),
        ));
    }

    let idl = load_orca_whirlpool_idl()?;
    parse_instruction_with_idl(data, ORCA_WHIRLPOOL_PROGRAM_ID, idl)
        .map_err(|e| VisualSignError::DecodeError(e.to_string()))
}

fn build_named_accounts(
    parsed: &SolanaParsedInstructionData,
    account_keys: &[String],
) -> Vec<(String, String)> {
    let mut named: Vec<(String, String)> = parsed
        .named_accounts
        .iter()
        .map(|(name, pubkey)| (name.clone(), pubkey.clone()))
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));

    if named.is_empty() {
        return account_keys
            .iter()
            .enumerate()
            .map(|(i, pubkey)| (format!("Account {i}"), pubkey.clone()))
            .collect();
    }

    named
}

fn build_expanded_fields(
    parsed: &SolanaParsedInstructionData,
    named_accounts: &[(String, String)],
    program_id: &str,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();

    fields.push(
        create_text_field("Program ID", program_id)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    );
    fields.push(
        create_text_field("Instruction Name", &parsed.instruction_name)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    );

    for (name, pubkey) in named_accounts {
        let label = format!("Account: {name}");
        fields.push(
            create_text_field(&label, pubkey)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        );
    }

    let mut args: Vec<(&String, &serde_json::Value)> = parsed.program_call_args.iter().collect();
    args.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in args {
        let label = format!("Arg: {name}");
        let rendered = format_arg_value(value);
        fields.push(
            create_text_field(&label, &rendered)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        );
    }

    fields.push(
        create_raw_data_field(data, Some(hex::encode(data)))
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    );

    Ok(fields)
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_orca_whirlpool_idl_loads() {
        let idl = load_orca_whirlpool_idl().expect("IDL should load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should contain at least one instruction"
        );
    }

    #[test]
    fn test_orca_whirlpool_idl_has_discriminators() {
        let idl = load_orca_whirlpool_idl().expect("IDL should load");
        for ix in &idl.instructions {
            let disc = ix
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("instruction {} missing discriminator", ix.name));
            assert_eq!(
                disc.len(),
                8,
                "instruction {} discriminator must be 8 bytes, got {}",
                ix.name,
                disc.len()
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage = [0x00u8; 9];
        let accounts: Vec<String> = vec![];
        let result = parse_orca_whirlpool_instruction(&garbage, &accounts);
        assert!(
            result.is_err(),
            "Unknown discriminator should return an error"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let short = [0x01u8, 0x02, 0x03];
        let accounts: Vec<String> = vec![];
        let result = parse_orca_whirlpool_instruction(&short, &accounts);
        assert!(result.is_err(), "Short data should return an error");
    }
}
