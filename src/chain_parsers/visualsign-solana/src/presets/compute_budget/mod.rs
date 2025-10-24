//! Compute Budget preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use borsh::de::BorshDeserialize;
use config::ComputeBudgetConfig;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_number_field, create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

// Create a static instance that we can reference
static COMPUTE_BUDGET_CONFIG: ComputeBudgetConfig = ComputeBudgetConfig;

pub struct ComputeBudgetVisualizer;

impl InstructionVisualizer for ComputeBudgetVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let compute_budget_instruction =
            ComputeBudgetInstruction::try_from_slice(&instruction.data).map_err(|e| {
                VisualSignError::DecodeError(format!(
                    "Failed to parse compute budget instruction: {e}"
                ))
            })?;

        let instruction_text = format_compute_budget_instruction(&compute_budget_instruction);

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
            fields: create_compute_budget_expanded_fields(
                &compute_budget_instruction,
                &instruction.program_id.to_string(),
                &instruction.data,
            ),
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
        Some(&COMPUTE_BUDGET_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("ComputeBudget")
    }
}

fn format_compute_budget_instruction(instruction: &ComputeBudgetInstruction) -> String {
    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            format!("Request Heap Frame: {bytes} bytes")
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            format!("Set Compute Unit Limit: {units} units")
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            format!("Set Compute Unit Price: {micro_lamports} micro-lamports per compute unit")
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            format!("Set Loaded Accounts Data Size Limit: {bytes} bytes")
        }
        ComputeBudgetInstruction::Unused => "Unused Compute Budget Instruction".to_string(),
    }
}

fn create_compute_budget_expanded_fields(
    instruction: &ComputeBudgetInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id).unwrap()];

    // Add specific fields based on instruction type
    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            fields
                .push(create_number_field("Heap Frame Size", &bytes.to_string(), "bytes").unwrap());
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            fields.push(
                create_number_field("Compute Unit Limit", &units.to_string(), "units").unwrap(),
            );
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            fields.push(
                create_number_field(
                    "Price per Compute Unit",
                    &micro_lamports.to_string(),
                    "micro-lamports",
                )
                .unwrap(),
            );
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            fields
                .push(create_number_field("Data Size Limit", &bytes.to_string(), "bytes").unwrap());
        }
        ComputeBudgetInstruction::Unused => {
            // No additional fields for unused instruction
        }
    }

    let hex_fallback_string = hex::encode(data).to_string();
    let raw_data_field = create_raw_data_field(data, Some(hex_fallback_string)).unwrap();

    fields.push(raw_data_field);
    fields
}
