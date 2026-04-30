//! Jupiter swap preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::{SwapTokenInfo, get_token_info};
use config::JupiterSwapConfig;
use solana_parser::{Idl, decode_idl_data, parse_instruction_with_idl};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_amount_field, create_number_field, create_raw_data_field, create_text_field,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

/// Jupiter v6 program ID
pub(crate) const JUPITER_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

#[derive(Debug, Clone)]
pub enum JupiterSwapInstruction {
    Route {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u8,
    },
    ExactOutRoute {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u8,
    },
    SharedAccountsRoute {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u8,
    },
    RouteV2 {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u16,
        positive_slippage_bps: u16,
    },
    ExactOutRouteV2 {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u16,
        positive_slippage_bps: u16,
    },
    SharedAccountsRouteV2 {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u16,
        positive_slippage_bps: u16,
    },
    SharedAccountsExactOutRouteV2 {
        in_token: Option<SwapTokenInfo>,
        out_token: Option<SwapTokenInfo>,
        slippage_bps: u16,
        platform_fee_bps: u16,
        positive_slippage_bps: u16,
    },
    Unknown {
        /// Optional instruction name from IDL if available
        instruction_name: Option<String>,
    },
}

// Create a static instance that we can reference
static JUPITER_CONFIG: JupiterSwapConfig = JupiterSwapConfig;

pub struct JupiterSwapVisualizer;

impl InstructionVisualizer for JupiterSwapVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

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
            fields: vec![
                create_text_field("Instruction", &instruction_text)
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            ],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: create_jupiter_swap_expanded_fields(
                &jupiter_instruction,
                &instruction.program_id.to_string(),
                &instruction.data,
            )?,
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

        let fallback_text = format!(
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
                    fallback_text,
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

/// Jupiter v6 IDL refreshed from mainnet.
/// Kept locally (not sourced from `solana_parser::ProgramType`) so we can pick up
/// newer instructions like `route_v2` without bumping the upstream rev.
/// Also used to override the stale IDL bundled inside `solana_parser` when that
/// crate parses a full transaction (see `decode_v0_transfers`).
pub(crate) const JUPITER_IDL_JSON: &str = include_str!("jupiter_agg_v6.json");

fn get_jupiter_idl() -> Option<Idl> {
    decode_idl_data(JUPITER_IDL_JSON).ok()
}

/// Helper to extract u64 argument from parsed IDL args
fn extract_u64_arg(
    args: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    args.get(name)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("Missing or invalid argument: {name}").into())
}

