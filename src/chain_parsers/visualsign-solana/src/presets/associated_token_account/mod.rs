//! Associated Token Account preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::AssociatedTokenAccountConfig;
use spl_associated_token_account::instruction::AssociatedTokenAccountInstruction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_text_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

// Create a static instance that we can reference
static ATA_CONFIG: AssociatedTokenAccountConfig = AssociatedTokenAccountConfig;

pub struct AssociatedTokenAccountVisualizer;

impl InstructionVisualizer for AssociatedTokenAccountVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let ata_instruction = parse_ata_instruction(context.data())
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        create_ata_preview_layout(&ata_instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&ATA_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("AssociatedTokenAccount")
    }
}

fn create_ata_preview_layout(
    ata_instruction: &AssociatedTokenAccountInstruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id_str = match context.program_id() {
        ProgramRef::Resolved(pk) => pk.to_string(),
        ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
    };
    let instruction_text = format_ata_instruction(ata_instruction);

    let condensed = SignablePayloadFieldListLayout {
        fields: vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: instruction_text.clone(),
                    label: "Instruction".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: instruction_text.clone(),
                },
            },
        }],
    };

    let expanded = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", &program_id_str)?,
            create_text_field("Instruction", &instruction_text)?,
        ],
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

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: instruction_text,
                fallback_text: format!(
                    "Program ID: {}\nData: {}",
                    program_id_str,
                    hex::encode(context.data())
                ),
            },
            preview_layout,
        },
    })
}

fn parse_ata_instruction(data: &[u8]) -> Result<AssociatedTokenAccountInstruction, &'static str> {
    if data.is_empty() {
        return Ok(AssociatedTokenAccountInstruction::Create);
    }
    match data[0] {
        0 => Ok(AssociatedTokenAccountInstruction::Create),
        1 => Ok(AssociatedTokenAccountInstruction::CreateIdempotent),
        2 => Ok(AssociatedTokenAccountInstruction::RecoverNested),
        _ => Err("Unknown ATA instruction"),
    }
}

fn format_ata_instruction(instruction: &AssociatedTokenAccountInstruction) -> String {
    match instruction {
        AssociatedTokenAccountInstruction::Create => "Create Associated Token Account".to_string(),
        AssociatedTokenAccountInstruction::CreateIdempotent => {
            "Create Associated Token Account (Idempotent)".to_string()
        }
        AssociatedTokenAccountInstruction::RecoverNested => {
            "Recover Nested Associated Token Account".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ata_instruction_empty_data() {
        let empty_data: &[u8] = &[];
        let instruction = parse_ata_instruction(empty_data)
            .expect("Failed to parse ATA instruction with empty data");

        assert!(
            matches!(instruction, AssociatedTokenAccountInstruction::Create),
            "Expected Create instruction for empty data"
        );

        let formatted = format_ata_instruction(&instruction);
        assert_eq!(formatted, "Create Associated Token Account");
    }

    #[test]
    fn test_parse_ata_instruction_with_discriminator_0() {
        let data = [0u8];
        let instruction = parse_ata_instruction(&data)
            .expect("Failed to parse ATA instruction with discriminator 0");

        assert!(
            matches!(instruction, AssociatedTokenAccountInstruction::Create),
            "Expected Create instruction for discriminator 0"
        );
    }

    #[test]
    fn test_parse_ata_instruction_create_idempotent() {
        let data = [1u8];
        let instruction =
            parse_ata_instruction(&data).expect("Failed to parse CreateIdempotent instruction");

        assert!(
            matches!(
                instruction,
                AssociatedTokenAccountInstruction::CreateIdempotent
            ),
            "Expected CreateIdempotent instruction for discriminator 1"
        );

        let formatted = format_ata_instruction(&instruction);
        assert_eq!(formatted, "Create Associated Token Account (Idempotent)");
    }

    #[test]
    fn test_parse_ata_instruction_recover_nested() {
        let data = [2u8];
        let instruction =
            parse_ata_instruction(&data).expect("Failed to parse RecoverNested instruction");

        assert!(
            matches!(
                instruction,
                AssociatedTokenAccountInstruction::RecoverNested
            ),
            "Expected RecoverNested instruction for discriminator 2"
        );

        let formatted = format_ata_instruction(&instruction);
        assert_eq!(formatted, "Recover Nested Associated Token Account");
    }

    #[test]
    fn test_parse_ata_instruction_unknown() {
        let data = [99u8];
        let result = parse_ata_instruction(&data);

        assert!(result.is_err(), "Expected error for unknown discriminator");
        assert_eq!(result.unwrap_err(), "Unknown ATA instruction");
    }
}
