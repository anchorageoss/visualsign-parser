//! Drift Protocol preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::DriftConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::BTreeMap;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const DRIFT_PROGRAM_ID: &str = "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH";

const DRIFT_IDL_JSON: &str = include_str!("drift.json");

static DRIFT_CONFIG: DriftConfig = DriftConfig;

pub struct DriftVisualizer;

impl InstructionVisualizer for DriftVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let program_id = context.resolve_program_id()?.to_string();
        let accounts = context.resolve_accounts()?;
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text = format!("Program ID: {program_id}\nData: {instruction_data_hex}");

        let parsed = parse_drift_instruction(data, &accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &program_id)?,
            Err(_) => build_fallback_fields(&program_id)?,
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        let expanded_with_raw = append_raw_data(expanded_fields, data, &instruction_data_hex)?;
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
        Some(&DRIFT_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Drift")
    }
}

fn get_drift_idl() -> Option<&'static Idl> {
    static IDL: std::sync::LazyLock<Option<Idl>> =
        std::sync::LazyLock::new(|| decode_idl_data(DRIFT_IDL_JSON).ok());
    IDL.as_ref()
}

fn parse_drift_instruction(
    data: &[u8],
    accounts: &[solana_sdk::instruction::AccountMeta],
) -> Result<DriftParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_drift_idl().ok_or("Drift IDL not available")?;
    let parsed = parse_instruction_with_idl(data, DRIFT_PROGRAM_ID, idl)?;

    let named_accounts = build_named_accounts(data, idl, accounts);

    Ok(DriftParsedInstruction {
        parsed,
        named_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[solana_sdk::instruction::AccountMeta],
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

struct DriftParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
}

fn build_parsed_fields(
    instruction: &DriftParsedInstruction,
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
    let title = format!("Drift: {}", parsed.instruction_name);

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Drift")?);
    condensed_fields.push(create_text_field("Instruction", &parsed.instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Instruction", &parsed.instruction_name)?);
    expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);

    for (account_name, account_address) in &instruction.named_accounts {
        let label = if parsed.program_call_args.contains_key(account_name) {
            format!("Current {account_name}")
        } else {
            account_name.clone()
        };
        expanded_fields.push(create_text_field(&label, account_address)?);
    }

    for (key, value) in &parsed.program_call_args {
        let label = if instruction.named_accounts.contains_key(key) {
            format!("New {key}")
        } else {
            key.clone()
        };
        expanded_fields.push(create_text_field(&label, &format_arg_value(value))?);
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
    let title = "Drift: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Drift")?);
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
    use solana_parser::IdlSource;

    fn make_parsed_instruction(instruction_name: &str) -> DriftParsedInstruction {
        DriftParsedInstruction {
            parsed: solana_parser::SolanaParsedInstructionData {
                instruction_name: instruction_name.to_string(),
                discriminator: "aabbccdd".to_string(),
                named_accounts: Default::default(),
                program_call_args: serde_json::Map::new(),
                idl_source: IdlSource::Custom,
                idl_hash: String::new(),
            },
            named_accounts: BTreeMap::new(),
        }
    }

    #[test]
    fn test_build_fallback_fields_returns_result() {
        let (title, condensed, expanded) =
            build_fallback_fields(DRIFT_PROGRAM_ID).unwrap();
        assert_eq!(title, "Drift: Unknown Instruction");
        assert_eq!(condensed.len(), 2);
        assert_eq!(expanded.len(), 2);
    }

    #[test]
    fn test_build_parsed_fields_returns_result() {
        let instruction = make_parsed_instruction("deposit");
        let (title, condensed, expanded) =
            build_parsed_fields(&instruction, DRIFT_PROGRAM_ID)
                .unwrap();
        assert_eq!(title, "Drift: deposit");
        assert!(!condensed.is_empty());
        assert!(!expanded.is_empty());
    }

    #[test]
    fn test_append_raw_data_returns_result() {
        let data = &[0x01u8, 0x02, 0x03];
        let hex_str = hex::encode(data);
        let fields = append_raw_data(vec![], data, &hex_str).unwrap();
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn test_drift_idl_loads() {
        let idl = get_drift_idl();
        assert!(idl.is_some(), "Drift IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_drift_idl_has_discriminators() {
        let idl = get_drift_idl().unwrap();
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
        let result = parse_drift_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_drift_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
