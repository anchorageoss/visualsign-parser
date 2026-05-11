//! DFlow Aggregator preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::DflowAggregatorConfig;
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

        let program_id = instruction.program_id.to_string();
        let instruction_data_hex = hex::encode(&instruction.data);
        let fallback_text = format!("Program ID: {program_id}\nData: {instruction_data_hex}");

        let parsed = parse_dflow_aggregator_instruction(&instruction.data, &instruction.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &program_id)?,
            Err(e) => {
                tracing::warn!("Failed to parse DFlow Aggregator instruction with IDL: {e}");
                build_fallback_fields(&program_id)?
            }
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        expanded_fields.push(create_raw_data_field(
            &instruction.data,
            Some(instruction_data_hex),
        )?);
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
        Some(&DFLOW_AGGREGATOR_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("DFlow Aggregator")
    }
}

fn get_dflow_aggregator_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(DFLOW_AGGREGATOR_IDL_JSON).ok())
        .as_ref()
}

fn parse_dflow_aggregator_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<DflowAggregatorParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_dflow_aggregator_idl().ok_or("DFlow Aggregator IDL not available")?;
    let parsed = parse_instruction_with_idl(data, DFLOW_AGGREGATOR_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(DflowAggregatorParsedInstruction {
        parsed,
        named_accounts,
        extra_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[AccountMeta],
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut named_accounts = BTreeMap::new();
    let mut extra_accounts = Vec::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_meta) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
            } else {
                extra_accounts.push(account_meta.pubkey.to_string());
            }
        }
    }

    (named_accounts, extra_accounts)
}

struct DflowAggregatorParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &DflowAggregatorParsedInstruction,
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
    let title = format!("DFlow Aggregator: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "DFlow Aggregator")?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        push_arg_fields(&mut condensed_fields, key, value)?;
    }

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
    let title = "DFlow Aggregator: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "DFlow Aggregator")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

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
    use serde_json::json;
    use solana_parser::IdlSource;
    use solana_sdk::pubkey::Pubkey;

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

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

    #[test]
    fn test_push_arg_fields_renders_scalars() {
        let mut fields = Vec::new();
        push_arg_fields(&mut fields, "s", &json!("hello")).unwrap();
        push_arg_fields(&mut fields, "n", &json!(42)).unwrap();
        push_arg_fields(&mut fields, "b", &json!(true)).unwrap();
        push_arg_fields(&mut fields, "z", &serde_json::Value::Null).unwrap();

        assert_eq!(
            fields
                .iter()
                .map(field_label_value)
                .collect::<Vec<(String, String)>>(),
            vec![
                ("s".to_string(), "hello".to_string()),
                ("n".to_string(), "42".to_string()),
                ("b".to_string(), "true".to_string()),
                ("z".to_string(), "null".to_string()),
            ]
        );
    }

    #[test]
    fn test_push_arg_fields_recurses_into_array_with_indexed_labels() {
        let mut fields = Vec::new();
        push_arg_fields(&mut fields, "actions", &json!(["a", "b", "c"])).unwrap();
        let pairs: Vec<(String, String)> = fields.iter().map(field_label_value).collect();
        assert_eq!(
            pairs,
            vec![
                ("actions[0]".to_string(), "a".to_string()),
                ("actions[1]".to_string(), "b".to_string()),
                ("actions[2]".to_string(), "c".to_string()),
            ]
        );
    }

    #[test]
    fn test_push_arg_fields_recurses_into_object_with_dotted_labels() {
        let mut fields = Vec::new();
        push_arg_fields(
            &mut fields,
            "params",
            &json!({"amount": 100, "side": "buy"}),
        )
        .unwrap();
        // Without the `preserve_order` feature, serde_json::Map is a BTreeMap
        // (sorted by key, not insertion-ordered). Assert as a set to stay
        // robust against either backing map.
        let pairs: std::collections::BTreeSet<(String, String)> =
            fields.iter().map(field_label_value).collect();
        let expected: std::collections::BTreeSet<(String, String)> = [
            ("params.amount".to_string(), "100".to_string()),
            ("params.side".to_string(), "buy".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(pairs, expected);
    }

    #[test]
    fn test_push_arg_fields_renders_empty_collections() {
        let mut fields = Vec::new();
        push_arg_fields(&mut fields, "empty_arr", &json!([])).unwrap();
        push_arg_fields(&mut fields, "empty_obj", &json!({})).unwrap();
        assert_eq!(
            fields
                .iter()
                .map(field_label_value)
                .collect::<Vec<(String, String)>>(),
            vec![
                ("empty_arr".to_string(), "[]".to_string()),
                ("empty_obj".to_string(), "{}".to_string()),
            ]
        );
    }

    #[test]
    fn test_build_named_accounts_surfaces_extra_accounts() {
        // Look up the discriminator from the IDL rather than hard-coding the
        // bytes, so the test stays correct across IDL regenerations.
        // close_empty_token_account has 4 named accounts; provide 6 entries so
        // the last 2 land in extra_accounts.
        let idl = get_dflow_aggregator_idl().unwrap();
        let close_ix = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "close_empty_token_account")
            .expect("close_empty_token_account exists in the bundled IDL");
        let close_disc = close_ix
            .discriminator
            .as_ref()
            .expect("instruction has a computed discriminator")
            .clone();
        let pubkeys: Vec<Pubkey> = (0..6).map(|_| Pubkey::new_unique()).collect();
        let accounts: Vec<AccountMeta> = pubkeys
            .iter()
            .map(|pk| AccountMeta::new_readonly(*pk, false))
            .collect();

        let (named, extra) = build_named_accounts(&close_disc, idl, &accounts);

        assert_eq!(named.len(), 4, "first 4 accounts should be named");
        assert_eq!(extra.len(), 2, "remaining 2 accounts should be extras");
        assert_eq!(extra[0], pubkeys[4].to_string());
        assert_eq!(extra[1], pubkeys[5].to_string());
    }

    #[test]
    fn test_remaining_account_label_is_human_readable() {
        // Render the parsed-fields path with extra accounts and assert that the labels
        // are "Remaining Account 1", "Remaining Account 2", etc., not snake_case,
        // and that their text values are the corresponding pubkeys in order.
        let pubkeys: Vec<String> = (0..3).map(|_| Pubkey::new_unique().to_string()).collect();
        let parsed = DflowAggregatorParsedInstruction {
            parsed: SolanaParsedInstructionData {
                instruction_name: "test_ix".to_string(),
                discriminator: "00".to_string(),
                named_accounts: Default::default(),
                program_call_args: serde_json::Map::new(),
                idl_source: IdlSource::Custom,
                idl_hash: String::new(),
            },
            named_accounts: BTreeMap::new(),
            extra_accounts: pubkeys.clone(),
        };

        let (_title, _condensed, expanded) = build_parsed_fields(&parsed, "PROGRAM_ID").unwrap();
        let entries: Vec<(String, String)> = expanded
            .iter()
            .map(field_label_value)
            .filter(|(label, _)| label.starts_with("Remaining Account"))
            .collect();
        assert_eq!(
            entries,
            vec![
                ("Remaining Account 1".to_string(), pubkeys[0].clone()),
                ("Remaining Account 2".to_string(), pubkeys[1].clone()),
                ("Remaining Account 3".to_string(), pubkeys[2].clone()),
            ]
        );
    }
}
