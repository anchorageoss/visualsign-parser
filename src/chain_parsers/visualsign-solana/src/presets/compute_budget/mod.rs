//! Compute Budget preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
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
        let compute_budget_instruction =
            ComputeBudgetInstruction::try_from_slice(context.data()).map_err(|e| {
                VisualSignError::DecodeError(format!(
                    "Failed to parse compute budget instruction: {e}"
                ))
            })?;

        create_compute_budget_preview_layout(&compute_budget_instruction, context)
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

fn create_compute_budget_preview_layout(
    instruction: &ComputeBudgetInstruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id_str = match context.program_id() {
        ProgramRef::Resolved(pk) => pk.to_string(),
        ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
    };
    let instruction_text = format_compute_budget_instruction(instruction);

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

    let mut expanded_fields = vec![create_text_field("Program ID", &program_id_str)?];

    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            expanded_fields
                .push(create_number_field("Heap Frame Size", &bytes.to_string(), "bytes")?);
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            expanded_fields.push(create_number_field(
                "Compute Unit Limit",
                &units.to_string(),
                "units",
            )?);
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            expanded_fields.push(create_number_field(
                "Price per Compute Unit",
                &micro_lamports.to_string(),
                "micro-lamports",
            )?);
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            expanded_fields
                .push(create_number_field("Data Size Limit", &bytes.to_string(), "bytes")?);
        }
        ComputeBudgetInstruction::Unused => {}
    }

    let hex_fallback_string = hex::encode(context.data());
    expanded_fields.push(create_raw_data_field(context.data(), Some(hex_fallback_string))?);

    let expanded = SignablePayloadFieldListLayout {
        fields: expanded_fields,
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
