//! Token 2022 preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::format_token_amount;
use config::Token2022Config;
use solana_sdk::instruction::AccountMeta;
use spl_token_2022::instruction::TokenInstruction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_number_field, create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

static TOKEN_2022_CONFIG: Token2022Config = Token2022Config;

// Token 2022 extension instruction discriminators
const PAUSABLE_EXTENSION_DISCRIMINATOR: u8 = 44;
const PAUSABLE_PAUSE_DISCRIMINATOR: u8 = 1;
const PAUSABLE_RESUME_DISCRIMINATOR: u8 = 2;

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
    Pause {
        mint: String,
        pause_authority: String,
    },
    Resume {
        mint: String,
        pause_authority: String,
    },
    Freeze {
        account: String,
        mint: String,
        freeze_authority: String,
    },
    Thaw {
        account: String,
        mint: String,
        freeze_authority: String,
    },
}

fn parse_token_2022_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<Token2022Instruction, String> {
    // Check for Pause instruction
    if is_pause_instruction(data) {
        if accounts.len() < 2 {
            return Err("Invalid pause: insufficient accounts".to_string());
        }

        return Ok(Token2022Instruction::Pause {
            mint: accounts[0].pubkey.to_string(),
            pause_authority: accounts[1].pubkey.to_string(),
        });
    }

    // Check for Resume instruction
    if is_resume_instruction(data) {
        if accounts.len() < 2 {
            return Err("Invalid resume: insufficient accounts".to_string());
        }

        return Ok(Token2022Instruction::Resume {
            mint: accounts[0].pubkey.to_string(),
            pause_authority: accounts[1].pubkey.to_string(),
        });
    }

    // Try to parse as standard TokenInstruction first
    if let Ok(sdk_instruction) = TokenInstruction::unpack(data) {
        match sdk_instruction {
            TokenInstruction::MintToChecked { amount, decimals } => {
                if accounts.len() < 3 {
                    return Err("Invalid mintToChecked: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::MintToChecked {
                    amount,
                    decimals,
                    mint: accounts[0].pubkey.to_string(),
                    account: accounts[1].pubkey.to_string(),
                    mint_authority: accounts[2].pubkey.to_string(),
                });
            }
            TokenInstruction::BurnChecked { amount, decimals } => {
                if accounts.len() < 3 {
                    return Err("Invalid burnChecked: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::BurnChecked {
                    amount,
                    decimals,
                    account: accounts[0].pubkey.to_string(),
                    mint: accounts[1].pubkey.to_string(),
                    authority: accounts[2].pubkey.to_string(),
                });
            }
            TokenInstruction::FreezeAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid freezeAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::Freeze {
                    account: accounts[0].pubkey.to_string(),
                    mint: accounts[1].pubkey.to_string(),
                    freeze_authority: accounts[2].pubkey.to_string(),
                });
            }
            TokenInstruction::ThawAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid thawAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::Thaw {
                    account: accounts[0].pubkey.to_string(),
                    mint: accounts[1].pubkey.to_string(),
                    freeze_authority: accounts[2].pubkey.to_string(),
                });
            }
            _ => {
                // Not a standard TokenInstruction, return error
                return Err(format!(
                    "Unsupported Token 2022 instruction: unknown discriminator {}",
                    if data.is_empty() {
                        "empty".to_string()
                    } else {
                        format!("0x{:02x}", data[0])
                    }
                ));
            }
        }
    }

    Err(format!(
        "Unsupported Token 2022 instruction: unknown discriminator {}",
        if data.is_empty() {
            "empty".to_string()
        } else {
            format!("0x{:02x}", data[0])
        }
    ))
}

// Check if the instruction is a Pause instruction
fn is_pause_instruction(data: &[u8]) -> bool {
    !data.is_empty()
        && data[0] == PAUSABLE_EXTENSION_DISCRIMINATOR
        && data.len() > 1
        && data[1] == PAUSABLE_PAUSE_DISCRIMINATOR
}

// Check if the instruction is a Resume instruction
fn is_resume_instruction(data: &[u8]) -> bool {
    !data.is_empty()
        && data[0] == PAUSABLE_EXTENSION_DISCRIMINATOR
        && data.len() > 1
        && data[1] == PAUSABLE_RESUME_DISCRIMINATOR
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
        Token2022Instruction::Pause {
            mint,
            pause_authority,
        } => {
            let title = "Pause Token".to_string();

            let condensed = vec![create_text_field("Action", "Pause Token")?];

            let expanded = vec![
                create_text_field("Instruction", "Pause")?,
                create_text_field("Mint", mint)?,
                create_text_field("Pause Authority", pause_authority)?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::Resume {
            mint,
            pause_authority,
        } => {
            let title = "Resume Token".to_string();

            let condensed = vec![create_text_field("Action", "Resume Token")?];

            let expanded = vec![
                create_text_field("Instruction", "Resume")?,
                create_text_field("Mint", mint)?,
                create_text_field("Pause Authority", pause_authority)?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::Freeze {
            account,
            mint,
            freeze_authority,
        } => {
            let title = "Freeze Account".to_string();

            let condensed = vec![create_text_field("Action", "Freeze Account")?];

            let expanded = vec![
                create_text_field("Instruction", "Freeze")?,
                create_text_field("Token Account", account)?,
                create_text_field("Mint", mint)?,
                create_text_field("Freeze Authority", freeze_authority)?,
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::Thaw {
            account,
            mint,
            freeze_authority,
        } => {
            let title = "Thaw Account".to_string();

            let condensed = vec![create_text_field("Action", "Thaw Account")?];

            let expanded = vec![
                create_text_field("Instruction", "Thaw")?,
                create_text_field("Token Account", account)?,
                create_text_field("Mint", mint)?,
                create_text_field("Freeze Authority", freeze_authority)?,
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
