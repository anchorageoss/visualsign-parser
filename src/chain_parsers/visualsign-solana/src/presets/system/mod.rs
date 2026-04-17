//! System program preset for Solana

mod account_labels;
mod config;
use crate::core::{
    AccountRef, InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::SystemConfig;
use solana_program::system_instruction::SystemInstruction;
use visualsign::errors::VisualSignError;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAmountV2,
    SignablePayloadFieldCommon,
};

// Create a static instance that we can reference
static SYSTEM_CONFIG: SystemConfig = SystemConfig;

pub struct SystemVisualizer;

impl InstructionVisualizer for SystemVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let system_instruction = bincode::deserialize::<SystemInstruction>(context.data())
            .map_err(|e| {
                VisualSignError::DecodeError(format!("Failed to parse system instruction: {e}"))
            })?;

        create_system_preview_layout(&system_instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SYSTEM_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("System")
    }
}

fn create_system_preview_layout(
    instruction: &SystemInstruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    use visualsign::field_builders::*;

    let program_id_str = match context.program_id() {
        ProgramRef::Resolved(pk) => pk.to_string(),
        ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
    };

    match instruction {
        SystemInstruction::Transfer { lamports } => {
            let condensed_fields = vec![create_text_field(
                "Instruction",
                &format!("Transfer: {lamports} lamports"),
            )?];

            let expanded_fields = vec![
                create_text_field("Program ID", &program_id_str)?,
                AnnotatedPayloadField {
                    static_annotation: None,
                    dynamic_annotation: None,
                    signable_payload_field: SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{} SOL", (*lamports as f64) / 1_000_000_000.0),
                            label: "Transfer Amount".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: lamports.to_string(),
                            abbreviation: Some("lamports".to_string()),
                        },
                    },
                },
                create_text_field("Raw Data", &hex::encode(context.data()))?,
            ];

            let condensed = visualsign::SignablePayloadFieldListLayout {
                fields: condensed_fields,
            };
            let expanded = visualsign::SignablePayloadFieldListLayout {
                fields: expanded_fields,
            };

            let preview_layout = visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: format!("Transfer: {lamports} lamports"),
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
                        label: format!("Transfer: {lamports} lamports"),
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
        SystemInstruction::CreateAccount {
            lamports,
            space,
            owner,
        } => {
            let new_account = match context.account(1) {
                Some(AccountRef::Resolved(pk)) => pk.to_string(),
                Some(AccountRef::Unresolved { raw_index }) => format!("unresolved({raw_index})"),
                None => "unknown".to_string(),
            };
            let payer = match context.account(0) {
                Some(AccountRef::Resolved(pk)) => pk.to_string(),
                Some(AccountRef::Unresolved { raw_index }) => format!("unresolved({raw_index})"),
                None => "unknown".to_string(),
            };

            let condensed_fields = vec![
                create_text_field("Action", "Create Account")?,
                create_text_field("Space", &format!("{space} bytes"))?,
                create_text_field(
                    "Rent",
                    &format!("{} SOL", (*lamports as f64) / 1_000_000_000.0),
                )?,
            ];

            let expanded_fields = vec![
                create_text_field("Action", "Create Account")?,
                create_text_field("New Account", &new_account)?,
                create_text_field("Payer", &payer)?,
                create_number_field("Space (bytes)", &space.to_string(), "")?,
                create_number_field("Rent (lamports)", &lamports.to_string(), "")?,
                create_text_field(
                    "Rent (SOL)",
                    &format!("{}", (*lamports as f64) / 1_000_000_000.0),
                )?,
                create_text_field("Owner Program", &owner.to_string())?,
                create_text_field("Program", "System Program")?,
            ];

            let condensed = visualsign::SignablePayloadFieldListLayout {
                fields: condensed_fields,
            };
            let expanded = visualsign::SignablePayloadFieldListLayout {
                fields: expanded_fields,
            };

            let preview_layout = visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Create Account".to_string(),
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
                        label: "Create Account".to_string(),
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
        _ => {
            let instruction_name = account_labels::system_instruction_label(instruction);

            let condensed_fields = vec![
                create_text_field("Action", &instruction_name)?,
                create_text_field("Program", "System Program")?,
            ];

            let expanded_fields = vec![
                create_text_field("Action", &instruction_name)?,
                create_text_field("Program", "System Program")?,
                create_text_field("Instruction Data", &format!("{instruction:?}"))?,
            ];

            let condensed = visualsign::SignablePayloadFieldListLayout {
                fields: condensed_fields,
            };
            let expanded = visualsign::SignablePayloadFieldListLayout {
                fields: expanded_fields,
            };

            let preview_layout = visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: instruction_name.to_string(),
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
                        label: instruction_name,
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
    }
}
