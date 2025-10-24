//! SPL Token preset implementation for Solana
//! Handles the Token Program (TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::SplTokenConfig;
use spl_token::instruction::TokenInstruction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::*;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

// Create a static instance that we can reference
static SPL_TOKEN_CONFIG: SplTokenConfig = SplTokenConfig;

pub struct SplTokenVisualizer;

impl InstructionVisualizer for SplTokenVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        let token_instruction = TokenInstruction::unpack(&instruction.data).map_err(|e| {
            VisualSignError::DecodeError(format!("Failed to unpack SPL token instruction: {e}"))
        })?;

        create_token_preview_layout(&token_instruction, instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SPL_TOKEN_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("SplToken")
    }
}

fn create_token_preview_layout(
    token_instruction: &TokenInstruction,
    instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    match token_instruction {
        TokenInstruction::MintTo { amount } => {
            let instruction_name = format!("Mint To: {amount}");

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", "Mint To")?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Raw Data", &hex::encode(&instruction.data))?,
            ];

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::MintToChecked { amount, decimals } => {
            let instruction_name = format!("Mint To: {amount} (decimals: {decimals})");

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", "Mint To (Checked)")?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
                create_text_field("Raw Data", &hex::encode(&instruction.data))?,
            ];

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        _ => {
            // Handle other token instructions with basic layout
            let instruction_name = format_token_instruction(token_instruction);

            let condensed_fields = vec![
                create_text_field("Instruction", &instruction_name)?,
                create_text_field("Program", "SPL Token")?,
            ];

            let expanded_fields = vec![
                create_text_field("Instruction", &instruction_name)?,
                create_text_field("Program", "SPL Token")?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Raw Data", &hex::encode(&instruction.data))?,
            ];

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
    }
}

fn create_preview_layout_field(
    title: &str,
    condensed_fields: Vec<AnnotatedPayloadField>,
    expanded_fields: Vec<AnnotatedPayloadField>,
    instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let condensed = SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title.to_string(),
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
                fallback_text: format!(
                    "Program ID: {}\nData: {}",
                    instruction.program_id,
                    hex::encode(&instruction.data)
                ),
            },
            preview_layout,
        },
    })
}

fn format_token_instruction(instruction: &TokenInstruction) -> String {
    match instruction {
        TokenInstruction::InitializeMint { .. } => "Initialize Mint".to_string(),
        TokenInstruction::InitializeMint2 { .. } => "Initialize Mint (v2)".to_string(),
        TokenInstruction::InitializeAccount => "Initialize Token Account".to_string(),
        TokenInstruction::InitializeAccount2 { .. } => "Initialize Token Account (v2)".to_string(),
        TokenInstruction::InitializeAccount3 { .. } => "Initialize Token Account (v3)".to_string(),
        TokenInstruction::InitializeMultisig { .. } => "Initialize Multisig".to_string(),
        TokenInstruction::InitializeMultisig2 { .. } => "Initialize Multisig (v2)".to_string(),
        TokenInstruction::Transfer { .. } => "Transfer".to_string(),
        TokenInstruction::TransferChecked { .. } => "Transfer (Checked)".to_string(),
        TokenInstruction::Approve { .. } => "Approve".to_string(),
        TokenInstruction::ApproveChecked { .. } => "Approve (Checked)".to_string(),
        TokenInstruction::Revoke => "Revoke".to_string(),
        TokenInstruction::SetAuthority { .. } => "Set Authority".to_string(),
        TokenInstruction::MintTo { .. } => "Mint To".to_string(),
        TokenInstruction::MintToChecked { .. } => "Mint To (Checked)".to_string(),
        TokenInstruction::Burn { .. } => "Burn".to_string(),
        TokenInstruction::BurnChecked { .. } => "Burn (Checked)".to_string(),
        TokenInstruction::CloseAccount => "Close Account".to_string(),
        TokenInstruction::FreezeAccount => "Freeze Account".to_string(),
        TokenInstruction::ThawAccount => "Thaw Account".to_string(),
        TokenInstruction::SyncNative => "Sync Native".to_string(),
        TokenInstruction::GetAccountDataSize { .. } => "Get Account Data Size".to_string(),
        TokenInstruction::InitializeImmutableOwner => "Initialize Immutable Owner".to_string(),
        TokenInstruction::AmountToUiAmount { .. } => "Amount To UI Amount".to_string(),
        TokenInstruction::UiAmountToAmount { .. } => "UI Amount To Amount".to_string(),
    }
}
