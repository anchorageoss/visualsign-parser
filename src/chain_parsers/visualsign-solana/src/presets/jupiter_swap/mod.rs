//! Jupiter swap preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::{SwapTokenInfo, get_token_info};
use config::JupiterSwapConfig;
use solana_parser::{Idl, ProgramType, decode_idl_data, parse_instruction_with_idl};
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

/// Get Jupiter v6 IDL from built-in IDLs in solana-parser
fn get_jupiter_idl() -> Option<Idl> {
    let program_id = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

    ProgramType::from_program_id(program_id).and_then(|pt| decode_idl_data(pt.idl_json()).ok())
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
    let program_id = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
    let idl = get_jupiter_idl().ok_or("Jupiter IDL not available")?;

    // Parse using solana_parser
    let parsed = parse_instruction_with_idl(data, program_id, &idl)?;

    // Extract instruction type and arguments
    match parsed.instruction_name.as_str() {
        "route" => {
            let in_amount = extract_u64_arg(&parsed.program_call_args, "in_amount")?;
            let quoted_out_amount =
                extract_u64_arg(&parsed.program_call_args, "quoted_out_amount")?;
            let slippage_bps = extract_u64_arg(&parsed.program_call_args, "slippage_bps")? as u16;
            let platform_fee_bps =
                extract_u64_arg(&parsed.program_call_args, "platform_fee_bps")? as u8;

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
            let slippage_bps = extract_u64_arg(&parsed.program_call_args, "slippage_bps")? as u16;
            let platform_fee_bps =
                extract_u64_arg(&parsed.program_call_args, "platform_fee_bps")? as u8;

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
            let slippage_bps = extract_u64_arg(&parsed.program_call_args, "slippage_bps")? as u16;
            let platform_fee_bps =
                extract_u64_arg(&parsed.program_call_args, "platform_fee_bps")? as u8;

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
        _ => Ok(JupiterSwapInstruction::Unknown {
            instruction_name: Some(parsed.instruction_name.clone()),
        }),
    }
}

fn parse_jupiter_swap_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 8 {
        return Err("Invalid instruction data length");
    }

    parse_jupiter_instruction_with_idl(data, accounts)
        .map_err(|_| "Failed to parse Jupiter instruction with IDL")
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
mod tests {
    use super::*;
    mod fixture_test;

    /// Real instruction data from sample_route.json fixture (WSOL -> USELESS swap)
    fn fixture_instruction_data() -> Vec<u8> {
        hex::decode("e517cb977ae3ad2a010000002f010064000180841e00000000003da9170000000000320000")
            .expect("valid hex")
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
        // Unknown discriminator should now return Err (not Ok(Unknown))
        let garbage_data = [
            0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x6a, 0x7b, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        ];

        let accounts = vec!["JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string()];

        let result = parse_jupiter_swap_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return Err");

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
}
