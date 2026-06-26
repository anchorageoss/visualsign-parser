//! Neutral Trade preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind, format_arg_value,
};
use config::NeutralTradeConfig;
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

pub(crate) const NEUTRAL_TRADE_PROGRAM_ID: &str = "BUNDDh4P5XviMm1f3gCvnq2qKx6TGosAGnoUK12e7cXU";

const NEUTRAL_TRADE_IDL_JSON: &str = include_str!("neutral_trade.json");

static NEUTRAL_TRADE_CONFIG: NeutralTradeConfig = NeutralTradeConfig;

pub struct NeutralTradeVisualizer;

impl InstructionVisualizer for NeutralTradeVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text =
            format!("Program ID: {}\nData: {instruction_data_hex}", view.program_id);

        let parsed = parse_neutral_trade_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                tracing::warn!("Failed to parse Neutral Trade instruction with IDL: {e}");
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
        Some(&NEUTRAL_TRADE_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending("Neutral Trade")
    }
}

fn get_neutral_trade_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(NEUTRAL_TRADE_IDL_JSON).ok())
        .as_ref()
}

fn parse_neutral_trade_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<NeutralTradeParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_neutral_trade_idl().ok_or("Neutral Trade IDL not available")?;
    let parsed = parse_instruction_with_idl(data, NEUTRAL_TRADE_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(NeutralTradeParsedInstruction {
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

struct NeutralTradeParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &NeutralTradeParsedInstruction,
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
    let title = format!("Neutral Trade: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Neutral Trade")?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_text_field("Program", "Neutral Trade")?);
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
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
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
    let title = "Neutral Trade: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Neutral Trade")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program", "Neutral Trade")?);
    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

    Ok((title, condensed_fields, expanded_fields))
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_parser::IdlSource;

    fn dummy_account_strings(n: usize) -> Vec<String> {
        use solana_sdk::pubkey::Pubkey;
        (0..n).map(|_| Pubkey::new_unique().to_string()).collect()
    }

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

    #[test]
    fn test_neutral_trade_idl_loads() {
        let idl = get_neutral_trade_idl();
        assert!(idl.is_some(), "Neutral Trade IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_neutral_trade_idl_has_discriminators() {
        let idl = get_neutral_trade_idl().unwrap();
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
        let result = parse_neutral_trade_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = dummy_account_strings(0);
        let result = parse_neutral_trade_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }

    #[test]
    fn test_build_named_accounts_surfaces_extra_accounts() {
        let idl = get_neutral_trade_idl().unwrap();
        let ix = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "close_user_bundle_account")
            .expect("close_user_bundle_account exists in the bundled IDL");
        let disc = ix
            .discriminator
            .as_ref()
            .expect("instruction has a computed discriminator")
            .clone();

        // close_user_bundle_account has 3 named accounts; provide 5 to get 2 extras
        let accounts = dummy_account_strings(5);
        let (named, extra) = build_named_accounts(&disc, idl, &accounts);

        assert_eq!(named.len(), 3, "first 3 accounts should be named");
        assert_eq!(extra.len(), 2, "remaining 2 accounts should be extras");
        assert_eq!(extra[0], accounts[3]);
        assert_eq!(extra[1], accounts[4]);
    }

    #[test]
    fn test_remaining_account_label_is_human_readable() {
        let pubkeys = dummy_account_strings(3);
        let parsed = NeutralTradeParsedInstruction {
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
