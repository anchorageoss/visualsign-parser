//! SPL Token preset implementation for Solana
//! Handles the Token Program (TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::SplTokenConfig;
use solana_program::program_option::COption;
use spl_token::instruction::{AuthorityType, TokenInstruction};
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

fn format_authority_type(authority_type: &AuthorityType) -> &'static str {
    match authority_type {
        AuthorityType::MintTokens => "Mint Tokens",
        AuthorityType::FreezeAccount => "Freeze Account",
        AuthorityType::AccountOwner => "Account Owner",
        AuthorityType::CloseAccount => "Close Account",
    }
}

fn format_coption_pubkey(coption: &COption<solana_sdk::pubkey::Pubkey>) -> String {
    match coption {
        COption::Some(pubkey) => pubkey.to_string(),
        COption::None => "None".to_string(),
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

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", "Mint To")?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // MintTo accounts: [0] mint, [1] destination account, [2] mint authority
            if let Some(mint_account) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("mint", &mint_account.pubkey.to_string())?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("account", &destination.pubkey.to_string())?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("mintAuthority", &authority.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

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

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", "Mint To (Checked)")?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // MintToChecked accounts: [0] mint, [1] destination account, [2] mint authority
            if let Some(mint_account) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("mint", &mint_account.pubkey.to_string())?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("account", &destination.pubkey.to_string())?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("mintAuthority", &authority.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::SetAuthority {
            authority_type,
            new_authority,
        } => {
            let authority_type_str = format_authority_type(authority_type);
            let new_authority_str = format_coption_pubkey(new_authority);
            let instruction_name = format!("Set Authority: {}", authority_type_str);

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", "Set Authority")?,
                create_text_field("Authority Type", authority_type_str)?,
                create_text_field("New Authority", &new_authority_str)?,
            ];

            // SetAuthority accounts: [0] account whose authority is being set, [1] current authority
            if let Some(account) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Account", &account.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Transfer { amount } => {
            let instruction_name = "Transfer";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Transfer accounts: [0] source account, [1] destination account, [2] owner
            if let Some(source) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Source", &source.pubkey.to_string())?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Destination", &destination.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::TransferChecked { amount, decimals } => {
            let instruction_name = "Transfer (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // TransferChecked accounts: [0] source account, [1] mint, [2] destination account, [3] owner
            if let Some(source) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Source", &source.pubkey.to_string())?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", &mint.pubkey.to_string())?);
            }
            if let Some(destination) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Destination", &destination.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Burn { amount } => {
            let instruction_name = "Burn";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Burn accounts: [0] token account to burn from, [1] mint, [2] owner
            if let Some(account) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Account", &account.pubkey.to_string())?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", &mint.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::BurnChecked { amount, decimals } => {
            let instruction_name = "Burn (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // BurnChecked accounts: [0] token account to burn from, [1] mint, [2] owner
            if let Some(account) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Account", &account.pubkey.to_string())?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", &mint.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Approve { amount } => {
            let instruction_name = "Approve";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Approve accounts: [0] source account, [1] delegate, [2] owner
            if let Some(source) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Source", &source.pubkey.to_string())?);
            }
            if let Some(delegate) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Delegate", &delegate.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::ApproveChecked { amount, decimals } => {
            let instruction_name = "Approve (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // ApproveChecked accounts: [0] source account, [1] mint, [2] delegate, [3] owner
            if let Some(source) = instruction.accounts.get(0) {
                expanded_fields.push(create_text_field("Source", &source.pubkey.to_string())?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", &mint.pubkey.to_string())?);
            }
            if let Some(delegate) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Delegate", &delegate.pubkey.to_string())?);
            }

            expanded_fields.push(create_text_field("Raw Data", &hex::encode(&instruction.data))?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
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
        // Note: MintTo and MintToChecked are handled specially in create_token_preview_layout
        // and never reach this function, so they are intentionally omitted here
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
        // These cases are handled specially above and should never reach here
        TokenInstruction::MintTo { .. } | TokenInstruction::MintToChecked { .. } => {
            unreachable!("MintTo instructions are handled specially in create_token_preview_layout")
        }
    }
}

#[cfg(test)]
mod tests;
