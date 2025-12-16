//! Associated Token Account preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
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
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let ata_instruction = parse_ata_instruction(&instruction.data)
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        let instruction_text = format_ata_instruction(&ata_instruction);

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
                create_text_field("Program ID", &instruction.program_id.to_string()).unwrap(),
                create_text_field("Instruction", &instruction_text).unwrap(),
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

        let fallback_instruction_str = format!(
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
                    fallback_text: fallback_instruction_str,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&ATA_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("AssociatedTokenAccount")
    }
}

fn parse_ata_instruction(data: &[u8]) -> Result<AssociatedTokenAccountInstruction, &'static str> {
    // The original SPL ATA "Create" instruction used empty data (no discriminator).
    // Discriminator bytes were added later for CreateIdempotent and RecoverNested.
    // Empty data or data[0] == 0 both mean "Create".
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
