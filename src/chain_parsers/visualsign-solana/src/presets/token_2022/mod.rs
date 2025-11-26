//! Token 2022 preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::format_token_amount;
use config::Token2022Config;
use solana_sdk::instruction::AccountMeta;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_number_field, create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

static TOKEN_2022_CONFIG: Token2022Config = Token2022Config;

pub struct Token2022Visualizer;

impl InstructionVisualizer for Token2022Visualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        // Parse the Token 2022 instruction
        let token_2022_instruction =
            parse_token_2022_instruction(&instruction.data, &instruction.accounts)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        // Generate proper preview layout
        create_token_2022_preview_layout(&token_2022_instruction, instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&TOKEN_2022_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Token2022")
    }
}

enum Token2022Instruction {
    MintToChecked {
        amount: u64,
        decimals: u8,
        mint: String,
        account: String,
        mint_authority: String,
    },
    BurnChecked {
        amount: u64,
        decimals: u8,
        account: String,
        mint: String,
        authority: String,
    },
}

fn parse_token_2022_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<Token2022Instruction, String> {
    if data.is_empty() {
        return Err("Empty instruction data".to_string());
    }

    // Token 2022 instruction discriminators:
    // mintToChecked = 14 (0x0E)
    // burnChecked = 15 (0x0F)
    match data[0] {
        14 => {
            // mintToChecked instruction format:
            // [0] instruction discriminator (14)
            // [1-8] amount (u64, little-endian)
            // [9] decimals (u8)
            if data.len() < 10 {
                return Err("Invalid mintToChecked instruction: insufficient data".to_string());
            }

            let amount = u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);
            let decimals = data[9];

            if accounts.len() < 3 {
                return Err("Invalid mintToChecked: insufficient accounts".to_string());
            }

            Ok(Token2022Instruction::MintToChecked {
                amount,
                decimals,
                mint: accounts[0].pubkey.to_string(),
                account: accounts[1].pubkey.to_string(),
                mint_authority: accounts[2].pubkey.to_string(),
            })
        }
        15 => {
            // burnChecked instruction format:
            // [0] instruction discriminator (15)
            // [1-8] amount (u64, little-endian)
            // [9] decimals (u8)
            if data.len() < 10 {
                return Err("Invalid burnChecked instruction: insufficient data".to_string());
            }

            let amount = u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);
            let decimals = data[9];

            if accounts.len() < 3 {
                return Err("Invalid burnChecked: insufficient accounts".to_string());
            }

            Ok(Token2022Instruction::BurnChecked {
                amount,
                decimals,
                account: accounts[0].pubkey.to_string(),
                mint: accounts[1].pubkey.to_string(),
                authority: accounts[2].pubkey.to_string(),
            })
        }
        _ => {
            let instruction_discriminator = data[0];
            Err(format!("Unsupported Token 2022 instruction: {instruction_discriminator}"))
        }
    }
}

fn create_token_2022_preview_layout(
    parsed: &Token2022Instruction,
    instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let (title, condensed_fields, expanded_fields) = match parsed {
        Token2022Instruction::MintToChecked {
            amount,
            decimals,
            mint,
            account,
            mint_authority,
        } => {
            let formatted_amount = format_token_amount(*amount, *decimals);
            let title = format!("Mint To Checked: {formatted_amount} tokens");

            let condensed = vec![
                create_text_field("Action", "Mint To Checked")?,
                create_text_field("Amount", &formatted_amount)?,
            ];

            let expanded = vec![
                create_text_field("Instruction", "Mint To Checked")?,
                create_text_field("Amount", &formatted_amount)?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_number_field("Decimals", &decimals.to_string(), "")?,
                create_text_field("Mint", mint)?,
                create_text_field("Destination Account", account)?,
                create_text_field("Mint Authority", mint_authority)?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::BurnChecked {
            amount,
            decimals,
            account,
            mint,
            authority,
        } => {
            let formatted_amount = format_token_amount(*amount, *decimals);
            let title = format!("Burn Checked: {formatted_amount} tokens");

            let condensed = vec![
                create_text_field("Action", "Burn Checked")?,
                create_text_field("Amount", &formatted_amount)?,
            ];

            let expanded = vec![
                create_text_field("Instruction", "Burn Checked")?,
                create_text_field("Amount", &formatted_amount)?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_number_field("Decimals", &decimals.to_string(), "")?,
                create_text_field("Token Account", account)?,
                create_text_field("Mint", mint)?,
                create_text_field("Authority", authority)?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title.clone(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(SignablePayloadFieldListLayout {
            fields: condensed_fields,
        }),
        expanded: Some(SignablePayloadFieldListLayout {
            fields: expanded_fields,
        }),
    };

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: {
                    let instruction_num = context.instruction_index() + 1;
                    format!("Instruction {instruction_num}")
                },
                fallback_text: {
                    let program_id = instruction.program_id;
                    format!("Token 2022: {title}\nProgram ID: {program_id}")
                },
            },
            preview_layout,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    mod fixture_test;
}
