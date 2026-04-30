//! Meteora DLMM preset implementation for Solana
//!
//! Generic IDL-driven visualizer for the Meteora DLMM (lb_clmm) program.
//! Uses the bundled IDL to decode instructions and render their arguments
//! and named accounts into a VisualSign preview layout.

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::MeteoraDlmmConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const METEORA_DLMM_DISPLAY_NAME: &str = "Meteora DLMM";
const METEORA_DLMM_IDL_JSON: &str = include_str!("meteora_dlmm.json");

static METEORA_DLMM_CONFIG: MeteoraDlmmConfig = MeteoraDlmmConfig;

pub struct MeteoraDlmmVisualizer;

#[derive(Debug, Clone)]
struct MeteoraDlmmParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: Vec<(String, String)>,
}

impl InstructionVisualizer for MeteoraDlmmVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let data = &instruction.data;
        if data.len() < 8 {
            return Err(VisualSignError::DecodeError(
                "Instruction data shorter than 8-byte discriminator".into(),
            ));
        }

        let idl = get_meteora_dlmm_idl()
            .ok_or_else(|| VisualSignError::DecodeError("Meteora DLMM IDL not available".into()))?;

        let parsed = parse_instruction_with_idl(data, METEORA_DLMM_PROGRAM_ID, idl)
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        let named_accounts = build_named_accounts(idl, instruction);

        let parsed_instruction = MeteoraDlmmParsedInstruction {
            parsed,
            named_accounts,
        };

        let instruction_text = format!(
            "{METEORA_DLMM_DISPLAY_NAME}: {}",
            parsed_instruction.parsed.instruction_name
        );

        let condensed = SignablePayloadFieldListLayout {
            fields: build_condensed_fields(&instruction_text, &parsed_instruction)?,
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: build_expanded_fields(
                &parsed_instruction,
                &instruction.program_id.to_string(),
                data,
            )?,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: instruction_text.clone(),
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
            hex::encode(data)
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
        Some(&METEORA_DLMM_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex(METEORA_DLMM_DISPLAY_NAME)
    }
}

fn get_meteora_dlmm_idl() -> Option<&'static Idl> {
    static IDL: std::sync::OnceLock<Option<Idl>> = std::sync::OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(METEORA_DLMM_IDL_JSON).ok())
        .as_ref()
}

fn build_named_accounts(
    idl: &Idl,
    instruction: &solana_sdk::instruction::Instruction,
) -> Vec<(String, String)> {
    let data = &instruction.data;
    if data.len() < 8 {
        return Vec::new();
    }

    let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| &data[0..8] == disc.as_slice())
    }) else {
        return Vec::new();
    };

    instruction
        .accounts
        .iter()
        .zip(idl_instruction.accounts.iter())
        .map(|(meta, idl_account)| (idl_account.name.clone(), meta.pubkey.to_string()))
        .collect()
}

fn build_condensed_fields(
    instruction_text: &str,
    parsed: &MeteoraDlmmParsedInstruction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Instruction", instruction_text)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    ];

    for (key, value) in &parsed.parsed.program_call_args {
        fields.push(
            create_text_field(key, &format_arg_value(value))
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        );
    }

    Ok(fields)
}

fn build_expanded_fields(
    parsed: &MeteoraDlmmParsedInstruction,
    program_id: &str,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        create_text_field("Instruction", &parsed.parsed.instruction_name)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        create_text_field("Discriminator", &parsed.parsed.discriminator)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    ];

    for (name, address) in &parsed.named_accounts {
        fields.push(
            create_text_field(name, address)
                .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
        );
    }

    for (key, value) in &parsed.parsed.program_call_args {
        fields.push(
            create_text_field(key, &format_arg_value(value))
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
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_meteora_dlmm_idl_loads() {
        let idl = get_meteora_dlmm_idl().expect("IDL must load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL must contain at least one instruction"
        );
    }

    #[test]
    fn test_meteora_dlmm_idl_has_discriminators() {
        let idl = get_meteora_dlmm_idl().expect("IDL must load");
        for inst in &idl.instructions {
            let disc = inst
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("Instruction {} missing discriminator", inst.name));
            assert_eq!(
                disc.len(),
                8,
                "Instruction {} has {}-byte discriminator, expected 8",
                inst.name,
                disc.len()
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let idl = get_meteora_dlmm_idl().expect("IDL must load");
        let garbage = [0u8; 9];
        let result = parse_instruction_with_idl(&garbage, METEORA_DLMM_PROGRAM_ID, idl);
        assert!(
            result.is_err(),
            "Unknown discriminator must fail IDL parsing"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        // Directly exercise the length guard via the visualizer contract: data < 8 must error.
        // parse_instruction_with_idl itself also rejects short data, so assert that path.
        let idl = get_meteora_dlmm_idl().expect("IDL must load");
        let short = [0u8; 3];
        let result = parse_instruction_with_idl(&short, METEORA_DLMM_PROGRAM_ID, idl);
        assert!(result.is_err(), "Data shorter than 8 bytes must fail");
    }

    #[test]
    fn test_swap_instruction_parses() {
        let idl = get_meteora_dlmm_idl().expect("IDL must load");
        // swap discriminator + amount_in=1000 (u64 LE) + min_amount_out=900 (u64 LE)
        let mut data = vec![248, 198, 158, 145, 225, 117, 135, 200];
        data.extend_from_slice(&1000u64.to_le_bytes());
        data.extend_from_slice(&900u64.to_le_bytes());

        let parsed = parse_instruction_with_idl(&data, METEORA_DLMM_PROGRAM_ID, idl)
            .expect("swap must parse");
        assert_eq!(parsed.instruction_name, "swap");
        assert!(parsed.program_call_args.contains_key("amount_in"));
        assert!(parsed.program_call_args.contains_key("min_amount_out"));
    }

    #[test]
    fn test_kind_is_dex() {
        let visualizer = MeteoraDlmmVisualizer;
        assert!(matches!(
            visualizer.kind(),
            VisualizerKind::Dex(METEORA_DLMM_DISPLAY_NAME)
        ));
    }

    #[test]
    fn test_build_named_accounts_pairs_ordered_accounts() {
        use solana_sdk::instruction::{AccountMeta, Instruction};
        use solana_sdk::pubkey::Pubkey;

        let idl = get_meteora_dlmm_idl().expect("IDL must load");

        // close_position2: [position, sender, rent_receiver, event_authority, program]
        let mut data = vec![174, 90, 35, 115, 186, 40, 147, 226];
        let position = Pubkey::new_unique();
        let sender = Pubkey::new_unique();
        let rent_receiver = Pubkey::new_unique();
        let event_authority = Pubkey::new_unique();
        let program = Pubkey::new_unique();
        data.extend([] as [u8; 0]);

        let instruction = Instruction {
            program_id: METEORA_DLMM_PROGRAM_ID.parse().unwrap(),
            accounts: vec![
                AccountMeta::new(position, false),
                AccountMeta::new_readonly(sender, true),
                AccountMeta::new(rent_receiver, false),
                AccountMeta::new_readonly(event_authority, false),
                AccountMeta::new_readonly(program, false),
            ],
            data,
        };

        let named = build_named_accounts(idl, &instruction);
        let lookup: HashMap<_, _> = named.into_iter().collect();
        assert_eq!(lookup.get("position"), Some(&position.to_string()));
        assert_eq!(lookup.get("sender"), Some(&sender.to_string()));
        assert_eq!(
            lookup.get("rent_receiver"),
            Some(&rent_receiver.to_string())
        );
    }
}
