//! Jupiter Perpetuals preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind, format_arg_value,
};
use config::JupiterPerpsConfig;
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

pub(crate) const JUPITER_PERPS_PROGRAM_ID: &str = "PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu";

const JUPITER_PERPS_IDL_JSON: &str = include_str!("jupiter_perps.json");

static JUPITER_PERPS_CONFIG: JupiterPerpsConfig = JupiterPerpsConfig;

pub struct JupiterPerpsVisualizer;

impl InstructionVisualizer for JupiterPerpsVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text = format!("Program ID: {}\nData: {instruction_data_hex}", view.program_id);

        let parsed = parse_jupiter_perps_instruction(data, &view.accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(_) => build_fallback_fields(&view.program_id)?,
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
        Some(&JUPITER_PERPS_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Jupiter Perpetuals")
    }
}

fn get_jupiter_perps_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(JUPITER_PERPS_IDL_JSON).ok())
        .as_ref()
}

fn parse_jupiter_perps_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterPerpsParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_jupiter_perps_idl().ok_or("Jupiter Perpetuals IDL not available")?;
    let parsed = parse_instruction_with_idl(data, JUPITER_PERPS_PROGRAM_ID, idl)?;

    let named_accounts = build_named_accounts(data, idl, accounts);

    Ok(JupiterPerpsParsedInstruction {
        parsed,
        named_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[String],
) -> BTreeMap<String, String> {
    let mut named_accounts = BTreeMap::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_str) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_str.clone());
            }
        }
    }

    named_accounts
}

struct JupiterPerpsParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
}

fn build_parsed_fields(
    instruction: &JupiterPerpsParsedInstruction,
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
    let title = format!("Jupiter Perpetuals: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Jupiter Perpetuals")?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Instruction", instruction_name)?);
    expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
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
    let title = "Jupiter Perpetuals: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Jupiter Perpetuals")?);
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


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_jupiter_perps_idl_loads() {
        let idl = get_jupiter_perps_idl();
        assert!(
            idl.is_some(),
            "Jupiter Perpetuals IDL should load successfully"
        );
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_jupiter_perps_idl_has_discriminators() {
        let idl = get_jupiter_perps_idl().unwrap();
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
        let result = parse_jupiter_perps_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_jupiter_perps_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
