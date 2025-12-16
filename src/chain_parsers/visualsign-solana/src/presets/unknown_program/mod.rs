//! Fallback visualizer for unknown/unsupported programs
//! This visualizer provides a best-effort display for programs that don't have dedicated visualizers
//! If an IDL is available for the program, it will attempt to decode using the IDL first

mod config;
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::UnknownProgramConfig;
use visualsign::errors::VisualSignError;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
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

        let idl_registry = context.idl_registry();

        // Try IDL-based parsing if available for this program
        if idl_registry.has_idl(&instruction.program_id) {
            if let Ok(field) = try_idl_parsing(context, idl_registry) {
                return Ok(field);
            }
            // IDL parsing failed, fall through to default visualization
        }

        create_unknown_program_preview_layout(instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&UNKNOWN_PROGRAM_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("UnknownProgram")
    }
}

/// Attempt to parse instruction using IDL from solana_parser
fn try_idl_parsing(
    context: &VisualizerContext,
    idl_registry: &crate::idl::IdlRegistry,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let instruction = context
        .current_instruction()
        .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

    let program_id = &instruction.program_id;
    let program_name = idl_registry.get_program_name(program_id);

    // For now, show that IDL is available but not yet fully integrated
    // TODO: Integrate with solana_parser::parse_transaction_with_idls for full decoding
    // This requires reconstructing the full transaction from context
    let instruction_data_hex = hex::encode(&instruction.data);

    let condensed_fields = vec![AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{program_name} (IDL available)"),
                label: "Program".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{program_name} (IDL available)"),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }];

    let expanded_fields = vec![
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: program_id.to_string(),
                    label: "Program ID".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: program_id.to_string(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: instruction_data_hex.clone(),
                    label: "Instruction Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: instruction_data_hex.clone(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "IDL available - full parsing coming soon".to_string(),
                    label: "Status".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "IDL available - full parsing coming soon".to_string(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
    ];

    let condensed = visualsign::SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = visualsign::SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: format!("{program_name} (IDL)"),
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
                label: format!("Instruction {}", context.instruction_index() + 1),
                fallback_text: format!("Program ID: {program_id}\nData: {instruction_data_hex}"),
            },
            preview_layout,
        },
    })
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