/// Parse Jupiter instruction using IDL-based approach
fn parse_jupiter_instruction_with_idl(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, Box<dyn std::error::Error>> {
    let idl = get_jupiter_idl().ok_or("Jupiter IDL not available")?;

    // Parse using solana_parser
    let parsed = parse_instruction_with_idl(data, JUPITER_PROGRAM_ID, &idl)?;

    // Extract instruction type and arguments
    match parsed.instruction_name.as_str() {
        "route" => {
            let in_amount = extract_u64_arg(&parsed.program_call_args, "in_amount")?;
            let quoted_out_amount =
                extract_u64_arg(&parsed.program_call_args, "quoted_out_amount")?;
            let slippage_bps =
                u16::try_from(extract_u64_arg(&parsed.program_call_args, "slippage_bps")?)?;
            let platform_fee_bps = u8::try_from(extract_u64_arg(
                &parsed.program_call_args,
                "platform_fee_bps",
            )?)?;

            // Get token info (preserve current logic)
            let in_token = accounts.first().map(|addr| get_token_info(addr, in_amount));
            let out_token = accounts
                .get(5)
                .map(|addr| get_token_info(addr, quoted_out_amount));

            Ok(JupiterSwapInstruction::Route {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
            })
        }
        "exact_out_route" => {
            // Note: exact_out_route uses out_amount and quoted_in_amount (reversed)
            let out_amount = extract_u64_arg(&parsed.program_call_args, "out_amount")?;
            let quoted_in_amount = extract_u64_arg(&parsed.program_call_args, "quoted_in_amount")?;
            let slippage_bps =
                u16::try_from(extract_u64_arg(&parsed.program_call_args, "slippage_bps")?)?;
            let platform_fee_bps = u8::try_from(extract_u64_arg(
                &parsed.program_call_args,
                "platform_fee_bps",
            )?)?;

            let in_token = accounts
                .first()
                .map(|addr| get_token_info(addr, quoted_in_amount));
            let out_token = accounts.get(5).map(|addr| get_token_info(addr, out_amount));

            Ok(JupiterSwapInstruction::ExactOutRoute {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
            })
        }
        "shared_accounts_route" => {
            let in_amount = extract_u64_arg(&parsed.program_call_args, "in_amount")?;
            let quoted_out_amount =
                extract_u64_arg(&parsed.program_call_args, "quoted_out_amount")?;
            let slippage_bps =
                u16::try_from(extract_u64_arg(&parsed.program_call_args, "slippage_bps")?)?;
            let platform_fee_bps = u8::try_from(extract_u64_arg(
                &parsed.program_call_args,
                "platform_fee_bps",
            )?)?;

            let in_token = accounts.first().map(|addr| get_token_info(addr, in_amount));
            let out_token = accounts
                .get(5)
                .map(|addr| get_token_info(addr, quoted_out_amount));

            Ok(JupiterSwapInstruction::SharedAccountsRoute {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
            })
        }
        "route_v2" => parse_route_v2(&parsed.program_call_args, accounts, false, false),
        "exact_out_route_v2" => parse_route_v2(&parsed.program_call_args, accounts, true, false),
        "shared_accounts_route_v2" => {
            parse_route_v2(&parsed.program_call_args, accounts, false, true)
        }
        "shared_accounts_exact_out_route_v2" => {
            parse_route_v2(&parsed.program_call_args, accounts, true, true)
        }
        _ => Ok(JupiterSwapInstruction::Unknown {
            instruction_name: Some(parsed.instruction_name.clone()),
        }),
    }
}

/// Parse any of the four v2 route variants. `exact_out` toggles amount field
/// names and in/out token assignment; `shared` shifts mint account indices.
fn parse_route_v2(
    args: &serde_json::Map<String, serde_json::Value>,
    accounts: &[String],
    exact_out: bool,
    shared: bool,
) -> Result<JupiterSwapInstruction, Box<dyn std::error::Error>> {
    let slippage_bps = u16::try_from(extract_u64_arg(args, "slippage_bps")?)?;
    let platform_fee_bps = u16::try_from(extract_u64_arg(args, "platform_fee_bps")?)?;
    let positive_slippage_bps = u16::try_from(extract_u64_arg(args, "positive_slippage_bps")?)?;

    // Mint indices differ between shared and non-shared v2 variants (see IDL).
    let (source_mint_idx, destination_mint_idx) = if shared { (6, 7) } else { (3, 4) };

    // exact_out uses out_amount / quoted_in_amount; non-exact_out uses in_amount / quoted_out_amount.
    let (in_amount, out_amount) = if exact_out {
        let quoted_in_amount = extract_u64_arg(args, "quoted_in_amount")?;
        let out_amount = extract_u64_arg(args, "out_amount")?;
        (quoted_in_amount, out_amount)
    } else {
        let in_amount = extract_u64_arg(args, "in_amount")?;
        let quoted_out_amount = extract_u64_arg(args, "quoted_out_amount")?;
        (in_amount, quoted_out_amount)
    };

    let in_token = accounts
        .get(source_mint_idx)
        .map(|addr| get_token_info(addr, in_amount));
    let out_token = accounts
        .get(destination_mint_idx)
        .map(|addr| get_token_info(addr, out_amount));

    Ok(match (exact_out, shared) {
        (false, false) => JupiterSwapInstruction::RouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        },
        (true, false) => JupiterSwapInstruction::ExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        },
        (false, true) => JupiterSwapInstruction::SharedAccountsRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        },
        (true, true) => JupiterSwapInstruction::SharedAccountsExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        },
    })
}

fn parse_jupiter_swap_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 8 {
        return Err("Invalid instruction data length");
    }

    match parse_jupiter_instruction_with_idl(data, accounts) {
        Ok(instruction) => Ok(instruction),
        Err(e) => {
            tracing::warn!("Failed to parse Jupiter instruction with IDL: {e}");
            Ok(JupiterSwapInstruction::Unknown {
                instruction_name: None,
            })
        }
    }
}

