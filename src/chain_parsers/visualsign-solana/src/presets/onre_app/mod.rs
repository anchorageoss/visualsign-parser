//! Onre App preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::OnreAppConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const ONRE_APP_PROGRAM_ID: &str = "onreuGhHHgVzMWSkj2oQDLDtvvGvoepBPkqyaubFcwe";

const ONRE_APP_IDL_JSON: &str = include_str!("onre_app.json");

static ONRE_APP_CONFIG: OnreAppConfig = OnreAppConfig;

pub struct OnreAppVisualizer;

impl InstructionVisualizer for OnreAppVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text =
            format!("Program ID: {}\nData: {instruction_data_hex}", view.program_id);

        let parsed = parse_onre_app_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                tracing::warn!("Failed to parse Onre App instruction with IDL: {e}");
                build_fallback_fields(&view.program_id)?
            }
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        expanded_fields.push(create_raw_data_field(data, Some(instruction_data_hex))?);
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
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
        Some(&ONRE_APP_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Onre App")
    }
}

fn get_onre_app_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(ONRE_APP_IDL_JSON).ok())
        .as_ref()
}

fn parse_onre_app_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<OnreAppParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_onre_app_idl().ok_or("Onre App IDL not available")?;
    let parsed = parse_instruction_with_idl(data, ONRE_APP_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(OnreAppParsedInstruction {
        parsed,
        named_accounts,
        extra_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[String],
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut named_accounts = BTreeMap::new();
    let mut extra_accounts = Vec::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_str) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_str.clone());
            } else {
                extra_accounts.push(account_str.clone());
            }
        }
    }

    (named_accounts, extra_accounts)
}

struct OnreAppParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &OnreAppParsedInstruction,
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
    let title = format!("Onre App: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Onre App")?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        push_arg_fields(&mut condensed_fields, key, value)?;
    }

    expanded_fields.push(create_text_field("Program", "Onre App")?);
    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Instruction", instruction_name)?);
    expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (index, pubkey) in instruction.extra_accounts.iter().enumerate() {
        expanded_fields.push(create_text_field(
            &format!("Remaining Account {}", index + 1),
            pubkey,
        )?);
    }

    for (key, value) in &parsed.program_call_args {
        push_arg_fields(&mut expanded_fields, key, value)?;
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
    let title = "Onre App: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Onre App")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program", "Onre App")?);
    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

    Ok((title, condensed_fields, expanded_fields))
}

fn push_arg_fields(
    fields: &mut Vec<AnnotatedPayloadField>,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), VisualSignError> {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                fields.push(create_text_field(key, "{}")?);
            } else {
                for (sub_key, sub_value) in map {
                    let label = format!("{key}.{sub_key}");
                    push_arg_fields(fields, &label, sub_value)?;
                }
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                fields.push(create_text_field(key, "[]")?);
            } else {
                for (i, item) in items.iter().enumerate() {
                    let label = format!("{key}[{i}]");
                    push_arg_fields(fields, &label, item)?;
                }
            }
        }
        serde_json::Value::String(s) => {
            fields.push(create_text_field(key, s)?);
        }
        serde_json::Value::Number(n) => {
            fields.push(create_text_field(key, &n.to_string())?);
        }
        serde_json::Value::Bool(b) => {
            fields.push(create_text_field(key, &b.to_string())?);
        }
        serde_json::Value::Null => {
            fields.push(create_text_field(key, "null")?);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn dummy_account_strings(n: usize) -> Vec<String> {
        use solana_sdk::pubkey::Pubkey;
        (0..n).map(|_| Pubkey::new_unique().to_string()).collect()
    }

    #[test]
    fn test_onre_app_idl_loads() {
        let idl = get_onre_app_idl();
        assert!(idl.is_some(), "Onre App IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_onre_app_idl_has_discriminators() {
        let idl = get_onre_app_idl().unwrap();
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
        let accounts = dummy_account_strings(0);
        let result = parse_onre_app_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = dummy_account_strings(0);
        let result = parse_onre_app_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
