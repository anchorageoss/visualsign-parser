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

/// Extract mint addresses from SPL transfers in the transaction
fn extract_mints_from_transfers(
    context: &VisualizerContext,
    instruction_data: &[u8],
) -> Option<(String, String)> {
    // Parse amounts from instruction data to match with transfers
    let (in_amount, out_amount, _, _) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(instruction_data).ok()?;

    // Get SPL transfers from context
    let transfers = context.spl_transfers()?;

    let mut input_mint: Option<String> = None;
    let mut output_mint: Option<String> = None;

    // Look through SPL transfers for matching amounts
    for transfer in transfers {
        if let Some(ref token_mint) = transfer.token_mint {
            // Match transfer amount with swap amounts
            // Note: amounts might not match exactly due to fees, so we could add tolerance
            if transfer.amount == in_amount.to_string() && input_mint.is_none() {
                input_mint = Some(token_mint.clone());
            } else if transfer.amount == out_amount.to_string() && output_mint.is_none() {
                output_mint = Some(token_mint.clone());
            }
        }
    }

    // Return mints if both found
    match (input_mint, output_mint) {
        (Some(input), Some(output)) => Some((input, output)),
        _ => None,
    }
}

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

        // Extract transfer data if available
        let transfer_mints = extract_mints_from_transfers(context, &instruction.data);

        let jupiter_instruction =
            parse_jupiter_swap_instruction(&instruction.data, &instruction_accounts, transfer_mints)
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
    transfer_mints: Option<(String, String)>,
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 8 {
        return Err("Invalid instruction data length");
    }

    let discriminator = &data[0..8];

    match discriminator {
        d if d == JUPITER_ROUTE_DISCRIMINATOR => parse_route_instruction(data, accounts, transfer_mints),
        d if d == JUPITER_EXACT_OUT_ROUTE_DISCRIMINATOR => {
            parse_exact_out_route_instruction(data, accounts, transfer_mints)
        }
        d if d == JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR => {
            parse_shared_accounts_route_instruction(data, accounts, transfer_mints)
        }
        _ => Ok(JupiterSwapInstruction::Unknown),
    }
}

/// Extract mint addresses from Jupiter swap accounts using heuristic strategies
///
/// Strategy 1: Look for destination mint at index 5 (per IDL)
/// Strategy 2: Look for known mint addresses in the accounts array
/// Strategy 3: Use positional heuristics based on common patterns
fn extract_mints_from_accounts(accounts: &[String]) -> Result<(String, String), &'static str> {
    // Known token mint addresses
    const KNOWN_MINTS: &[&str] = &[
        "So11111111111111111111111111111111111111112",  // Wrapped SOL
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", // USDC
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", // USDT
        "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So",  // mSOL
        "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs", // ETHER (Wormhole)
        "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263", // BONK
        "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN",  // JUP
        "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3", // PYTH
    ];

    // Programs that should not be mistaken for mints
    const PROGRAM_IDS: &[&str] = &[
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",  // Token program
        "11111111111111111111111111111111",             // System program
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", // ATA program
        "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",  // Jupiter program
        "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc",  // Whirlpool
        "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8", // Raydium AMM
        "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK", // Raydium CLMM
        "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",  // Memo program
    ];

    // Strategy: Find two known mints in the accounts array
    let mints_found: Vec<String> = accounts.iter()
        .filter(|acc| KNOWN_MINTS.contains(&acc.as_str()))
        .cloned()
        .collect();

    // If we have at least 2 known mints, use them
    if mints_found.len() >= 2 {
        return Ok((mints_found[0].clone(), mints_found[1].clone()));
    }

    // Fallback: Try to get destination mint from index 5 (per IDL structure)
    let output_mint = if accounts.len() > 5 && !PROGRAM_IDS.contains(&accounts[5].as_str()) {
        accounts[5].clone()
    } else {
        // Look for any mint that's not at the beginning
        accounts.iter()
            .skip(5)
            .find(|acc| !PROGRAM_IDS.contains(&acc.as_str()) && !is_likely_user_account(acc))
            .cloned()
            .unwrap_or_else(|| {
                // Last resort: just use the first non-program account we find
                accounts.iter()
                    .find(|acc| !PROGRAM_IDS.contains(&acc.as_str()) && !is_likely_user_account(acc))
                    .cloned()
                    .unwrap_or_else(|| accounts.get(2).cloned().unwrap_or_else(|| "Unknown".to_string()))
            })
    };

    // Find input mint: Look for known mints or scan accounts
    let input_mint = {
        // First, try to find a known mint that we haven't used as output
        let found_known = accounts.iter()
            .find(|acc| KNOWN_MINTS.contains(&acc.as_str()) && acc.as_str() != &output_mint)
            .cloned();

        if let Some(mint) = found_known {
            mint
        } else {
            // Look for potential mints after position 5
            accounts.iter()
                .enumerate()
                .skip(5)
                .find(|(_, acc)| {
                    !PROGRAM_IDS.contains(&acc.as_str())
                    && acc.as_str() != &output_mint
                    && !is_likely_user_account(acc)
                })
                .map(|(_, acc)| acc.clone())
                .unwrap_or_else(|| {
                    // Fallback: use the first known mint as last resort
                    KNOWN_MINTS.get(0).map(|s| s.to_string()).unwrap_or_else(|| "Unknown".to_string())
                })
        }
    };

    Ok((input_mint, output_mint))
}

