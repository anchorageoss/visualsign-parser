//! Fallback visualizer for unknown/unsupported programs
//! This visualizer provides a best-effort display for programs that don't have dedicated visualizers

mod config;
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::UnknownProgramConfig;
use visualsign::errors::VisualSignError;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldPreviewLayout,
};

// Create a static instance that we can reference
static UNKNOWN_PROGRAM_CONFIG: UnknownProgramConfig = UnknownProgramConfig;

pub struct UnknownProgramVisualizer;

impl InstructionVisualizer for UnknownProgramVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        create_unknown_program_preview_layout(instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&UNKNOWN_PROGRAM_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("UnknownProgram")
    }
}

fn create_unknown_program_preview_layout(
    instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    use visualsign::field_builders::*;

    let program_id = instruction.program_id.to_string();
    let instruction_data_hex = hex::encode(&instruction.data);

    // Condensed view - just the essentials
    let condensed_fields = vec![create_text_field("Program", &program_id)?];

    // Expanded view - adds instruction data
    let expanded_fields = vec![
        create_text_field("Program ID", &program_id)?,
        create_text_field("Instruction Data", &instruction_data_hex)?,
    ];

    let condensed = visualsign::SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = visualsign::SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(visualsign::SignablePayloadFieldTextV2 {
            text: program_id.clone(),
        }),
        subtitle: Some(visualsign::SignablePayloadFieldTextV2 {
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
                fallback_text: format!("Program ID: {program_id}\nData: {instruction_data_hex}"),
            },
            preview_layout,
        },
    })
}
