//! Kamino Vault preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::KaminoVaultConfig;
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

pub(crate) const KAMINO_VAULT_PROGRAM_ID: &str = "KvauGMspG5k6rtzrqqn7WNn3oZdyKqLKwK2XWQ8FLjd";

const KAMINO_VAULT_DISPLAY_NAME: &str = "Kamino Vault";

const KAMINO_VAULT_IDL_JSON: &str = include_str!("kamino_vault.json");

static KAMINO_VAULT_CONFIG: KaminoVaultConfig = KaminoVaultConfig;

pub struct KaminoVaultVisualizer;

impl InstructionVisualizer for KaminoVaultVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let parsed_result = parse_kamino_vault_instruction(&instruction.data);
        let program_id_str = instruction.program_id.to_string();

        let (condensed_fields, expanded_fields, title_text) = match &parsed_result {
            Ok(parsed) => {
                let named_accounts = match load_kamino_vault_idl() {
                    Some(idl) => {
                        build_named_accounts(idl, &instruction.data, &instruction.accounts)
                    }
                    None => HashMap::new(),
                };
                (
                    build_condensed_fields(&parsed.instruction_name)?,
                    build_parsed_fields(
                        &program_id_str,
                        parsed,
                        &named_accounts,
                        &instruction.data,
                    )?,
                    format!("{KAMINO_VAULT_DISPLAY_NAME}: {}", parsed.instruction_name),
                )
            }
            Err(_) => (
                build_fallback_condensed_fields()?,
                build_fallback_fields(&program_id_str, &instruction.data)?,
                format!("{KAMINO_VAULT_DISPLAY_NAME}: Unknown Instruction"),
            ),
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title_text }),
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

        let fallback_text = format!(
            "Program ID: {program_id_str}\nData: {}",
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
        Some(&KAMINO_VAULT_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(KAMINO_VAULT_DISPLAY_NAME)
    }
}

/// Load and cache the bundled Kamino Vault IDL.
fn load_kamino_vault_idl() -> Option<&'static Idl> {
    static IDL: std::sync::OnceLock<Option<Idl>> = std::sync::OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(KAMINO_VAULT_IDL_JSON).ok())
        .as_ref()
}

/// Parse an instruction's data using the bundled IDL.
fn parse_kamino_vault_instruction(
    data: &[u8],
) -> Result<SolanaParsedInstructionData, VisualSignError> {
    if data.len() < 8 {
        return Err(VisualSignError::DecodeError(
            "instruction data shorter than 8-byte discriminator".into(),
        ));
    }

    let idl = load_kamino_vault_idl()
        .ok_or_else(|| VisualSignError::DecodeError("failed to load Kamino Vault IDL".into()))?;

    parse_instruction_with_idl(data, KAMINO_VAULT_PROGRAM_ID, idl)
        .map_err(|e| VisualSignError::DecodeError(e.to_string()))
}

/// Build a map of IDL account name to pubkey by zipping instruction accounts with IDL accounts.
fn build_named_accounts(
    idl: &Idl,
    instruction_data: &[u8],
    instruction_accounts: &[solana_sdk::instruction::AccountMeta],
) -> HashMap<String, String> {
    let mut named = HashMap::new();

    let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .map(|d| instruction_data.len() >= 8 && &instruction_data[0..8] == d.as_slice())
            .unwrap_or(false)
    }) else {
        return named;
    };

    for (idx, account_meta) in instruction_accounts.iter().enumerate() {
        if let Some(idl_account) = idl_instruction.accounts.get(idx) {
            named.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
        }
    }

    named
}

/// Build the expanded fields shown when the IDL parsed successfully.
fn build_parsed_fields(
    program_id: &str,
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    let mut account_entries: Vec<(&String, &String)> = named_accounts.iter().collect();
    account_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, address) in account_entries {
        fields.push(create_text_field(name, address)?);
    }

    let mut arg_entries: Vec<(&String, &serde_json::Value)> =
        parsed.program_call_args.iter().collect();
    arg_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in arg_entries {
        fields.push(create_text_field(name, &format_arg_value(value))?);
    }

    append_raw_data(&mut fields, data)?;

    Ok(fields)
}

/// Build the expanded fields shown when IDL parsing failed.
fn build_fallback_fields(
    program_id: &str,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Status", "IDL parsing failed - showing raw data")?,
    ];
    append_raw_data(&mut fields, data)?;
    Ok(fields)
}

/// Build the condensed fields when parsing succeeded.
fn build_condensed_fields(
    instruction_name: &str,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok(vec![
        create_text_field("Program", KAMINO_VAULT_DISPLAY_NAME)?,
        create_text_field("Instruction", instruction_name)?,
    ])
}

/// Build the condensed fields when parsing failed.
fn build_fallback_condensed_fields() -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok(vec![create_text_field(
        "Program",
        KAMINO_VAULT_DISPLAY_NAME,
    )?])
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
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_kamino_vault_idl_loads() {
        let idl = load_kamino_vault_idl().expect("IDL should load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should declare at least one instruction"
        );
    }

    #[test]
    fn test_kamino_vault_idl_has_discriminators() {
        let idl = load_kamino_vault_idl().expect("IDL should load");
        for instruction in &idl.instructions {
            let discriminator = instruction.discriminator.as_ref().unwrap_or_else(|| {
                panic!("instruction '{}' missing discriminator", instruction.name)
            });
            assert_eq!(
                discriminator.len(),
                8,
                "instruction '{}' has non-8-byte discriminator",
                instruction.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage = [0xFFu8; 9];
        let result = parse_kamino_vault_instruction(&garbage);
        assert!(
            result.is_err(),
            "garbage discriminator should not match any instruction"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let too_short = [0x01u8, 0x02, 0x03];
        let result = parse_kamino_vault_instruction(&too_short);
        assert!(
            result.is_err(),
            "data shorter than discriminator should return error"
        );
    }
}
