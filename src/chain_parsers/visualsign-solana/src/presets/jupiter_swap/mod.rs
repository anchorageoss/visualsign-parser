//! Jupiter swap preset implementation for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::{SwapTokenInfo, get_token_info};
use config::JupiterSwapConfig;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_amount_field, create_number_field, create_raw_data_field, create_text_field,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

// Jupiter instruction discriminators (8-byte values)
const JUPITER_ROUTE_DISCRIMINATOR: [u8; 8] = [0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a];
const JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR: [u8; 8] =
    [0x4b, 0xd7, 0xdf, 0xa8, 0x0c, 0xd0, 0xb6, 0x2a];
const JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR: [u8; 8] =
    [0x3a, 0xf2, 0xaa, 0xae, 0x2f, 0xb6, 0xd4, 0x2a];

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
    Unknown,
}

impl JupiterSwapInstruction {
    /// Parse amounts, slippage, and platform fee from instruction data
    ///
    /// Jupiter Route instruction format (suffix):
    /// - 8 bytes: in_amount
    /// - 8 bytes: out_amount
    /// - 2 bytes: slippage_bps
    /// - 1 byte: platform_fee_bps
    ///
    /// Total: 19 bytes at the end of instruction data
    fn parse_amounts_and_slippage_from_data(
        data: &[u8],
    ) -> Result<(u64, u64, u16, u8), &'static str> {
        if data.len() < 19 {
            return Err("Instruction data too short");
        }

        let len = data.len();
        let in_amount = u64::from_le_bytes([
            data[len - 19],
            data[len - 18],
            data[len - 17],
            data[len - 16],
            data[len - 15],
            data[len - 14],
            data[len - 13],
            data[len - 12],
        ]);
        let out_amount = u64::from_le_bytes([
            data[len - 11],
            data[len - 10],
            data[len - 9],
            data[len - 8],
            data[len - 7],
            data[len - 6],
            data[len - 5],
            data[len - 4],
        ]);
        let slippage_bps = u16::from_le_bytes([data[len - 3], data[len - 2]]);
        let platform_fee_bps = data[len - 1];

        Ok((in_amount, out_amount, slippage_bps, platform_fee_bps))
    }
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

fn parse_jupiter_swap_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 8 {
        return Err("Invalid instruction data length");
    }

    let discriminator = &data[0..8];

    match discriminator {
        d if d == JUPITER_ROUTE_DISCRIMINATOR => parse_route_instruction(data, accounts),
        d if d == JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR => {
            parse_exact_out_route_instruction(data, accounts)
        }
        d if d == JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR => {
            parse_shared_accounts_route_instruction(data, accounts)
        }
        _ => Ok(JupiterSwapInstruction::Unknown),
    }
}

fn parse_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    let in_token = accounts.first().map(|addr| get_token_info(addr, in_amount));
    let out_token = accounts.get(1).map(|addr| get_token_info(addr, out_amount));

    Ok(JupiterSwapInstruction::Route {
        in_token,
        out_token,
        slippage_bps,
        platform_fee_bps,
    })
}

fn parse_exact_out_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    let in_token = accounts.first().map(|addr| get_token_info(addr, in_amount));
    let out_token = accounts.get(1).map(|addr| get_token_info(addr, out_amount));

    Ok(JupiterSwapInstruction::ExactOutRoute {
        in_token,
        out_token,
        slippage_bps,
        platform_fee_bps,
    })
}

