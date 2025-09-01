//! System program preset for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
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
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        // Try to parse as system instruction
        let system_instruction = bincode::deserialize::<SystemInstruction>(&instruction.data)
            .map_err(|e| {
                VisualSignError::DecodeError(format!("Failed to parse system instruction: {}", e))
            })?;

        // Generate proper preview layout
        create_system_preview_layout(&system_instruction, instruction, context)
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
    solana_instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    use visualsign::field_builders::*;

    match instruction {
        SystemInstruction::Transfer { lamports } => {
            let _from_key = solana_instruction
                .accounts
                .first()
                .map(|meta| meta.pubkey.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let _to_key = solana_instruction
                .accounts
                .get(1)
                .map(|meta| meta.pubkey.to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let condensed_fields = vec![create_text_field(
                "Instruction",
                &format!("Transfer: {} lamports", lamports),
            )?];

            let expanded_fields = vec![
                create_text_field("Program ID", &solana_instruction.program_id.to_string())?,
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
                create_text_field("Raw Data", &hex::encode(&solana_instruction.data))?,
            ];

            let condensed = visualsign::SignablePayloadFieldListLayout {
                fields: condensed_fields,
            };
            let expanded = visualsign::SignablePayloadFieldListLayout {
                fields: expanded_fields,
            };

            let preview_layout = visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: format!("Transfer: {} lamports", lamports),
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
                        fallback_text: format!(
                            "Program ID: {}\nData: {}",
                            solana_instruction.program_id,
                            hex::encode(&solana_instruction.data)
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
            let new_account = solana_instruction
                .accounts
                .get(1)
                .map(|meta| meta.pubkey.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let payer = solana_instruction
                .accounts
                .first()
                .map(|meta| meta.pubkey.to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let condensed_fields = vec![
                create_text_field("Action", "Create Account")?,
                create_text_field("Space", &format!("{} bytes", space))?,
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
                        label: format!("Instruction {}", context.instruction_index() + 1),
                        fallback_text: format!(
                            "Program ID: {}\nData: {}",
                            solana_instruction.program_id,
                            hex::encode(&solana_instruction.data)
                        ),
                    },
                    preview_layout,
                },
            })
        }
        _ => {
            // Handle other system instructions with basic layout
            let instruction_name = match instruction {
                SystemInstruction::Assign { .. } => "Assign Account",
                SystemInstruction::Allocate { .. } => "Allocate Space",
                SystemInstruction::InitializeNonceAccount(_) => "Initialize Nonce Account",
                SystemInstruction::AuthorizeNonceAccount(_) => "Authorize Nonce Account",
                SystemInstruction::AdvanceNonceAccount => "Advance Nonce Account",
                SystemInstruction::WithdrawNonceAccount(_) => "Withdraw from Nonce Account",
                SystemInstruction::CreateAccountWithSeed { .. } => "Create Account With Seed",
                SystemInstruction::AllocateWithSeed { .. } => "Allocate With Seed",
                SystemInstruction::AssignWithSeed { .. } => "Assign With Seed",
                SystemInstruction::TransferWithSeed { .. } => "Transfer With Seed",
                SystemInstruction::UpgradeNonceAccount => "Upgrade Nonce Account",
                _ => "System Instruction",
            };

            let condensed_fields = vec![
                create_text_field("Action", instruction_name)?,
                create_text_field("Program", "System Program")?,
            ];

            let expanded_fields = vec![
                create_text_field("Action", instruction_name)?,
                create_text_field("Program", "System Program")?,
                create_text_field("Instruction Data", &format!("{:?}", instruction))?,
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
                        label: format!("Instruction {}", context.instruction_index() + 1),
                        fallback_text: format!(
                            "Program ID: {}\nData: {}",
                            solana_instruction.program_id,
                            hex::encode(&solana_instruction.data)
                        ),
                    },
                    preview_layout,
                },
            })
        }
    }
}
