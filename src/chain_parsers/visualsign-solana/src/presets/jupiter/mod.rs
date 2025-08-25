//! Jupiter swap preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::{SwapTokenInfo, get_token_info};
use config::JupiterConfig;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_amount_field, create_number_field, create_raw_data_field, create_text_field,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

#[derive(Debug, Clone)]
pub enum JupiterSwapInstruction {
    Route {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
    },
    ExactOutRoute {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
    },
    SharedAccountsRoute {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
    },
    Unknown,
}

// Create a static instance that we can reference
static JUPITER_CONFIG: JupiterConfig = JupiterConfig;

pub struct JupiterVisualizer;

impl InstructionVisualizer for JupiterVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        // Create account list from instruction accounts
        let instruction_accounts: Vec<String> = instruction
            .accounts
            .iter()
            .map(|account| account.pubkey.to_string())
            .collect();

        let jupiter_instruction =
            parse_jupiter_swap_instruction(&instruction.data, &instruction_accounts)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        let instruction_text = format_jupiter_swap_instruction(&jupiter_instruction);

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
            fields: create_jupiter_swap_expanded_fields(
                &jupiter_instruction,
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
        Some(&JUPITER_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Jupiter")
    }
}

fn parse_jupiter_swap_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.is_empty() {
        return Err("Empty instruction data");
    }

    // Jupiter instructions use an 8-byte discriminator
    if data.len() < 8 {
        return Err("Invalid instruction data length");
    }

    let discriminator = &data[0..8];

    match discriminator {
        // Real-world Jupiter swap discriminator from production data
        [0xc1, 0x20, 0x9b, 0x33, 0x41, 0xd6, 0x9c, 0x81] => {
            parse_jupiter_route_instruction(data, accounts)
        }
        // Route instruction discriminator: 0xe517cb977ae3ad2a
        [0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5] => {
            parse_jupiter_route_instruction(data, accounts)
        }
        // ExactOutRoute instruction discriminator: 0x4bd7dfa80cd0b62a
        [0x2a, 0xb6, 0xd0, 0x0c, 0xa8, 0xdf, 0xd7, 0x4b] => {
            parse_jupiter_exact_out_route_instruction(data, accounts)
        }
        // SharedAccountsRoute instruction discriminator: 0x3af2aaae2fb6d42a
        [0x2a, 0xd4, 0xb6, 0x2f, 0xae, 0xaa, 0xf2, 0x3a] => {
            parse_jupiter_shared_accounts_route_instruction(data, accounts)
        }
        _ => Ok(JupiterSwapInstruction::Unknown),
    }
}

fn parse_jupiter_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 16 {
        return Err("Route instruction data too short");
    }

    // Parse amounts from instruction data
    let in_amount = u64::from_le_bytes([
        data[data.len() - 16],
        data[data.len() - 15],
        data[data.len() - 14],
        data[data.len() - 13],
        data[data.len() - 12],
        data[data.len() - 11],
        data[data.len() - 10],
        data[data.len() - 9],
    ]);
    let quoted_out_amount = u64::from_le_bytes([
        data[data.len() - 8],
        data[data.len() - 7],
        data[data.len() - 6],
        data[data.len() - 5],
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);

    // For Jupiter swaps, we need to infer token accounts from the instruction accounts
    let in_token = if accounts.len() > 0 {
        Some(get_token_info(&accounts[0], in_amount))
    } else {
        None
    };

    let out_token = if accounts.len() > 1 {
        Some(get_token_info(&accounts[1], quoted_out_amount))
    } else {
        None
    };

    Ok(JupiterSwapInstruction::Route {
        in_token,
        out_token,
        slippage_bps: 50, // Default
    })
}