fn format_jupiter_swap_instruction(instruction: &JupiterSwapInstruction) -> String {
    match instruction {
        JupiterSwapInstruction::Route {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        }
        | JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        }
        | JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        } => {
            let instruction_type = match instruction {
                JupiterSwapInstruction::Route { .. } => "Jupiter Swap",
                JupiterSwapInstruction::ExactOutRoute { .. } => "Jupiter Exact Out Route",
                JupiterSwapInstruction::SharedAccountsRoute { .. } => {
                    "Jupiter Shared Accounts Route"
                }
                _ => unreachable!(),
            };

            let mut result = format!(
                "{}: From {} {} To {} {} (slippage: {}bps",
                instruction_type,
                format_token_amount(in_token),
                format_token_symbol(in_token),
                format_token_amount(out_token),
                format_token_symbol(out_token),
                slippage_bps
            );

            if *platform_fee_bps > 0 {
                result.push_str(&format!(", platform fee: {platform_fee_bps}bps"));
            }

            result.push(')');
            result
        }
        JupiterSwapInstruction::RouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::ExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::SharedAccountsRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::SharedAccountsExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        } => {
            let instruction_type = match instruction {
                JupiterSwapInstruction::RouteV2 { .. } => "Jupiter Swap V2",
                JupiterSwapInstruction::ExactOutRouteV2 { .. } => "Jupiter Exact Out Route V2",
                JupiterSwapInstruction::SharedAccountsRouteV2 { .. } => {
                    "Jupiter Shared Accounts Route V2"
                }
                JupiterSwapInstruction::SharedAccountsExactOutRouteV2 { .. } => {
                    "Jupiter Shared Accounts Exact Out Route V2"
                }
                _ => unreachable!(),
            };

            let mut result = format!(
                "{}: From {} {} To {} {} (slippage: {}bps",
                instruction_type,
                format_token_amount(in_token),
                format_token_symbol(in_token),
                format_token_amount(out_token),
                format_token_symbol(out_token),
                slippage_bps
            );

            if *platform_fee_bps > 0 {
                result.push_str(&format!(", platform fee: {platform_fee_bps}bps"));
            }
            if *positive_slippage_bps > 0 {
                result.push_str(&format!(", positive slippage: {positive_slippage_bps}bps"));
            }

            result.push(')');
            result
        }
        JupiterSwapInstruction::Unknown { instruction_name } => {
            if let Some(name) = instruction_name {
                format!("Jupiter: {name}")
            } else {
                "Jupiter: Unknown Instruction".to_string()
            }
        }
    }
}