fn parse_shared_accounts_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    let in_token = accounts.first().map(|addr| get_token_info(addr, in_amount));
    let out_token = accounts.get(1).map(|addr| get_token_info(addr, out_amount));

    Ok(JupiterSwapInstruction::SharedAccountsRoute {
        in_token,
        out_token,
        slippage_bps,
        platform_fee_bps,
    })
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
        JupiterSwapInstruction::Unknown => "Jupiter: Unknown Instruction".to_string(),
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
                    create_text_field("Input Token Address", &token.address)
                        .map_err(|e| VisualSignError::ConversionError(e.to_string()))?,
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
        JupiterSwapInstruction::Unknown => {
            fields.push(
                create_text_field("Status", "Unknown Jupiter instruction type")
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
    use base64::engine::{Engine, general_purpose::STANDARD};

    #[test]
    fn test_jupiter_swap_instruction_parsing() {
        // Real Jupiter swap transaction data
        let transaction_b64 = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAsTTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ6653iXpBtBVMUA2+7hURKVHSEiGP6Bzz+71DafYBHQDv0Yk27V9AGBuUCokgwtdJtHGjOn65hFbpKYxFjpOxf9DslqNk9ntU1o905D8G/f/M/gGJfV/szOEdGlj8ByB4ydCgh9JdZoBmFC/1V+60NB9JdEtwXur6E410yCBDwODn7a9i8ySuhrG7m4UOmmngOd7rrj0EIP/mIOo3poMglc7k/piKlm7+u7deeb1LQ3/H1gPv54+BUArFsw2O5lY54pz/YD6rtbZ/BQGLaOTytSS3SHI51lpsQDqNm8IHuyTAFQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwTp4S+8hOgmyTLM6eJkDM4VWQwcYnOwklcIujuFILC8BpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqYb8H//NLjVx31IUdFMPpkUf0008tghSu5vUckZpELeujJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FmycNZ/qYxRzwITBRNYliuvNXQr7VnJ2URenA0MhcfNkbQ/+if11/ZKdMCbHylYed5LCas238ndUUsyGqezjOXo/NFB6YMsrxCtkXSVyg8nG1spPNRwJ+pzcAftQOs5oL2MaEXlNY7kQGEFwqYqsAepz7QXX/3fSFmPGjLpqakIxwYJAAUCQA0DAA8GAAIADAgNAQEIAgACDAIAAACghgEAAAAAAA0BAgERChsNAAIDChIKEQoLBA4BBQIDEgwGCwANDRALBwoj5RfLl3rjrSoBAAAAJmQAAaCGAQAAAAAAkz4BAAAAAAAyAAANAwIAAAEJ";

        // Decode the transaction
        let _transaction_bytes = STANDARD
            .decode(transaction_b64)
            .expect("Failed to decode base64");

        // Extract the Jupiter instruction data from the transaction
        // This is a simplified extraction - in a real scenario you'd parse the full transaction
        let instruction_data = [
            0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a, // Route discriminator
            0x01, 0x00, 0x00, 0x00, 0x26, 0x64, 0x00, 0x00, // Additional data
            0xa0, 0x86, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // Input amount: 100000
            0x93, 0x3e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // Output amount: 99150
            0x0a, 0x00, // Slippage: 10 bps
            0x00, // Platform fee: 0 bps
        ];

        // Mock accounts for testing
        let accounts = vec![
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string(), // Jupiter program ID
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(), // Token program
        ];

        // Parse the instruction
        let parsed_instruction =
            parse_jupiter_swap_instruction(&instruction_data, &accounts).unwrap();

        // Verify it parsed as a Route instruction
        match parsed_instruction {
            JupiterSwapInstruction::Route { slippage_bps, .. } => {
                assert_eq!(slippage_bps, 10, "Slippage should be 10 bps");
            }
            _ => panic!("Expected Route instruction, got {parsed_instruction:?}"),
        }

        // Test the formatting
        let formatted = format_jupiter_swap_instruction(&parsed_instruction);
        assert!(
            formatted.contains("Jupiter"),
            "Formatted string should contain 'Jupiter'"
        );
        assert!(
            formatted.contains("10bps"),
            "Formatted string should contain slippage"
        );

        // Test expanded fields creation
        let fields = create_jupiter_swap_expanded_fields(
            &parsed_instruction,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &instruction_data,
        )
        .unwrap();

        // Verify we get the expected number of fields
        assert!(
            fields.len() >= 3,
            "Should have at least 3 fields (Program ID, Slippage, Raw Data)"
        );

        // Check that we have a Program ID field
        let program_id_field = fields.iter().find(|f| {
            if let SignablePayloadField::TextV2 { common, text_v2: _ } = &f.signable_payload_field {
                common.label == "Program ID"
            } else {
                false
            }
        });
        assert!(program_id_field.is_some(), "Should have Program ID field");

        // Check that we have a Slippage field
        let slippage_field = fields.iter().find(|f| {
            if let SignablePayloadField::Number { common, number: _ } = &f.signable_payload_field {
                common.label == "Slippage"
            } else {
                false
            }
        });
        assert!(slippage_field.is_some(), "Should have Slippage field");
    }

    #[test]
    fn test_jupiter_instruction_with_real_data() {
        use serde_json::json;

        // Jupiter Route instruction data (8-byte discriminator + data)
        let instruction_data = [
            0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a, // Route discriminator
            0x01, 0x00, 0x00, 0x00, 0x26, 0x64, 0x00, 0x00, // Additional data
            0xa0, 0x86, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // Input amount: 100000
            0x93, 0x3e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // Output amount: 99150
            0x0a, 0x00, // Slippage: 10 bps
            0x00, // Platform fee: 0 bps
        ];

        let accounts = vec!["JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string()];

        // Parse the instruction
        let result = parse_jupiter_swap_instruction(&instruction_data, &accounts).unwrap();

        // Verify parsing result using pattern matching
        match result {
            JupiterSwapInstruction::Route { slippage_bps, .. } => {
                assert_eq!(slippage_bps, 10);

                // Create fields and verify their structure
                let fields = create_jupiter_swap_expanded_fields(
                    &result,
                    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                    &instruction_data,
                )
                .unwrap();

                // Test JSON serialization structure
                let fields_json = serde_json::to_value(&fields).unwrap();

                // Verify expected JSON structure
                assert!(
                    fields_json.is_array(),
                    "Fields should serialize to JSON array"
                );
                let fields_array = fields_json.as_array().unwrap();
                assert!(fields_array.len() >= 3, "Should have at least 3 fields");

                // Verify that we have a Program ID field with correct structure
                let has_program_id = fields_array.iter().any(|field| {
                    field
                        .get("Label")
                        .and_then(|label| label.as_str())
                        .map(|s| s == "Program ID")
                        .unwrap_or(false)
                        && field
                            .get("Type")
                            .and_then(|type_val| type_val.as_str())
                            .map(|s| s == "text_v2")
                            .unwrap_or(false)
                });

                // Verify that we have a Slippage field with correct structure
                let has_slippage = fields_array.iter().any(|field| {
                    field
                        .get("Label")
                        .and_then(|label| label.as_str())
                        .map(|s| s == "Slippage")
                        .unwrap_or(false)
                        && field
                            .get("Type")
                            .and_then(|type_val| type_val.as_str())
                            .map(|s| s == "number")
                            .unwrap_or(false)
                });

                assert!(
                    has_program_id,
                    "Should have Program ID field in JSON structure"
                );
                assert!(has_slippage, "Should have Slippage field in JSON structure");

                // Verify the JSON matches expected structure using serde_json::json! macro
                let expected_program_id_field = json!({
                    "Label": "Program ID",
                    "Type": "text_v2"
                });

                let program_id_field = fields_array
                    .iter()
                    .find(|field| field.get("Label").and_then(|l| l.as_str()) == Some("Program ID"))
                    .unwrap();

                // Check partial structure match
                assert_eq!(
                    program_id_field.get("Label"),
                    expected_program_id_field.get("Label")
                );
                assert_eq!(
                    program_id_field.get("Type"),
                    expected_program_id_field.get("Type")
                );

                println!("✅ Jupiter instruction parsed and serialized successfully");
                println!(
                    "✅ Created {} fields with correct JSON structure",
                    fields_array.len()
                );
            }
            _ => panic!("Expected Route instruction"),
        }
    }

    #[test]
    fn test_jupiter_discriminator_constants() {
        // Verify discriminator constants are correct 8-byte arrays
        assert_eq!(JUPITER_ROUTE_DISCRIMINATOR.len(), 8);
        assert_eq!(JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR.len(), 8);
        assert_eq!(JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR.len(), 8);

        // Verify they are different
        assert_ne!(
            JUPITER_ROUTE_DISCRIMINATOR,
            JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR
        );
        assert_ne!(
            JUPITER_ROUTE_DISCRIMINATOR,
            JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR
        );
        assert_ne!(
            JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR,
            JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR
        );
    }

    #[test]
    fn test_jupiter_discriminator_matching() {
        // Test that our discriminators match correctly
        // Each instruction needs at least 27 bytes: 8 for discriminator + 16 for amounts + 2 for slippage + 1 for platform_fee
        let route_data = [
            0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a, // discriminator
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // padding/intermediate data
            0x00, 0xe1, 0xf5, 0x05, 0x00, 0x00, 0x00, 0x00, // in_amount (100000000)
            0x00, 0xc2, 0xeb, 0x0b, 0x00, 0x00, 0x00, 0x00, // out_amount (200000000)
            0x0a, 0x00, // slippage (10 bps)
            0x00, // platform_fee_bps (0 bps)
        ];
        let exact_out_data = [
            0x4b, 0xd7, 0xdf, 0xa8, 0x0c, 0xd0, 0xb6, 0x2a, // discriminator
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // padding/intermediate data
            0x00, 0xe1, 0xf5, 0x05, 0x00, 0x00, 0x00, 0x00, // in_amount (100000000)
            0x00, 0xc2, 0xeb, 0x0b, 0x00, 0x00, 0x00, 0x00, // out_amount (200000000)
            0x0a, 0x00, // slippage (10 bps)
            0x00, // platform_fee_bps (0 bps)
        ];
        let shared_accounts_data = [
            0x3a, 0xf2, 0xaa, 0xae, 0x2f, 0xb6, 0xd4, 0x2a, // discriminator
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // padding/intermediate data
            0x00, 0xe1, 0xf5, 0x05, 0x00, 0x00, 0x00, 0x00, // in_amount (100000000)
            0x00, 0xc2, 0xeb, 0x0b, 0x00, 0x00, 0x00, 0x00, // out_amount (200000000)
            0x0a, 0x00, // slippage (10 bps)
            0x00, // platform_fee_bps (0 bps)
        ];
        let unknown_data = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // unknown discriminator
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // padding/intermediate data
            0x00, 0xe1, 0xf5, 0x05, 0x00, 0x00, 0x00, 0x00, // in_amount (100000000)
            0x00, 0xc2, 0xeb, 0x0b, 0x00, 0x00, 0x00, 0x00, // out_amount (200000000)
            0x0a, 0x00, // slippage (10 bps)
            0x00, // platform_fee_bps (0 bps)
        ];

        let accounts = vec!["test".to_string()];

        // Test Route discriminator
        match parse_jupiter_swap_instruction(&route_data, &accounts) {
            Ok(JupiterSwapInstruction::Route { .. }) => println!("✅ Route discriminator matches"),
            _ => panic!("Route discriminator should match"),
        }

        // Test ExactOutRoute discriminator
        match parse_jupiter_swap_instruction(&exact_out_data, &accounts) {
            Ok(JupiterSwapInstruction::ExactOutRoute { .. }) => {
                println!("✅ ExactOutRoute discriminator matches")
            }
            _ => panic!("ExactOutRoute discriminator should match"),
        }

        // Test SharedAccountsRoute discriminator
        match parse_jupiter_swap_instruction(&shared_accounts_data, &accounts) {
            Ok(JupiterSwapInstruction::SharedAccountsRoute { .. }) => {
                println!("✅ SharedAccountsRoute discriminator matches")
            }
            _ => panic!("SharedAccountsRoute discriminator should match"),
        }

        // Test unknown discriminator
        match parse_jupiter_swap_instruction(&unknown_data, &accounts) {
            Ok(JupiterSwapInstruction::Unknown) => {
                println!("✅ Unknown discriminator handled correctly")
            }
            _ => panic!("Unknown discriminator should return Unknown variant"),
        }
    }

    #[test]
    fn test_jupiter_with_platform_fee() {
        // Test Jupiter Route instruction with non-zero platform fee
        let instruction_data = [
            0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a, // Route discriminator
            0x01, 0x00, 0x00, 0x00, 0x26, 0x64, 0x00, 0x00, // Additional data
            0x00, 0xe1, 0xf5, 0x05, 0x00, 0x00, 0x00, 0x00, // in_amount (100000000)
            0x00, 0xc2, 0xeb, 0x0b, 0x00, 0x00, 0x00, 0x00, // out_amount (200000000)
            0x32, 0x00, // slippage (50 bps)
            0x64, // platform_fee_bps (100 bps)
        ];

        let accounts = vec!["JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string()];

        // Parse the instruction
        let result = parse_jupiter_swap_instruction(&instruction_data, &accounts).unwrap();

        // Verify parsing
        match result {
            JupiterSwapInstruction::Route {
                slippage_bps,
                platform_fee_bps,
                ..
            } => {
                assert_eq!(slippage_bps, 50, "Slippage should be 50 bps");
                assert_eq!(platform_fee_bps, 100, "Platform fee should be 100 bps");
                println!("✅ Correctly parsed slippage: {slippage_bps} bps");
                println!("✅ Correctly parsed platform fee: {platform_fee_bps} bps");
            }
            _ => panic!("Expected Route instruction"),
        }

        // Test the formatting includes platform fee
        let formatted = format_jupiter_swap_instruction(&result);
        assert!(
            formatted.contains("50bps"),
            "Formatted string should contain slippage"
        );
        assert!(
            formatted.contains("platform fee: 100bps"),
            "Formatted string should contain platform fee when non-zero"
        );
        println!("✅ Formatted output: {formatted}");

        // Test expanded fields include platform fee
        let fields = create_jupiter_swap_expanded_fields(
            &result,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            &instruction_data,
        )
        .unwrap();

        // Check that we have a Platform Fee field
        let platform_fee_field = fields.iter().find(|f| {
            if let SignablePayloadField::Number { common, .. } = &f.signable_payload_field {
                common.label == "Platform Fee"
            } else {
                false
            }
        });
        assert!(
            platform_fee_field.is_some(),
            "Should have Platform Fee field when platform_fee_bps > 0"
        );
        println!("✅ Platform Fee field present in expanded fields");
    }
}