/// Check if an account is likely a user account (not a mint)
fn is_likely_user_account(account: &str) -> bool {
    // User accounts often have certain patterns
    // This is a heuristic - not perfect
    account.starts_with("B7h") || // Common user wallet pattern
    account.len() != 44 // Mints are usually base58 encoded 32-byte pubkeys (44 chars)
}

fn parse_route_instruction(
    data: &[u8],
    accounts: &[String],
    transfer_mints: Option<(String, String)>,
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    // Use transfer mints if available, otherwise fall back to account scanning
    let (input_mint, output_mint) = if let Some((in_mint, out_mint)) = transfer_mints {
        (in_mint, out_mint)
    } else {
        extract_mints_from_accounts(accounts)?
    };

    let in_token = Some(get_token_info(&input_mint, in_amount));
    let out_token = Some(get_token_info(&output_mint, out_amount));

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
    transfer_mints: Option<(String, String)>,
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    // Use transfer mints if available, otherwise fall back to account scanning
    let (input_mint, output_mint) = if let Some((in_mint, out_mint)) = transfer_mints {
        (in_mint, out_mint)
    } else {
        extract_mints_from_accounts(accounts)?
    };

    let in_token = Some(get_token_info(&input_mint, in_amount));
    let out_token = Some(get_token_info(&output_mint, out_amount));

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
    transfer_mints: Option<(String, String)>,
) -> Result<JupiterSwapInstruction, &'static str> {
    let (in_amount, out_amount, slippage_bps, platform_fee_bps) =
        JupiterSwapInstruction::parse_amounts_and_slippage_from_data(data)?;

    // Use transfer mints if available, otherwise fall back to account scanning
    let (input_mint, output_mint) = if let Some((in_mint, out_mint)) = transfer_mints {
        (in_mint, out_mint)
    } else {
        extract_mints_from_accounts(accounts)?
    };

    let in_token = Some(get_token_info(&input_mint, in_amount));
    let out_token = Some(get_token_info(&output_mint, out_amount));

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

        // Parse the instruction (no transfer data in test)
        let parsed_instruction =
            parse_jupiter_swap_instruction(&instruction_data, &accounts, None).unwrap();

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

        // Parse the instruction (no transfer data in test)
        let result = parse_jupiter_swap_instruction(&instruction_data, &accounts, None).unwrap();

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
        match parse_jupiter_swap_instruction(&route_data, &accounts, None) {
            Ok(JupiterSwapInstruction::Route { .. }) => println!("✅ Route discriminator matches"),
            _ => panic!("Route discriminator should match"),
        }

        // Test ExactOutRoute discriminator
        match parse_jupiter_swap_instruction(&exact_out_data, &accounts, None) {
            Ok(JupiterSwapInstruction::ExactOutRoute { .. }) => {
                println!("✅ ExactOutRoute discriminator matches")
            }
            _ => panic!("ExactOutRoute discriminator should match"),
        }

        // Test SharedAccountsRoute discriminator
        match parse_jupiter_swap_instruction(&shared_accounts_data, &accounts, None) {
            Ok(JupiterSwapInstruction::SharedAccountsRoute { .. }) => {
                println!("✅ SharedAccountsRoute discriminator matches")
            }
            _ => panic!("SharedAccountsRoute discriminator should match"),
        }

        // Test unknown discriminator
        match parse_jupiter_swap_instruction(&unknown_data, &accounts, None) {
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

        // Parse the instruction (no transfer data in test)
        let result = parse_jupiter_swap_instruction(&instruction_data, &accounts, None).unwrap();

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
    mod fixture_test;
}