fn format_token_amount(token: &Option<SwapTokenInfo>) -> String {
    token
        .as_ref()
        .map(|t| t.amount.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn format_token_symbol(token: &Option<SwapTokenInfo>) -> String {
    token
        .as_ref()
        .map(|t| t.symbol.clone())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn create_jupiter_swap_expanded_fields(
    instruction: &JupiterSwapInstruction,
    program_id: &str,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    ];

    match instruction {
        JupiterSwapInstruction::Route {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        }
        | JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        }
        | JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
        } => {
            // Add input token fields
            if let Some(token) = in_token {
                fields.extend([
                    create_text_field("Input Token", &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_amount_field("Input Amount", &token.amount.to_string(), &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Input Token Name", &token.name)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    // TODO: Add back Input Token Address
                ]);
            }

            // Add output token fields
            if let Some(token) = out_token {
                fields.extend([
                    create_text_field("Output Token", &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_amount_field(
                        "Quoted Output Amount",
                        &token.amount.to_string(),
                        &token.symbol,
                    )
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Output Token Name", &token.name)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Output Token Address", &token.address)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                ]);
            }

            // Add slippage field
            fields.push(
                create_number_field("Slippage", &slippage_bps.to_string(), "bps")
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );

            // Add platform fee field if non-zero
            if *platform_fee_bps > 0 {
                fields.push(
                    create_number_field("Platform Fee", &platform_fee_bps.to_string(), "bps")
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                );
            }
        }
        JupiterSwapInstruction::RouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::ExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::SharedAccountsRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        }
        | JupiterSwapInstruction::SharedAccountsExactOutRouteV2 {
            in_token,
            out_token,
            slippage_bps,
            platform_fee_bps,
            positive_slippage_bps,
        } => {
            if let Some(token) = in_token {
                fields.extend([
                    create_text_field("Input Token", &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_amount_field("Input Amount", &token.amount.to_string(), &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Input Token Name", &token.name)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Input Token Address", &token.address)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                ]);
            }

            if let Some(token) = out_token {
                fields.extend([
                    create_text_field("Output Token", &token.symbol)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_amount_field(
                        "Quoted Output Amount",
                        &token.amount.to_string(),
                        &token.symbol,
                    )
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Output Token Name", &token.name)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                    create_text_field("Output Token Address", &token.address)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                ]);
            }

            fields.push(
                create_number_field("Slippage", &slippage_bps.to_string(), "bps")
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );

            if *platform_fee_bps > 0 {
                fields.push(
                    create_number_field("Platform Fee", &platform_fee_bps.to_string(), "bps")
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                );
            }

            if *positive_slippage_bps > 0 {
                fields.push(
                    create_number_field(
                        "Positive Slippage",
                        &positive_slippage_bps.to_string(),
                        "bps",
                    )
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
                );
            }
        }
        JupiterSwapInstruction::Unknown { instruction_name } => {
            let status_text = if let Some(name) = instruction_name {
                format!("Jupiter instruction: {name} (not explicitly handled)")
            } else {
                "Unknown Jupiter instruction type".to_string()
            };
            fields.push(
                create_text_field("Status", &status_text)
                    .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
            );
        }
    }

    // Add raw data field
    fields.push(
        create_raw_data_field(data, Some(hex::encode(data)))
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
    );

    Ok(fields)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    mod fixture_test;

    /// Real instruction data from sample_route.json fixture (WSOL -> USELESS swap)
    fn fixture_instruction_data() -> Vec<u8> {
        hex::decode("e517cb977ae3ad2a010000002f010064000180841e00000000003da9170000000000320000")
            .expect("valid hex")
    }

    /// Route fixture body bytes (after discriminator) — reusable across instruction types
    fn fixture_route_plan_body() -> Vec<u8> {
        let full = fixture_instruction_data();
        full[8..].to_vec()
    }

    /// Accounts from sample_route.json fixture (need at least indices 0 and 5)
    fn fixture_accounts() -> Vec<String> {
        vec![
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            "B7hSadyLX8YhNT8RDcK8RbnR3KAfX4HbWvV89XmeqitA".to_string(),
            "3c5JEJ3un3HZAtWvZ77nhNGxDGqmWM7uZ1cx4bGDsKE8".to_string(),
            "FAXnNWMXbadmfMTfWtEu3WDymtRwsxYLGdbKoJbfLKsK".to_string(),
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string(),
            "Dz9mQ9NzkBcCsuGPFJ3r1bS4wgqKMHBPiVuniW8Mbonk".to_string(),
        ]
    }

    #[test]
    fn test_jupiter_swap_instruction_parsing() {
        let data = fixture_instruction_data();
        let accounts = fixture_accounts();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match parsed {
            JupiterSwapInstruction::Route { slippage_bps, .. } => {
                assert_eq!(slippage_bps, 50, "Slippage should be 50 bps");
            }
            _ => panic!("Expected Route instruction, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(formatted.contains("Jupiter"), "Should contain 'Jupiter'");
        assert!(formatted.contains("50bps"), "Should contain slippage");

        let fields = create_jupiter_swap_expanded_fields(
            &parsed,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &data,
        )
        .unwrap();

        let has_program_id = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, .. } = &f.signable_payload_field {
                common.label == "Program ID"
            } else {
                false
            }
        });
        assert!(has_program_id, "Should have Program ID field");

        let has_slippage = fields.iter().any(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Slippage"
            } else {
                false
            }
        });
        assert!(has_slippage, "Should have Slippage field");
    }

    #[test]
    fn test_jupiter_instruction_with_real_data() {
        use serde_json::json;

        let data = fixture_instruction_data();
        let accounts = fixture_accounts();

        let result = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match result {
            JupiterSwapInstruction::Route { slippage_bps, .. } => {
                assert_eq!(slippage_bps, 50);

                let fields = create_jupiter_swap_expanded_fields(
                    &result,
                    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                    &data,
                )
                .unwrap();

                let fields_json = serde_json::to_value(&fields).unwrap();
                assert!(
                    fields_json.is_array(),
                    "Fields should serialize to JSON array"
                );
                let fields_array = fields_json.as_array().unwrap();
                assert!(fields_array.len() >= 3, "Should have at least 3 fields");

                let has_program_id = fields_array.iter().any(|field| {
                    field.get("Label").and_then(|l| l.as_str()) == Some("Program ID")
                        && field.get("Type").and_then(|t| t.as_str()) == Some("text_v2")
                });

                let has_slippage = fields_array.iter().any(|field| {
                    field.get("Label").and_then(|l| l.as_str()) == Some("Slippage")
                        && field.get("Type").and_then(|t| t.as_str()) == Some("number")
                });

                assert!(
                    has_program_id,
                    "Should have Program ID field in JSON structure"
                );
                assert!(has_slippage, "Should have Slippage field in JSON structure");

                let expected_program_id_field = json!({
                    "Label": "Program ID",
                    "Type": "text_v2"
                });

                let program_id_field = fields_array
                    .iter()
                    .find(|field| field.get("Label").and_then(|l| l.as_str()) == Some("Program ID"))
                    .unwrap();

                assert_eq!(
                    program_id_field.get("Label"),
                    expected_program_id_field.get("Label")
                );
                assert_eq!(
                    program_id_field.get("Type"),
                    expected_program_id_field.get("Type")
                );
            }
            _ => panic!("Expected Route instruction"),
        }
    }

    #[test]
    fn test_jupiter_with_platform_fee() {
        // Construct Route directly to isolate formatting from parsing
        let instruction = JupiterSwapInstruction::Route {
            in_token: None,
            out_token: None,
            slippage_bps: 50,
            platform_fee_bps: 100,
        };

        let formatted = format_jupiter_swap_instruction(&instruction);
        assert!(formatted.contains("50bps"), "Should contain slippage");
        assert!(
            formatted.contains("platform fee: 100bps"),
            "Should contain platform fee when non-zero"
        );

        let fields = create_jupiter_swap_expanded_fields(
            &instruction,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &[0x01, 0x02, 0x03],
        )
        .unwrap();

        let has_platform_fee = fields.iter().any(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Platform Fee"
            } else {
                false
            }
        });
        assert!(
            has_platform_fee,
            "Should have Platform Fee field when platform_fee_bps > 0"
        );
    }

    #[test]
    fn test_jupiter_uncovered_instruction_fallthrough() {
        // Unknown discriminator should gracefully degrade to Unknown variant
        let garbage_data = [
            0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x6a, 0x7b, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        ];

        let accounts = vec!["JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string()];

        let result = parse_jupiter_swap_instruction(&garbage_data, &accounts).unwrap();
        assert!(
            matches!(
                result,
                JupiterSwapInstruction::Unknown {
                    instruction_name: None
                }
            ),
            "Unknown discriminator should gracefully degrade to Unknown variant"
        );

        // Test expanded fields for Unknown variant by constructing directly
        let instruction = JupiterSwapInstruction::Unknown {
            instruction_name: None,
        };

        let formatted = format_jupiter_swap_instruction(&instruction);
        assert_eq!(formatted, "Jupiter: Unknown Instruction");

        let fields = create_jupiter_swap_expanded_fields(
            &instruction,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &garbage_data,
        )
        .unwrap();

        assert!(fields.len() >= 3, "Should have at least 3 fields");

        let has_program_id = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, .. } = &f.signable_payload_field {
                common.label == "Program ID"
            } else {
                false
            }
        });
        assert!(has_program_id, "Should have Program ID field");

        let has_status = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, text_v2 } = &f.signable_payload_field {
                common.label == "Status" && text_v2.text == "Unknown Jupiter instruction type"
            } else {
                false
            }
        });
        assert!(has_status, "Should have Status field");

        let has_raw_data = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, .. } = &f.signable_payload_field {
                common.label == "Raw Data"
            } else {
                false
            }
        });
        assert!(has_raw_data, "Should have Raw Data field");
    }

    #[test]
    fn test_jupiter_instruction_name_from_idl() {
        // Test that when we have an instruction name from IDL but don't explicitly handle it,
        // we display the instruction name instead of just "Unknown"

        // Create a mock Unknown instruction with a name (as would come from IDL parsing)
        let instruction = JupiterSwapInstruction::Unknown {
            instruction_name: Some("setTokenLedger".to_string()),
        };

        // Test formatting includes the instruction name
        let formatted = format_jupiter_swap_instruction(&instruction);
        assert_eq!(
            formatted, "Jupiter: setTokenLedger",
            "Should show instruction name from IDL"
        );

        // Test expanded fields show the instruction name
        let fields = create_jupiter_swap_expanded_fields(
            &instruction,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &[0x01, 0x02, 0x03], // minimal data
        )
        .unwrap();

        // Check that status field includes the instruction name
        let status_field = fields.iter().find(|f| {
            if let SignablePayloadField::TextV2 { common, text_v2 } = &f.signable_payload_field {
                common.label == "Status"
                    && text_v2.text.contains("setTokenLedger")
                    && text_v2.text.contains("not explicitly handled")
            } else {
                false
            }
        });
        assert!(
            status_field.is_some(),
            "Status field should show instruction name from IDL"
        );

        // Test with None instruction name
        let instruction_no_name = JupiterSwapInstruction::Unknown {
            instruction_name: None,
        };
        let formatted_no_name = format_jupiter_swap_instruction(&instruction_no_name);
        assert_eq!(
            formatted_no_name, "Jupiter: Unknown Instruction",
            "Should fallback to generic unknown when no name available"
        );
    }

    #[test]
    fn test_jupiter_exact_out_route_parsing() {
        // ExactOutRoute: same body layout as Route, different discriminator
        // Amount fields are reversed: out_amount / quoted_in_amount instead of in_amount / quoted_out_amount
        let discriminator = hex::decode("d033ef977b2bed5c").expect("valid hex");
        let body = fixture_route_plan_body();
        let data: Vec<u8> = [discriminator, body].concat();
        let accounts = fixture_accounts();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::ExactOutRoute {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
            } => {
                assert_eq!(*slippage_bps, 50, "Slippage should be 50 bps");
                assert_eq!(*platform_fee_bps, 0, "Platform fee should be 0");
                // ExactOutRoute reverses amounts: in_token gets quoted_in_amount, out_token gets out_amount
                assert_eq!(
                    in_token.as_ref().unwrap().amount,
                    1550653,
                    "in_token should use quoted_in_amount"
                );
                assert_eq!(
                    out_token.as_ref().unwrap().amount,
                    2000000,
                    "out_token should use out_amount"
                );
            }
            _ => panic!("Expected ExactOutRoute instruction, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(
            formatted.contains("Jupiter Exact Out Route"),
            "Should contain 'Jupiter Exact Out Route', got: {formatted}"
        );
        assert!(formatted.contains("50bps"), "Should contain slippage");

        let fields = create_jupiter_swap_expanded_fields(
            &parsed,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &data,
        )
        .unwrap();

        let has_program_id = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, .. } = &f.signable_payload_field {
                common.label == "Program ID"
            } else {
                false
            }
        });
        assert!(has_program_id, "Should have Program ID field");

        let has_slippage = fields.iter().any(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Slippage"
            } else {
                false
            }
        });
        assert!(has_slippage, "Should have Slippage field");
    }

    #[test]
    fn test_jupiter_shared_accounts_route_parsing() {
        // SharedAccountsRoute: extra leading `id: u8` byte after discriminator, then same body as Route
        let discriminator = hex::decode("c1209b3341d69c81").expect("valid hex");
        let id_byte = vec![0x00u8]; // id = 0
        let body = fixture_route_plan_body();
        let data: Vec<u8> = [discriminator, id_byte, body].concat();
        let accounts = fixture_accounts();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::SharedAccountsRoute {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
            } => {
                assert_eq!(*slippage_bps, 50, "Slippage should be 50 bps");
                assert_eq!(*platform_fee_bps, 0, "Platform fee should be 0");
                // Same field order as Route: in_amount then quoted_out_amount
                assert_eq!(
                    in_token.as_ref().unwrap().amount,
                    2000000,
                    "in_token should use in_amount"
                );
                assert_eq!(
                    out_token.as_ref().unwrap().amount,
                    1550653,
                    "out_token should use quoted_out_amount"
                );
            }
            _ => panic!("Expected SharedAccountsRoute instruction, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(
            formatted.contains("Jupiter Shared Accounts Route"),
            "Should contain 'Jupiter Shared Accounts Route', got: {formatted}"
        );
        assert!(formatted.contains("50bps"), "Should contain slippage");

        let fields = create_jupiter_swap_expanded_fields(
            &parsed,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &data,
        )
        .unwrap();

        let has_program_id = fields.iter().any(|f| {
            if let SignablePayloadField::TextV2 { common, .. } = &f.signable_payload_field {
                common.label == "Program ID"
            } else {
                false
            }
        });
        assert!(has_program_id, "Should have Program ID field");

        let has_slippage = fields.iter().any(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Slippage"
            } else {
                false
            }
        });
        assert!(has_slippage, "Should have Slippage field");
    }

    /// Build a minimal v2 route body (after discriminator + any leading `id` byte).
    /// Layout: amount_a, amount_b, slippage_bps, platform_fee_bps, positive_slippage_bps,
    /// then an empty route_plan vec. Semantic meaning of amount_a/amount_b differs
    /// per variant (in/out vs. out/in) but byte layout is identical.
    fn build_route_v2_body(
        amount_a: u64,
        amount_b: u64,
        slippage_bps: u16,
        platform_fee_bps: u16,
        positive_slippage_bps: u16,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&amount_a.to_le_bytes());
        body.extend_from_slice(&amount_b.to_le_bytes());
        body.extend_from_slice(&slippage_bps.to_le_bytes());
        body.extend_from_slice(&platform_fee_bps.to_le_bytes());
        body.extend_from_slice(&positive_slippage_bps.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // empty route_plan
        body
    }

    /// 8 accounts — enough for shared v2 variants (source_mint at index 6, destination_mint at 7).
    fn fixture_accounts_shared_v2() -> Vec<String> {
        vec![
            "11111111111111111111111111111111".to_string(),
            "22222222222222222222222222222222".to_string(),
            "33333333333333333333333333333333".to_string(),
            "44444444444444444444444444444444".to_string(),
            "55555555555555555555555555555555".to_string(),
            "66666666666666666666666666666666".to_string(),
            "So11111111111111111111111111111111111111112".to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
        ]
    }

    #[test]
    fn test_jupiter_route_v2_parsing() {
        // route_v2 discriminator: [187, 100, 250, 204, 49, 196, 175, 20]
        let discriminator = hex::decode("bb64facc31c4af14").expect("valid hex");
        let body = build_route_v2_body(2_000_000, 1_550_653, 50, 0, 25);
        let data: Vec<u8> = [discriminator, body].concat();
        let accounts = fixture_accounts();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::RouteV2 {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
                positive_slippage_bps,
            } => {
                assert_eq!(*slippage_bps, 50);
                assert_eq!(*platform_fee_bps, 0);
                assert_eq!(*positive_slippage_bps, 25);
                // source_mint at accounts[3] -> in_amount
                assert_eq!(in_token.as_ref().unwrap().amount, 2_000_000);
                // destination_mint at accounts[4] -> quoted_out_amount
                assert_eq!(out_token.as_ref().unwrap().amount, 1_550_653);
            }
            _ => panic!("Expected RouteV2, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(formatted.contains("Jupiter Swap V2"));
        assert!(formatted.contains("positive slippage: 25bps"));

        let fields = create_jupiter_swap_expanded_fields(
            &parsed,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &data,
        )
        .unwrap();

        let has_positive_slippage = fields.iter().any(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Positive Slippage"
            } else {
                false
            }
        });
        assert!(
            has_positive_slippage,
            "Should have Positive Slippage field when positive_slippage_bps > 0"
        );
    }

    #[test]
    fn test_jupiter_exact_out_route_v2_parsing() {
        // exact_out_route_v2 discriminator: [157, 138, 184, 82, 21, 244, 243, 36]
        let discriminator = hex::decode("9d8ab85215f4f324").expect("valid hex");
        // For exact_out: first u64 is out_amount, second is quoted_in_amount.
        let body = build_route_v2_body(2_000_000, 1_550_653, 50, 0, 0);
        let data: Vec<u8> = [discriminator, body].concat();
        let accounts = fixture_accounts();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::ExactOutRouteV2 {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
                positive_slippage_bps,
            } => {
                assert_eq!(*slippage_bps, 50);
                assert_eq!(*platform_fee_bps, 0);
                assert_eq!(*positive_slippage_bps, 0);
                // in_token gets quoted_in_amount (second u64), out_token gets out_amount (first)
                assert_eq!(in_token.as_ref().unwrap().amount, 1_550_653);
                assert_eq!(out_token.as_ref().unwrap().amount, 2_000_000);
            }
            _ => panic!("Expected ExactOutRouteV2, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(formatted.contains("Jupiter Exact Out Route V2"));
        // positive_slippage == 0 should NOT appear in the formatted line
        assert!(!formatted.contains("positive slippage"));
    }

    #[test]
    fn test_jupiter_shared_accounts_route_v2_parsing() {
        // shared_accounts_route_v2 discriminator: [209, 152, 83, 147, 124, 254, 216, 233]
        let discriminator = hex::decode("d19853937cfed8e9").expect("valid hex");
        let id_byte = vec![0x00u8];
        let body = build_route_v2_body(2_000_000, 1_550_653, 50, 10, 5);
        let data: Vec<u8> = [discriminator, id_byte, body].concat();
        let accounts = fixture_accounts_shared_v2();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::SharedAccountsRouteV2 {
                in_token,
                out_token,
                slippage_bps,
                platform_fee_bps,
                positive_slippage_bps,
            } => {
                assert_eq!(*slippage_bps, 50);
                assert_eq!(*platform_fee_bps, 10);
                assert_eq!(*positive_slippage_bps, 5);
                // source_mint at accounts[6], destination_mint at accounts[7]
                assert_eq!(in_token.as_ref().unwrap().amount, 2_000_000);
                assert_eq!(out_token.as_ref().unwrap().amount, 1_550_653);
            }
            _ => panic!("Expected SharedAccountsRouteV2, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(formatted.contains("Jupiter Shared Accounts Route V2"));
        assert!(formatted.contains("platform fee: 10bps"));
        assert!(formatted.contains("positive slippage: 5bps"));
    }

    #[test]
    fn test_jupiter_shared_accounts_exact_out_route_v2_parsing() {
        // shared_accounts_exact_out_route_v2 discriminator: [53, 96, 229, 202, 216, 187, 250, 24]
        let discriminator = hex::decode("3560e5cad8bbfa18").expect("valid hex");
        let id_byte = vec![0x00u8];
        // For exact_out: first u64 is out_amount, second is quoted_in_amount.
        let body = build_route_v2_body(2_000_000, 1_550_653, 50, 0, 0);
        let data: Vec<u8> = [discriminator, id_byte, body].concat();
        let accounts = fixture_accounts_shared_v2();

        let parsed = parse_jupiter_swap_instruction(&data, &accounts).unwrap();

        match &parsed {
            JupiterSwapInstruction::SharedAccountsExactOutRouteV2 {
                in_token,
                out_token,
                ..
            } => {
                // in_token gets quoted_in_amount, out_token gets out_amount
                assert_eq!(in_token.as_ref().unwrap().amount, 1_550_653);
                assert_eq!(out_token.as_ref().unwrap().amount, 2_000_000);
            }
            _ => panic!("Expected SharedAccountsExactOutRouteV2, got {parsed:?}"),
        }

        let formatted = format_jupiter_swap_instruction(&parsed);
        assert!(formatted.contains("Jupiter Shared Accounts Exact Out Route V2"));
    }
}
