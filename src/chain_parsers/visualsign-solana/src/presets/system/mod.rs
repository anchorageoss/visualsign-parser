//! System program preset for Solana

mod account_labels;
mod config;
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    resolve_account_display, resolve_program_display,
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

    let program_id_str = resolve_program_display(context);

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
                        label: format!("Instruction {}", context.instruction_index() + 1),
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
            let new_account = resolve_account_display(context, 1);
            let payer = resolve_account_display(context, 0);

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
                        label: format!("Instruction {}", context.instruction_index() + 1),
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
                        label: format!("Instruction {}", context.instruction_index() + 1),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_parser::solana::structs::SolanaAccount;
    use solana_sdk::instruction::CompiledInstruction;
    use solana_sdk::pubkey::Pubkey;

    fn text_field_value(fields: &[AnnotatedPayloadField], label: &str) -> Option<String> {
        fields.iter().find_map(|f| match &f.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } if common.label == label => {
                Some(text_v2.text.clone())
            }
            _ => None,
        })
    }

    #[test]
    fn test_create_account_with_missing_new_account_index_renders_oob_placeholder() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            // Only the payer (position 0) is present; New Account (position 1) is OOB.
            accounts: vec![0],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);

        let instruction = SystemInstruction::CreateAccount {
            lamports: 100,
            space: 10,
            owner: Pubkey::new_unique(),
        };

        let field = create_system_preview_layout(&instruction, &ctx).unwrap();
        let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            field.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        let expanded = preview_layout.expanded.unwrap();

        assert_eq!(
            text_field_value(&expanded.fields, "New Account").unwrap(),
            "unresolved(oob:1)"
        );
        assert_eq!(
            text_field_value(&expanded.fields, "Payer").unwrap(),
            keys[0].to_string()
        );
    }
}
