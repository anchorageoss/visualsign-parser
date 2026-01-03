//! Fallback visualizer for unknown/unsupported programs
//! This visualizer provides a best-effort display for programs that don't have dedicated visualizers
//! If an IDL is available for the program, it will attempt to decode using the IDL first

mod config;
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::UnknownProgramConfig;
use solana_parser::{SolanaParsedInstructionData, parse_instruction_with_idl};
use std::collections::HashMap;
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
    let idl_name = idl_registry.get_idl_name(program_id);

    // Try to parse the instruction with IDL
    let parsed_result = try_parse_with_idl(instruction, idl_registry);
    let instruction_data_hex = hex::encode(&instruction.data);

    // Format program display as "UserName (name: idl_name)" if IDL name exists
    let program_display = if let Some(idl_name) = &idl_name {
        format!("{program_name} (name: {idl_name})")
    } else {
        program_name.clone()
    };

    let mut condensed_fields = vec![AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: program_display.clone(),
                label: "Program".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: program_display,
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }];

    let mut expanded_fields = vec![
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
    ];

    // Add parsed instruction fields if IDL parsing succeeded
    match parsed_result {
        Ok(parsed) => {
            // Add instruction name to condensed view
            condensed_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: parsed.instruction_name.clone(),
                        label: "Instruction".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: parsed.instruction_name.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });

            // Add instruction name to expanded view
            expanded_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: parsed.instruction_name.clone(),
                        label: "Instruction".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: parsed.instruction_name.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });

            // Add discriminator
            expanded_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: parsed.discriminator.clone(),
                        label: "Discriminator".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: parsed.discriminator.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });

            // Add named accounts (e.g., mint, depositor_token_account, etc.)
            for (account_name, account_address) in &parsed.named_accounts {
                expanded_fields.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: account_address.clone(),
                            label: account_name.clone(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: account_address.clone(),
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });
            }

            // Add each argument as a separate field in condensed view
            for (key, value) in &parsed.program_call_args {
                condensed_fields.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: value.to_string(),
                            label: key.clone(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: value.to_string(),
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });
            }
        }
        Err(_) => {
            expanded_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "IDL parsing failed - showing raw data".to_string(),
                        label: "Status".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "IDL parsing failed - showing raw data".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }
    }

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

/// Try to parse instruction using the new parse_instruction_with_idl function
fn try_parse_with_idl(
    instruction: &solana_sdk::instruction::Instruction,
    idl_registry: &crate::idl::IdlRegistry,
) -> Result<solana_parser::SolanaParsedInstructionData, Box<dyn std::error::Error>> {
    let program_id_str = instruction.program_id.to_string();
    let instruction_data = &instruction.data;

    // Try to get the IDL for this program
    let idl = idl_registry
        .get_idl(&program_id_str)
        .ok_or("No IDL found for program")?;

    // Parse the instruction with the IDL
    let mut parsed: SolanaParsedInstructionData =
        parse_instruction_with_idl(instruction_data, &program_id_str, &idl)?;

    // Manually create the named_accounts map by matching instruction accounts with IDL
    let mut named_accounts = HashMap::new();

    // Find the matching instruction in the IDL to get account names
    if let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        if let Some(ref disc) = inst.discriminator {
            instruction_data.len() >= 8 && &instruction_data[0..8] == disc.as_slice()
        } else {
            false
        }
    }) {
        // Match each account in the instruction with its name from the IDL
        for (index, account_meta) in instruction.accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_meta.pubkey.to_string());
            }
        }
    }

    parsed.named_accounts = named_accounts;

    Ok(parsed)
}