fn parse_jupiter_exact_out_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 16 {
        return Err("ExactOutRoute instruction data too short");
    }

    let in_amount = u64::from_le_bytes([
        data[data.len() - 16],
        data[data.len() - 15],
        data[data.len() - 14],
        data[data.len() - 13],
        data[data.len() - 12],
        data[data.len() - 11],
        data[data.len() - 10],
        data[data.len() - 9],
    ]);
    let out_amount = u64::from_le_bytes([
        data[data.len() - 8],
        data[data.len() - 7],
        data[data.len() - 6],
        data[data.len() - 5],
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);

    let in_token = if accounts.len() > 0 {
        Some(get_token_info(&accounts[0], in_amount))
    } else {
        None
    };

    let out_token = if accounts.len() > 1 {
        Some(get_token_info(&accounts[1], out_amount))
    } else {
        None
    };

    Ok(JupiterSwapInstruction::ExactOutRoute {
        in_token,
        out_token,
        slippage_bps: 50, // Default
    })
}

fn parse_jupiter_shared_accounts_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 16 {
        return Err("SharedAccountsRoute instruction data too short");
    }

    let in_amount = u64::from_le_bytes([
        data[data.len() - 16],
        data[data.len() - 15],
        data[data.len() - 14],
        data[data.len() - 13],
        data[data.len() - 12],
        data[data.len() - 11],
        data[data.len() - 10],
        data[data.len() - 9],
    ]);
    let quoted_out_amount = u64::from_le_bytes([
        data[data.len() - 8],
        data[data.len() - 7],
        data[data.len() - 6],
        data[data.len() - 5],
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);

    let in_token = if accounts.len() > 0 {
        Some(get_token_info(&accounts[0], in_amount))
    } else {
        None
    };

    let out_token = if accounts.len() > 1 {
        Some(get_token_info(&accounts[1], quoted_out_amount))
    } else {
        None
    };

    Ok(JupiterSwapInstruction::SharedAccountsRoute {
        in_token,
        out_token,
        slippage_bps: 50, // Default
    })
}

fn format_jupiter_swap_instruction(instruction: &JupiterSwapInstruction) -> String {
    match instruction {
        JupiterSwapInstruction::Route {
            in_token,
            out_token,
            slippage_bps,
        } => {
            format!(
                "Jupiter Swap: {} {} → {} {} (slippage: {}bps)",
                in_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                in_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                slippage_bps
            )
        }
        JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            format!(
                "Jupiter Exact Out Route: {} {} → {} {} (slippage: {}bps)",
                in_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                in_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                slippage_bps
            )
        }
        JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            format!(
                "Jupiter Shared Accounts Route: {} {} → {} {} (slippage: {}bps)",
                in_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                in_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.amount.to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
                out_token
                    .as_ref()
                    .map(|t| t.symbol.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                slippage_bps
            )
        }
        JupiterSwapInstruction::Unknown => "Jupiter: Unknown Instruction".to_string(),
    }
}

fn create_jupiter_swap_expanded_fields(
    instruction: &JupiterSwapInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id).unwrap()];

    // Add specific fields based on instruction type
    match instruction {
        JupiterSwapInstruction::Route {
            in_token,
            out_token,
            slippage_bps,
        }
        | JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
        }
        | JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            if let Some(token) = in_token {
                fields.push(create_text_field("Input Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field("Input Amount", &token.amount.to_string(), &token.symbol)
                        .unwrap(),
                );
                fields.push(create_text_field("Input Token Name", &token.name).unwrap());
                fields.push(create_text_field("Input Token Address", &token.address).unwrap());
            }

            if let Some(token) = out_token {
                fields.push(create_text_field("Output Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Quoted Output Amount",
                        &token.amount.to_string(),
                        &token.symbol,
                    )
                    .unwrap(),
                );
                fields.push(create_text_field("Output Token Name", &token.name).unwrap());
                fields.push(create_text_field("Output Token Address", &token.address).unwrap());
            }

            fields.push(create_number_field("Slippage", &slippage_bps.to_string(), "bps").unwrap());
        }
        JupiterSwapInstruction::Unknown => {
            fields.push(create_text_field("Status", "Unknown Jupiter instruction type").unwrap());
        }
    }

    let hex_fallback_string = hex::encode(data).to_string();
    let raw_data_field = create_raw_data_field(data, Some(hex_fallback_string)).unwrap();
    fields.push(raw_data_field);
    fields
}
