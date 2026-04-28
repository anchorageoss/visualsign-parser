//! MetaDAO Conditional Vault preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::MetadaoConditionalVaultConfig;
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

pub(crate) const METADAO_CONDITIONAL_VAULT_PROGRAM_ID: &str =
    "VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg";

const DISPLAY_NAME: &str = "MetaDAO Conditional Vault";

const IDL_JSON: &str = include_str!("metadao_conditional_vault.json");

static CONFIG: MetadaoConditionalVaultConfig = MetadaoConditionalVaultConfig;

pub struct MetadaoConditionalVaultVisualizer;

impl InstructionVisualizer for MetadaoConditionalVaultVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        if instruction.data.len() < 8 {
            return Err(VisualSignError::DecodeError(
                "Instruction data too short for Anchor discriminator".to_string(),
            ));
        }

        let idl = load_idl()?;
        let parsed = parse_instruction_with_idl(
            &instruction.data,
            METADAO_CONDITIONAL_VAULT_PROGRAM_ID,
            &idl,
        )
        .map_err(|e| VisualSignError::DecodeError(format!("IDL parse failed: {e}")))?;

        let named_accounts = build_named_accounts(&idl, &instruction.data, &instruction.accounts);

        build_visualization(context, instruction, &parsed, &named_accounts)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex(DISPLAY_NAME)
    }
}

fn load_idl() -> Result<Idl, VisualSignError> {
    decode_idl_data(IDL_JSON)
        .map_err(|e| VisualSignError::DecodeError(format!("Failed to decode bundled IDL: {e}")))
}

fn build_named_accounts(
    idl: &Idl,
    instruction_data: &[u8],
    accounts: &[solana_sdk::instruction::AccountMeta],
) -> HashMap<String, String> {
    let mut named = HashMap::new();
    if instruction_data.len() < 8 {
        return named;
    }
    let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| &instruction_data[0..8] == disc.as_slice())
    }) else {
        return named;
    };
    for (index, account_meta) in accounts.iter().enumerate() {
        if let Some(idl_account) = idl_instruction.accounts.get(index) {
            named.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
        }
    }
    named
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

fn build_visualization(
    context: &VisualizerContext,
    instruction: &solana_sdk::instruction::Instruction,
    parsed: &SolanaParsedInstructionData,
    named_accounts: &HashMap<String, String>,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id = instruction.program_id.to_string();
    let title = format!("{DISPLAY_NAME}: {}", parsed.instruction_name);

    let condensed_fields = vec![create_text_field("Instruction", &parsed.instruction_name)?];

    let mut expanded_fields = vec![
        create_text_field("Program", DISPLAY_NAME)?,
        create_text_field("Program ID", &program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    let mut account_keys: Vec<&String> = named_accounts.keys().collect();
    account_keys.sort();
    for key in account_keys {
        if let Some(addr) = named_accounts.get(key) {
            expanded_fields.push(create_text_field(key, addr)?);
        }
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_raw_data_field(
        &instruction.data,
        Some(hex::encode(&instruction.data)),
    )?);

    let condensed = SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title.clone(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    let fallback_text = format!(
        "Program ID: {program_id}\nData: {}",
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_sdk::{instruction::AccountMeta, pubkey::Pubkey};

    fn dummy_account_metas(count: usize) -> Vec<AccountMeta> {
        (0..count)
            .map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false))
            .collect()
    }

    #[test]
    fn test_metadao_conditional_vault_idl_loads() {
        let idl = load_idl().expect("IDL must load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should declare at least one instruction"
        );
    }

    #[test]
    fn test_metadao_conditional_vault_idl_has_discriminators() {
        let idl = load_idl().expect("IDL must load");
        for instruction in &idl.instructions {
            let discriminator = instruction
                .discriminator
                .as_ref()
                .expect("decode_idl_data must populate every discriminator");
            assert_eq!(
                discriminator.len(),
                8,
                "discriminator for `{}` should be 8 bytes, got {}",
                instruction.name,
                discriminator.len()
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let idl = load_idl().expect("IDL must load");
        let bogus = [0xFFu8; 9];
        let result = parse_instruction_with_idl(&bogus, METADAO_CONDITIONAL_VAULT_PROGRAM_ID, &idl);
        assert!(
            result.is_err(),
            "unknown discriminator should fail to parse"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let idl = load_idl().expect("IDL must load");
        let short = [0x01u8, 0x02, 0x03];
        let result = parse_instruction_with_idl(&short, METADAO_CONDITIONAL_VAULT_PROGRAM_ID, &idl);
        assert!(result.is_err(), "data shorter than 8 bytes should error");
    }

    #[test]
    fn test_build_named_accounts_matches_idl_account_names() {
        let idl = load_idl().expect("IDL must load");
        let split_tokens = idl
            .instructions
            .iter()
            .find(|i| i.name == "splitTokens")
            .expect("splitTokens instruction present");
        let discriminator = split_tokens
            .discriminator
            .clone()
            .expect("discriminator computed");
        let amount: u64 = 42;
        let mut data = discriminator;
        data.extend_from_slice(&amount.to_le_bytes());

        let metas = dummy_account_metas(split_tokens.accounts.len());
        let named = build_named_accounts(&idl, &data, &metas);

        assert_eq!(
            named.len(),
            split_tokens.accounts.len(),
            "every IDL account should be named"
        );
        for idl_account in &split_tokens.accounts {
            assert!(
                named.contains_key(&idl_account.name),
                "missing named account `{}`",
                idl_account.name
            );
        }
    }
}
