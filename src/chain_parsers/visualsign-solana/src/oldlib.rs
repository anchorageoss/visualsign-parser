use base64::engine::Engine;
use borsh::de::BorshDeserialize;
use solana_parser::solana::parser::parse_transaction;
use solana_program::system_instruction::SystemInstruction;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use spl_associated_token_account::instruction::AssociatedTokenAccountInstruction;
use spl_stake_pool::instruction::StakePoolInstruction;
use std::collections::HashMap;
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    field_builders::{
        create_amount_field, create_number_field, create_raw_data_field, create_text_field,
    },
    vsptrait::{VisualSignError, VisualSignOptions},
};


#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
}

/// Static lookup table for common Solana token addresses
pub fn get_token_lookup_table() -> HashMap<&'static str, TokenInfo> {
    // This is a simplified static lookup table for common tokens
    // In a real application, this could be replaced with a more dynamic solution
    // or fetched from a reliable source like a token registry.
    let mut tokens = HashMap::new();

    // SOL (native)
    tokens.insert(
        "11111111111111111111111111111111",
        TokenInfo {
            symbol: "SOL",
            name: "Solana",
            decimals: 9,
        },
    );

    // USDC
    tokens.insert(
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        TokenInfo {
            symbol: "USDC",
            name: "USD Coin",
            decimals: 6,
        },
    );

    // USDT
    tokens.insert(
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        TokenInfo {
            symbol: "USDT",
            name: "Tether USD",
            decimals: 6,
        },
    );

    tokens
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<SolanaTransaction, Box<dyn std::error::Error>> {
    let bytes = match encodings {
        SupportedEncodings::Base64 => {
            base64::engine::general_purpose::STANDARD.decode(raw_transaction)?
        }
        SupportedEncodings::Hex => hex::decode(raw_transaction)?,
    };

    let transaction: SolanaTransaction = bincode::deserialize(&bytes)?;
    Ok(transaction)
}

fn parse_system_instruction(data: &[u8]) -> Result<SystemInstruction, &'static str> {
    bincode::deserialize(data).map_err(|_| "Failed to unpack system instruction")
}

pub fn parse_stake_pool_instruction(
    instruction: &[u8],
) -> Result<StakePoolInstruction, &'static str> {
    StakePoolInstruction::try_from_slice(instruction)
        .map_err(|_| "Failed to decode stake pool instruction")
}

fn parse_ata_instruction(data: &[u8]) -> Result<AssociatedTokenAccountInstruction, &'static str> {
    if data.is_empty() {
        return Err("Empty data");
    }
    match data[0] {
        0 => Ok(AssociatedTokenAccountInstruction::Create),
        1 => Ok(AssociatedTokenAccountInstruction::CreateIdempotent),
        2 => Ok(AssociatedTokenAccountInstruction::RecoverNested),
        _ => Err("Unknown ATA instruction"),
    }
}

fn parse_compute_budget_instruction(data: &[u8]) -> Result<ComputeBudgetInstruction, &'static str> {
    ComputeBudgetInstruction::try_from_slice(data)
        .map_err(|_| "Failed to decode compute budget instruction")
}

/// Enhanced swap instruction with token information
#[derive(Debug, Clone)]
pub struct SwapTokenInfo {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub amount: u64,
    pub human_readable_amount: String,
}

/// Helper function to format token amounts
pub fn format_token_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10_u64.pow(decimals as u32);
    let whole = amount / divisor;
    let fractional = amount % divisor;

    if fractional == 0 {
        format!("{}", whole)
    } else {
        let fractional_str = format!("{:0width$}", fractional, width = decimals as usize);
        let trimmed = fractional_str.trim_end_matches('0');
        if trimmed.is_empty() {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, trimmed)
        }
    }
}

/// Helper function to get token info from address
pub fn get_token_info(address: &str, amount: u64) -> SwapTokenInfo {
    let token_lookup = get_token_lookup_table();

    if let Some(token_info) = token_lookup.get(address) {
        SwapTokenInfo {
            address: address.to_string(),
            symbol: token_info.symbol.to_string(),
            name: token_info.name.to_string(),
            decimals: token_info.decimals,
            amount,
            human_readable_amount: format_token_amount(amount, token_info.decimals),
        }
    } else {
        // Unknown token - show truncated address
        let truncated = if address.len() > 8 {
            format!("{}...{}", &address[0..4], &address[address.len() - 4..])
        } else {
            address.to_string()
        };

        SwapTokenInfo {
            address: address.to_string(),
            symbol: truncated.clone(),
            name: format!("Unknown Token ({})", truncated),
            decimals: 0, // Unknown decimals
            amount,
            human_readable_amount: amount.to_string(),
        }
    }
}

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
            parse_jupiter_route_instruction(&data[8..], accounts)
        }
        // Route instruction discriminator: 0xe517cb977ae3ad2a
        [0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5] => {
            parse_jupiter_route_instruction(&data[8..], accounts)
        }
        // ExactOutRoute instruction discriminator: 0x4bd7dfa80cd0b62a
        [0x2a, 0xb6, 0xd0, 0x0c, 0xa8, 0xdf, 0xd7, 0x4b] => {
            parse_jupiter_exact_out_route_instruction(&data[8..], accounts)
        }
        // SharedAccountsRoute instruction discriminator: 0x3af2aaae2fb6d42a
        [0x2a, 0xd4, 0xb6, 0x2f, 0xae, 0xaa, 0xf2, 0x3a] => {
            parse_jupiter_shared_accounts_route_instruction(&data[8..], accounts)
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

    // Skip the route plan (variable length), parse basic amounts at the end
    // This is a simplified parser - in reality we'd need to parse the full borsh structure
    let in_amount = u64::from_le_bytes(
        data[data.len() - 16..data.len() - 8]
            .try_into()
            .map_err(|_| "Invalid in_amount")?,
    );
    let quoted_out_amount = u64::from_le_bytes(
        data[data.len() - 8..]
            .try_into()
            .map_err(|_| "Invalid quoted_out_amount")?,
    );

    // For Jupiter swaps, we need to infer token accounts from the instruction accounts
    // Typically: accounts[0] = source token account, accounts[1] = destination token account
    // But Jupiter uses multiple accounts, so we'll use common positions or fall back to unknown
    let in_token = if accounts.len() > 2 {
        Some(get_token_info(&accounts[2], in_amount)) // Often the source mint
    } else {
        None
    };

    let out_token = if accounts.len() > 3 {
        Some(get_token_info(&accounts[3], quoted_out_amount)) // Often the destination mint
    } else {
        None
    };

    Ok(JupiterSwapInstruction::Route {
        in_token,
        out_token,
        slippage_bps: 50, // Default, we'd need to parse the full structure to get the real value
    })
}

fn parse_jupiter_exact_out_route_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterSwapInstruction, &'static str> {
    if data.len() < 16 {
        return Err("ExactOutRoute instruction data too short");
    }

    let in_amount = u64::from_le_bytes(
        data[data.len() - 16..data.len() - 8]
            .try_into()
            .map_err(|_| "Invalid in_amount")?,
    );
    let out_amount = u64::from_le_bytes(
        data[data.len() - 8..]
            .try_into()
            .map_err(|_| "Invalid out_amount")?,
    );

    let in_token = if accounts.len() > 2 {
        Some(get_token_info(&accounts[2], in_amount))
    } else {
        None
    };

    let out_token = if accounts.len() > 3 {
        Some(get_token_info(&accounts[3], out_amount))
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

    let in_amount = u64::from_le_bytes(
        data[data.len() - 16..data.len() - 8]
            .try_into()
            .map_err(|_| "Invalid in_amount")?,
    );
    let quoted_out_amount = u64::from_le_bytes(
        data[data.len() - 8..]
            .try_into()
            .map_err(|_| "Invalid quoted_out_amount")?,
    );

    let in_token = if accounts.len() > 2 {
        Some(get_token_info(&accounts[2], in_amount))
    } else {
        None
    };

    let out_token = if accounts.len() > 3 {
        Some(get_token_info(&accounts[3], quoted_out_amount))
    } else {
        None
    };

    Ok(JupiterSwapInstruction::SharedAccountsRoute {
        in_token,
        out_token,
        slippage_bps: 50, // Default
    })
}

fn format_compute_budget_instruction(
    instruction: &solana_sdk::compute_budget::ComputeBudgetInstruction,
) -> String {
    use solana_sdk::compute_budget::ComputeBudgetInstruction;

    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            format!("Request Heap Frame: {} bytes", bytes)
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            format!("Set Compute Unit Limit: {} units", units)
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            format!(
                "Set Compute Unit Price: {} micro-lamports per compute unit",
                micro_lamports
            )
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            format!("Set Loaded Accounts Data Size Limit: {} bytes", bytes)
        }
        ComputeBudgetInstruction::Unused => "Unused Compute Budget Instruction".to_string(),
    }
}

fn format_jupiter_swap_instruction(instruction: &JupiterSwapInstruction) -> String {
    match instruction {
        JupiterSwapInstruction::Route {
            in_token,
            out_token,
            slippage_bps,
        } => {
            let in_display = if let Some(token) = in_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            let out_display = if let Some(token) = out_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            format!(
                "Jupiter Swap: {} → {} (slippage: {}bps)",
                in_display, out_display, slippage_bps
            )
        }
        JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            let in_display = if let Some(token) = in_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            let out_display = if let Some(token) = out_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            format!(
                "Jupiter Exact Out Swap: {} → {} (slippage: {}bps)",
                in_display, out_display, slippage_bps
            )
        }
        JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            let in_display = if let Some(token) = in_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            let out_display = if let Some(token) = out_token {
                format!("{} {}", token.human_readable_amount, token.symbol)
            } else {
                "Unknown Token".to_string()
            };

            format!(
                "Jupiter Shared Route: {} → {} (slippage: {}bps)",
                in_display, out_display, slippage_bps
            )
        }
        JupiterSwapInstruction::Unknown => "Jupiter: Unknown Instruction".to_string(),
    }
}

fn convert_to_visual_sign_payload(
    transaction: SolanaTransaction,
    decode_transfers: bool,
    title: Option<String>,
) -> SignablePayload {
    let message = transaction.message.clone();
    let account_keys: Vec<String> = message
        .account_keys
        .iter()
        .map(|key| key.to_string())
        .collect();

    let mut fields = vec![
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: "Solana".to_string(),
                label: "Network".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: "Solana".to_string(),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: account_keys.join(", "),
                label: "Account Keys".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: account_keys.join(", "),
            },
        },
    ];

    if decode_transfers {
        let message_clone = transaction.message.clone();
        let parsed_transaction = parse_transaction(
            hex::encode(message_clone.serialize()),
            false, /* because we're passing the message only */
        )
        .unwrap();

        for (i, transfer) in parsed_transaction
            .clone()
            .solana_parsed_transaction
            .payload
            .unwrap()
            .transaction_metadata
            .unwrap()
            .transfers
            .iter()
            .enumerate()
        {
            fields.push(SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "Transfer {}: {} -> {}: {}",
                        i + 1,
                        transfer.from,
                        transfer.to,
                        transfer.amount
                    ),
                    label: format!("Transfer {}", i + 1),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!(
                        "From: {}\nTo: {}\nAmount: {}",
                        transfer.from, transfer.to, transfer.amount
                    ),
                },
            });
        }

        let payload = parsed_transaction
            .clone()
            .solana_parsed_transaction
            .payload
            .unwrap();
        for (i, spl_transfer) in payload
            .transaction_metadata
            .unwrap()
            .spl_transfers
            .iter()
            .enumerate()
        {
            fields.push(SignablePayloadField::TextV2  {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("SPL Transfer {}: {} -> {}: {}", i + 1, spl_transfer.from, spl_transfer.to, spl_transfer.amount),
                    label: format!("SPL Transfer {}", i + 1),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("From: {}\nTo: {}\nOwner: {}\nAmount: {}\nMint: {:?}\nDecimals: {:?}\nFee: {:?}", spl_transfer.from, spl_transfer.to, spl_transfer.owner, spl_transfer.amount, spl_transfer.token_mint, spl_transfer.decimals, spl_transfer.fee),
                },
            });
        }
    }

    // this might have double the transfers but I don't know yet how to filter them out if decode_transfers is true
    for (i, instruction) in message.instructions.iter().enumerate() {
        let program_id = message.account_keys[instruction.program_id_index as usize].to_string();
        let data = hex::encode(&instruction.data);

        let decoded_data = match program_id.as_str() {
            id if id == solana_sdk::system_program::id().to_string() => {
                match parse_system_instruction(&instruction.data) {
                    Ok(instruction_type) => format_system_instruction(&instruction_type),
                    Err(err) => {
                        println!("Failed to parse system instruction: {}", err);
                        "Unknown Instruction".to_string()
                    }
                }
            }
            "ComputeBudget111111111111111111111111111111" => {
                match parse_compute_budget_instruction(&instruction.data) {
                    Ok(instruction_type) => format_compute_budget_instruction(&instruction_type),
                    Err(err) => {
                        println!("Failed to parse compute budget instruction: {}", err);
                        "Unknown Instruction".to_string()
                    }
                }
            }
            program_id if program_id.starts_with("AToken") => {
                // Decode associated token address
                match parse_ata_instruction(&instruction.data) {
                    Ok(instruction_type) => format_ata_instruction(&instruction_type),
                    Err(err) => {
                        println!(
                            "Failed to parse associated token address instruction: {}",
                            err
                        );
                        "Unknown Instruction".to_string()
                    }
                }
            }
            program_id if program_id.starts_with("SPoo1") => {
                // Decode stake pool instruction
                match parse_stake_pool_instruction(&instruction.data) {
                    Ok(instruction_type) => format_stake_pool_instruction(&instruction_type),
                    Err(err) => {
                        println!("Failed to parse stake pool instruction: {}", err);
                        "Unknown Instruction".to_string()
                    }
                }
            }
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4" => {
                // Decode Jupiter swap instruction
                // Create account list from instruction accounts
                let instruction_accounts: Vec<String> = instruction
                    .accounts
                    .iter()
                    .map(|&account_index| message.account_keys[account_index as usize].to_string())
                    .collect();

                match parse_jupiter_swap_instruction(&instruction.data, &instruction_accounts) {
                    Ok(instruction_type) => format_jupiter_swap_instruction(&instruction_type),
                    Err(err) => {
                        println!("Failed to parse Jupiter swap instruction: {}", err);
                        "Unknown Instruction".to_string()
                    }
                }
            }

            _ => "Unknown Program ID".to_string(),
        };
        let instruction_user_display = match decoded_data.as_str() {
            "Unknown Instruction" => {
                format!("Instruction {}: Unknown ProgramID or Instruction", i + 1)
            }
            _ => decoded_data.clone(),
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![AnnotatedPayloadField {
                static_annotation: None,
                dynamic_annotation: None,
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: instruction_user_display.clone(),
                        label: "Instruction".to_string(),
                    },
                    text_v2: visualsign::SignablePayloadFieldTextV2 {
                        text: decoded_data.clone(),
                    },
                },
            }],
        };

        let expanded = match program_id.as_str() {
            "ComputeBudget111111111111111111111111111111" => {
                if let Ok(instruction_type) = parse_compute_budget_instruction(&instruction.data) {
                    SignablePayloadFieldListLayout {
                        fields: create_compute_budget_expanded_fields(
                            &instruction_type,
                            &program_id,
                            &instruction.data,
                        ),
                    }
                } else {
                    create_default_expanded_fields(&program_id, &instruction.data)
                }
            }
            "11111111111111111111111111111111" => {
                if let Ok(instruction_type) = parse_system_instruction(&instruction.data) {
                    SignablePayloadFieldListLayout {
                        fields: create_system_instruction_expanded_fields(
                            &instruction_type,
                            &program_id,
                            &instruction.data,
                        ),
                    }
                } else {
                    create_default_expanded_fields(&program_id, &instruction.data)
                }
            }
            program_id if program_id.starts_with("AToken") => {
                if let Ok(instruction_type) = parse_ata_instruction(&instruction.data) {
                    format_associated_token_instruction(&instruction_type, program_id)
                } else {
                    create_default_expanded_fields(program_id, &instruction.data)
                }
            }
            program_id if program_id.starts_with("SPoo1") => {
                if let Ok(instruction_type) = parse_stake_pool_instruction(&instruction.data) {
                    SignablePayloadFieldListLayout {
                        fields: vec![
                            create_text_field(
                                "Stake Pool Instruction",
                                &format_stake_pool_instruction(&instruction_type),
                            )
                            .unwrap(),
                        ],
                    }
                } else {
                    create_default_expanded_fields(program_id, &instruction.data)
                }
            }
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4" => {
                // Create account list from instruction accounts
                let instruction_accounts: Vec<String> = instruction
                    .accounts
                    .iter()
                    .map(|&account_index| message.account_keys[account_index as usize].to_string())
                    .collect();

                if let Ok(instruction_type) =
                    parse_jupiter_swap_instruction(&instruction.data, &instruction_accounts)
                {
                    SignablePayloadFieldListLayout {
                        fields: create_jupiter_swap_expanded_fields(
                            &instruction_type,
                            &program_id,
                            &instruction.data,
                        ),
                    }
                } else {
                    create_default_expanded_fields(&program_id, &instruction.data)
                }
            }
            _ => create_default_expanded_fields(&program_id, &instruction.data),
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: instruction_user_display.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: "".to_string(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        let fallback_instruction_str = format!(
            "Program ID: {program_id}
Data: {data}"
        );

        fields.push(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: fallback_instruction_str.clone(),
                label: format!("Instruction {}", i + 1),
            },
            preview_layout,
        });
    }

    let title = title.unwrap_or_else(|| "Solana Transaction".to_string());
    SignablePayload::new(
        0,
        title,
        None,
        fields,
        "SolanaTx".to_string(), // TODO derive this from the transaction
    )
}

// Public API functions for ease of use
pub fn transaction_to_visual_sign(
    transaction: SolanaTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = SolanaTransactionWrapper::new(transaction);
    let converter = SolanaVisualSignConverter;
    converter.to_visual_sign_payload(wrapper, options)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = SolanaVisualSignConverter;
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

fn format_system_instruction(instruction: &SystemInstruction) -> String {
    match instruction {
        SystemInstruction::CreateAccount {
            lamports,
            space,
            owner,
        } => {
            format!(
                "Create Account: {} lamports, {} bytes, owner: {}",
                lamports, space, owner
            )
        }
        SystemInstruction::Assign { owner } => {
            format!("Assign Account Owner: {}", owner)
        }
        SystemInstruction::Transfer { lamports } => {
            format!("Transfer: {} lamports", lamports)
        }
        SystemInstruction::CreateAccountWithSeed {
            base,
            seed,
            lamports,
            space,
            owner,
        } => {
            format!(
                "Create Account with Seed: base: {}, seed: '{}', {} lamports, {} bytes, owner: {}",
                base, seed, lamports, space, owner
            )
        }
        SystemInstruction::AdvanceNonceAccount => "Advance Nonce Account".to_string(),
        SystemInstruction::WithdrawNonceAccount(lamports) => {
            format!("Withdraw from Nonce Account: {} lamports", lamports)
        }
        SystemInstruction::InitializeNonceAccount(authorized) => {
            format!("Initialize Nonce Account: authorized: {}", authorized)
        }
        SystemInstruction::AuthorizeNonceAccount(authorized) => {
            format!("Authorize Nonce Account: new authorized: {}", authorized)
        }
        SystemInstruction::Allocate { space } => {
            format!("Allocate Account Space: {} bytes", space)
        }
        SystemInstruction::AllocateWithSeed {
            base,
            seed,
            space,
            owner,
        } => {
            format!(
                "Allocate with Seed: base: {}, seed: '{}', {} bytes, owner: {}",
                base, seed, space, owner
            )
        }
        SystemInstruction::AssignWithSeed { base, seed, owner } => {
            format!(
                "Assign with Seed: base: {}, seed: '{}', owner: {}",
                base, seed, owner
            )
        }
        SystemInstruction::TransferWithSeed {
            lamports,
            from_seed,
            from_owner,
        } => {
            format!(
                "Transfer with Seed: {} lamports, seed: '{}', from owner: {}",
                lamports, from_seed, from_owner
            )
        }
        SystemInstruction::UpgradeNonceAccount => "Upgrade Nonce Account".to_string(),
    }
}

fn format_ata_instruction(instruction: &AssociatedTokenAccountInstruction) -> String {
    match instruction {
        AssociatedTokenAccountInstruction::Create => "Create Associated Token Account".to_string(),
        AssociatedTokenAccountInstruction::CreateIdempotent => {
            "Create Associated Token Account (Idempotent)".to_string()
        }
        AssociatedTokenAccountInstruction::RecoverNested => {
            "Recover Nested Associated Token Account".to_string()
        }
    }
}

fn format_associated_token_instruction(
    instruction: &AssociatedTokenAccountInstruction,
    program_id: &str,
) -> SignablePayloadFieldListLayout {
    SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", program_id).unwrap(),
            create_text_field("Instruction", &format_ata_instruction(instruction)).unwrap(),
        ],
    }
}

fn format_stake_pool_instruction(instruction: &StakePoolInstruction) -> String {
    format!(
        "Stake Pool Instruction: {}",
        get_stake_pool_instruction_name(instruction)
    )
}

fn get_stake_pool_instruction_name(instruction: &StakePoolInstruction) -> &'static str {
    match instruction {
        StakePoolInstruction::Initialize { .. } => "Initialize",
        StakePoolInstruction::AddValidatorToPool(_) => "Add Validator to Pool",
        StakePoolInstruction::RemoveValidatorFromPool => "Remove Validator from Pool",
        StakePoolInstruction::DecreaseValidatorStake { .. } => "Decrease Validator Stake",
        StakePoolInstruction::IncreaseValidatorStake { .. } => "Increase Validator Stake",
        StakePoolInstruction::SetPreferredValidator { .. } => "Set Preferred Validator",
        StakePoolInstruction::UpdateValidatorListBalance { .. } => "Update Validator List Balance",
        StakePoolInstruction::UpdateStakePoolBalance => "Update Stake Pool Balance",
        StakePoolInstruction::CleanupRemovedValidatorEntries => "Cleanup Removed Validator Entries",
        StakePoolInstruction::DepositStake => "Deposit Stake",
        StakePoolInstruction::WithdrawStake(_) => "Withdraw Stake",
        StakePoolInstruction::SetManager => "Set Manager",
        StakePoolInstruction::SetFee { .. } => "Set Fee",
        StakePoolInstruction::SetStaker => "Set Staker",
        StakePoolInstruction::DepositSol(_) => "Deposit SOL",
        StakePoolInstruction::SetFundingAuthority(_) => "Set Funding Authority",
        StakePoolInstruction::WithdrawSol(_) => "Withdraw SOL",
        StakePoolInstruction::IncreaseAdditionalValidatorStake { .. } => {
            "Increase Additional Validator Stake"
        }
        StakePoolInstruction::DecreaseAdditionalValidatorStake { .. } => {
            "Decrease Additional Validator Stake"
        }
        StakePoolInstruction::DecreaseValidatorStakeWithReserve { .. } => {
            "Decrease Validator Stake with Reserve"
        }
        StakePoolInstruction::CreateTokenMetadata { .. } => "Create Token Metadata",
        StakePoolInstruction::UpdateTokenMetadata { .. } => "Update Token Metadata",
        StakePoolInstruction::DepositStakeWithSlippage { .. } => "Deposit Stake with Slippage",
        StakePoolInstruction::WithdrawStakeWithSlippage { .. } => "Withdraw Stake with Slippage",
        StakePoolInstruction::DepositSolWithSlippage { .. } => "Deposit SOL with Slippage",
        StakePoolInstruction::WithdrawSolWithSlippage { .. } => "Withdraw SOL with Slippage",
        #[allow(deprecated)]
        StakePoolInstruction::Redelegate { .. } => "Redelegate",
    }
}

fn create_sol_amount_field(label: &str, lamports: u64) -> AnnotatedPayloadField {
    let sol_value = lamports as f64 / 1_000_000_000.0;
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} SOL", sol_value),
                label: label.to_string(),
            },
            amount_v2: visualsign::SignablePayloadFieldAmountV2 {
                amount: lamports.to_string(),
                abbreviation: Some("lamports".to_string()),
            },
        },
    }
}
fn create_compute_budget_expanded_fields(
    instruction: &ComputeBudgetInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id).unwrap()];

    // Add specific fields based on instruction type
    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            fields.push(
                create_number_field("Heap Frame Size", &bytes.to_string(), "bytes")
                    .expect("needs to be numeric"),
            );
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            fields.push(
                create_number_field("Compute Unit Limit", &units.to_string(), "units")
                    .expect("compute units need to be numeric"),
            );
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            fields.push(
                create_number_field(
                    "Price per Compute Unit",
                    &micro_lamports.to_string(),
                    "micro-lamports",
                )
                .expect("price needs to be numeric"),
            );
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            fields.push(
                create_number_field("Data Size Limit", &bytes.to_string(), "bytes")
                    .expect("data size limit needs to be numeric"),
            );
        }
        ComputeBudgetInstruction::Unused => {
            // No additional fields for unused instruction
        }
    }

    let hex_fallback_string = hex::encode(data).to_string();
    let raw_data_field = create_raw_data_field(data, Some(hex_fallback_string)).unwrap();

    fields.push(raw_data_field);
    fields
}

fn create_system_instruction_expanded_fields(
    instruction: &SystemInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id).unwrap()];

    // Add specific fields based on instruction type
    match instruction {
        SystemInstruction::CreateAccount {
            lamports,
            space,
            owner,
        } => {
            fields.push(create_sol_amount_field("Amount", *lamports));
            fields.push(
                create_number_field("Space", &space.to_string(), "bytes")
                    .expect("space needs to be numeric"),
            );
            fields.push(create_text_field("Owner", &owner.to_string()).unwrap());
        }
        SystemInstruction::Transfer { lamports } => {
            fields.push(create_sol_amount_field("Transfer Amount", *lamports));
        }
        SystemInstruction::Assign { owner } => {
            fields.push(create_text_field("New Owner", &owner.to_string()).unwrap());
        }
        SystemInstruction::Allocate { space } => {
            fields.push(
                create_number_field("Space", &space.to_string(), "bytes")
                    .expect("space needs to be numeric"),
            );
        }
        _ => {
            // For other system instructions, just show the instruction type
            fields.push(
                create_text_field("Instruction Details", &format!("{:?}", instruction)).unwrap(),
            );
        } // TODO: add expansion for rest of the SystemInstruction enums
    }

    let hex_fallback_string = hex::encode(data).to_string();
    let raw_data_field = create_raw_data_field(data, Some(hex_fallback_string)).unwrap();
    fields.push(raw_data_field);
    fields
}

fn create_default_expanded_fields(program_id: &str, data: &[u8]) -> SignablePayloadFieldListLayout {
    SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", program_id).unwrap(),
            create_text_field("Raw Data", &hex::encode(data)).unwrap(),
        ],
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
        } => {
            if let Some(token) = in_token {
                fields.push(create_text_field("Input Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Input Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("amount needs to be numeric"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Input Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Input Token Address", &token.address).unwrap());
            }

            if let Some(token) = out_token {
                fields.push(create_text_field("Output Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Quoted Output Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("Quoted Output Amount needs to be numeric or token symbol missing"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Output Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Output Token Address", &token.address).unwrap());
            }

            fields.push(
                create_number_field("Slippage", &slippage_bps.to_string(), "bps")
                    .expect("slippage expected to be numeric"),
            );
        }
        JupiterSwapInstruction::ExactOutRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            if let Some(token) = in_token {
                fields.push(create_text_field("Input Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Max Input Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("Invalid Max Input Amount"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Input Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Input Token Address", &token.address).unwrap());
            }

            if let Some(token) = out_token {
                fields.push(create_text_field("Output Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Exact Output Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("Invalid Exact Output Amount"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Output Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Output Token Address", &token.address).unwrap());
            }

            fields.push(
                create_number_field("Slippage", &slippage_bps.to_string(), "bps")
                    .expect("Slippage needs to be numeric"),
            );
        }
        JupiterSwapInstruction::SharedAccountsRoute {
            in_token,
            out_token,
            slippage_bps,
        } => {
            if let Some(token) = in_token {
                fields.push(create_text_field("Input Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Input Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("Invalid Input Amount"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Input Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Input Token Address", &token.address).unwrap());
            }

            if let Some(token) = out_token {
                fields.push(create_text_field("Output Token", &token.symbol).unwrap());
                fields.push(
                    create_amount_field(
                        "Quoted Output Amount",
                        &token.human_readable_amount,
                        &token.symbol,
                    )
                    .expect("Invalid Quoted Output Amount"),
                );
                if !token.name.is_empty() && token.name != token.symbol {
                    fields.push(create_text_field("Output Token Name", &token.name).unwrap());
                }
                fields.push(create_text_field("Output Token Address", &token.address).unwrap());
            }

            fields.push(
                create_number_field("Slippage", &slippage_bps.to_string(), "bps")
                    .expect("Slippage needs to be numeric"),
            );
        }
        JupiterSwapInstruction::Unknown => {
            fields.push(
                create_text_field("Instruction Type", "Unknown Jupiter Instruction").unwrap(),
            );
        }
    }
    let hex_fallback_string = hex::encode(data).to_string();
    let raw_data_field = create_raw_data_field(data, Some(hex_fallback_string)).unwrap();
    fields.push(raw_data_field);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::compute_budget::ComputeBudgetInstruction;

    #[test]
    fn test_create_compute_budget_expanded_fields_set_compute_unit_limit() {
        let instruction = ComputeBudgetInstruction::SetComputeUnitLimit(1400000);
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x02, 0x00, 0x60, 0x5C, 0x15, 0x00]; // Sample encoded data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        assert_eq!(fields.len(), 3); // Program ID + Compute Unit Limit + Raw Data

        // Check Program ID field
        match &fields[0].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Program ID");
                assert_eq!(text_v2.text, program_id);
            }
            _ => panic!("Expected TextV2 field for Program ID"),
        }

        // Check Compute Unit Limit field
        match &fields[1].signable_payload_field {
            SignablePayloadField::Number { common, number } => {
                assert_eq!(common.label, "Compute Unit Limit");
                assert_eq!(common.fallback_text, "1400000 units");
                assert_eq!(number.number, "1400000");
            }
            _ => panic!("Expected Number field for Compute Unit Limit"),
        }

        // Check Raw Data field
        match &fields[2].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Raw Data");
                assert_eq!(text_v2.text, hex::encode(&data));
            }
            _ => panic!("Expected TextV2 field for Raw Data"),
        }
    }

    #[test]
    fn test_create_compute_budget_expanded_fields_set_compute_unit_price() {
        let instruction = ComputeBudgetInstruction::SetComputeUnitPrice(50000);
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x03, 0x50, 0xC3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // Sample encoded data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        assert_eq!(fields.len(), 3); // Program ID + Price + Raw Data

        // Check Price per Compute Unit field
        match &fields[1].signable_payload_field {
            SignablePayloadField::Number { common, number } => {
                assert_eq!(common.label, "Price per Compute Unit");
                assert_eq!(common.fallback_text, "50000 micro-lamports");
                assert_eq!(number.number, "50000");
            }
            _ => panic!("Expected Number field for Price per Compute Unit"),
        }

        // Check Raw Data field
        match &fields[2].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Raw Data");
                assert_eq!(text_v2.text, hex::encode(&data));
            }
            _ => panic!("Expected TextV2 field for Raw Data"),
        }
    }

    #[test]
    fn test_create_compute_budget_expanded_fields_request_heap_frame() {
        let instruction = ComputeBudgetInstruction::RequestHeapFrame(262144);
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x01, 0x00, 0x00, 0x04, 0x00]; // Sample encoded data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        assert_eq!(fields.len(), 3); // Program ID + Heap Frame Size + Raw Data

        // Check Heap Frame Size field
        match &fields[1].signable_payload_field {
            SignablePayloadField::Number { common, number } => {
                assert_eq!(common.label, "Heap Frame Size");
                assert_eq!(common.fallback_text, "262144 bytes");
                assert_eq!(number.number, "262144");
            }
            _ => panic!("Expected Number field for Heap Frame Size"),
        }
    }

    #[test]
    fn test_create_compute_budget_expanded_fields_set_loaded_accounts_data_size_limit() {
        let instruction = ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(65536);
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x04, 0x00, 0x00, 0x01, 0x00]; // Sample encoded data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        assert_eq!(fields.len(), 3); // Program ID + Data Size Limit + Raw Data

        // Check Data Size Limit field
        match &fields[1].signable_payload_field {
            SignablePayloadField::Number { common, number } => {
                assert_eq!(common.label, "Data Size Limit");
                assert_eq!(common.fallback_text, "65536 bytes");
                assert_eq!(number.number, "65536");
            }
            _ => panic!("Expected Number field for Data Size Limit"),
        }
    }

    #[test]
    fn test_create_compute_budget_expanded_fields_unused() {
        let instruction = ComputeBudgetInstruction::Unused;
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x00]; // Sample encoded data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        assert_eq!(fields.len(), 2); // Program ID + Raw Data (no specific field for Unused)

        // Should only have Program ID and Raw Data fields
        match &fields[0].signable_payload_field {
            SignablePayloadField::TextV2 { common, .. } => {
                assert_eq!(common.label, "Program ID");
            }
            _ => panic!("Expected TextV2 field for Program ID"),
        }

        match &fields[1].signable_payload_field {
            SignablePayloadField::TextV2 { common, .. } => {
                assert_eq!(common.label, "Raw Data");
            }
            _ => panic!("Expected TextV2 field for Raw Data"),
        }
    }

    #[test]
    fn test_format_compute_budget_instruction_string_output() {
        // Test that the string formatting doesn't include number formatting
        let compute_budget_instruction =
            solana_sdk::compute_budget::ComputeBudgetInstruction::SetComputeUnitLimit(1400000);
        let result = format_compute_budget_instruction(&compute_budget_instruction);

        assert_eq!(result, "Set Compute Unit Limit: 1400000 units");
        // Ensure no comma formatting in the string output
        assert!(!result.contains("1,400,000"));
    }

    #[test]
    fn test_parse_compute_budget_instruction_with_library_data() {
        // Test parsing instruction data created by the library itself
        use solana_sdk::compute_budget::ComputeBudgetInstruction;

        // Create various compute budget instructions using the library
        let instructions = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(1400000),
            ComputeBudgetInstruction::set_compute_unit_price(50000),
            ComputeBudgetInstruction::request_heap_frame(262144),
            ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(65536),
        ];

        for instruction in instructions {
            // Parse the instruction data that was created by the library
            let parsed_result = parse_compute_budget_instruction(&instruction.data);
            assert!(
                parsed_result.is_ok(),
                "Should successfully parse library-created instruction data"
            );

            // Verify the parsed instruction matches expectations
            let parsed_instruction = parsed_result.unwrap();
            match (&instruction, &parsed_instruction) {
                (_, ComputeBudgetInstruction::SetComputeUnitLimit(_))
                | (_, ComputeBudgetInstruction::SetComputeUnitPrice(_))
                | (_, ComputeBudgetInstruction::RequestHeapFrame(_))
                | (_, ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(_)) => {
                    // These are the expected variants, test passes
                }
                _ => panic!("Unexpected instruction variant parsed"),
            }
        }
    }

    #[test]
    fn test_compute_budget_expanded_fields_field_types() {
        let instruction = ComputeBudgetInstruction::SetComputeUnitLimit(1000000);
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x02, 0x40, 0x42, 0x0F, 0x00]; // Sample data

        let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

        // Verify that numeric values use Number field type, not TextV2
        match &fields[1].signable_payload_field {
            SignablePayloadField::Number { .. } => {
                // This is correct - numeric values should use Number field
            }
            SignablePayloadField::TextV2 { .. } => {
                panic!("Numeric values should use Number field, not TextV2");
            }
            _ => panic!("Unexpected field type for numeric value"),
        }
    }

    #[test]
    fn test_all_compute_budget_instruction_variants() {
        let program_id = "ComputeBudget111111111111111111111111111111";
        let data = vec![0x01, 0x02, 0x03];

        let instructions = vec![
            ComputeBudgetInstruction::RequestHeapFrame(32768),
            ComputeBudgetInstruction::SetComputeUnitLimit(200000),
            ComputeBudgetInstruction::SetComputeUnitPrice(1000),
            ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(10240),
            ComputeBudgetInstruction::Unused,
        ];

        for instruction in instructions {
            let fields = create_compute_budget_expanded_fields(&instruction, program_id, &data);

            // All should have at least Program ID and Raw Data
            assert!(fields.len() >= 2);

            // First field should always be Program ID
            match &fields[0].signable_payload_field {
                SignablePayloadField::TextV2 { common, .. } => {
                    assert_eq!(common.label, "Program ID");
                }
                _ => panic!("First field should be Program ID"),
            }

            // Last field should always be Raw Data
            let last_field = fields.last().unwrap();
            match &last_field.signable_payload_field {
                SignablePayloadField::TextV2 { common, .. } => {
                    assert_eq!(common.label, "Raw Data");
                }
                _ => panic!("Last field should be Raw Data"),
            }
        }
    }

    #[test]
    fn test_compute_budget_instruction_integration() {
        // Create a minimal transaction with a compute budget instruction
        use solana_sdk::{message::Message, pubkey::Pubkey};
        use std::str::FromStr;

        let _compute_budget_program_id =
            Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap();

        // Create SetComputeUnitLimit instruction using the library
        let instruction =
            solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(1400000);

        let payer = Pubkey::new_unique();
        let message = Message::new(&[instruction], Some(&payer));
        let transaction = SolanaTransaction::new_unsigned(message);

        // Convert to visual sign payload
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Test Compute Budget Transaction".to_string()),
        };

        let payload = convert_to_visual_sign_payload(transaction, false, options.transaction_name);

        // Find the compute budget instruction in the payload
        let compute_budget_instruction = payload
            .fields
            .iter()
            .find(|field| {
                if let SignablePayloadField::PreviewLayout { preview_layout, .. } = field {
                    if let Some(title) = &preview_layout.title {
                        title.text.contains("Set Compute Unit Limit")
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .expect("Should find compute budget instruction");

        // Check that it has the expanded view with proper field types
        if let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            compute_budget_instruction
        {
            if let Some(expanded) = &preview_layout.expanded {
                // Should have at least 3 fields: Program ID, Compute Unit Limit, Raw Data
                assert!(expanded.fields.len() >= 3);

                // Check that the compute unit limit field is a Number type
                let compute_unit_field = expanded
                    .fields
                    .iter()
                    .find(|field| match &field.signable_payload_field {
                        SignablePayloadField::Number { common, .. } => {
                            common.label == "Compute Unit Limit"
                        }
                        _ => false,
                    })
                    .expect("Should find Compute Unit Limit number field");

                if let SignablePayloadField::Number { number, .. } =
                    &compute_unit_field.signable_payload_field
                {
                    assert_eq!(number.number, "1400000");
                }
            }
        }
    }

    #[test]
    fn test_system_instruction_expanded_fields_transfer() {
        let instruction = SystemInstruction::Transfer {
            lamports: 1000000000,
        }; // 1 SOL
        let program_id = "11111111111111111111111111111111";
        let data = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0xCA, 0x9A, 0x3B, 0x00]; // Sample encoded data

        let fields = create_system_instruction_expanded_fields(&instruction, program_id, &data);

        // Should have Program ID + Transfer Amount + Raw Data
        assert_eq!(fields.len(), 3);

        // Check Transfer Amount field uses AmountV2 type
        match &fields[1].signable_payload_field {
            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                assert_eq!(common.label, "Transfer Amount");
                assert_eq!(amount_v2.amount, "1000000000");
                assert_eq!(amount_v2.abbreviation, Some("lamports".to_string()));
                // Fallback text should show SOL conversion
                assert!(common.fallback_text.contains("1 SOL"));
            }
            _ => panic!("Expected AmountV2 field for Transfer Amount"),
        }
    }

    #[test]
    fn test_system_instruction_integration() {
        // Create a minimal transaction with a system transfer instruction
        use solana_sdk::{message::Message, pubkey::Pubkey};
        use solana_system_interface::instruction as system_instruction;

        let from_pubkey = Pubkey::new_unique();
        let to_pubkey = Pubkey::new_unique();
        let lamports = 1000000000; // 1 SOL

        // Create system transfer instruction using the library
        let instruction = system_instruction::transfer(&from_pubkey, &to_pubkey, lamports);

        let message = Message::new(&[instruction], Some(&from_pubkey));
        let transaction = SolanaTransaction::new_unsigned(message);

        // Convert to visual sign payload
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Test System Transfer Transaction".to_string()),
        };

        let payload = convert_to_visual_sign_payload(transaction, false, options.transaction_name);

        // Find the system instruction in the payload
        let system_instruction = payload
            .fields
            .iter()
            .find(|field| {
                if let SignablePayloadField::PreviewLayout { preview_layout, .. } = field {
                    if let Some(title) = &preview_layout.title {
                        title.text.contains("Transfer: 1000000000 lamports")
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .expect("Should find system transfer instruction");

        // Check that it has the expanded view with proper field types
        if let SignablePayloadField::PreviewLayout { preview_layout, .. } = system_instruction {
            if let Some(expanded) = &preview_layout.expanded {
                // Should have at least 3 fields: Program ID, Transfer Amount, Raw Data
                assert!(expanded.fields.len() >= 3);

                // Check that the transfer amount field is an AmountV2 type
                let transfer_amount_field = expanded
                    .fields
                    .iter()
                    .find(|field| match &field.signable_payload_field {
                        SignablePayloadField::AmountV2 { common, .. } => {
                            common.label == "Transfer Amount"
                        }
                        _ => false,
                    })
                    .expect("Should find Transfer Amount field");

                if let SignablePayloadField::AmountV2 { amount_v2, .. } =
                    &transfer_amount_field.signable_payload_field
                {
                    assert_eq!(amount_v2.amount, "1000000000");
                    assert_eq!(amount_v2.abbreviation, Some("lamports".to_string()));
                }
            }
        }
    }

    #[test]
    fn test_format_ata_instruction() {
        let create = AssociatedTokenAccountInstruction::Create;
        let create_idempotent = AssociatedTokenAccountInstruction::CreateIdempotent;
        let recover_nested = AssociatedTokenAccountInstruction::RecoverNested;

        assert_eq!(
            format_ata_instruction(&create),
            "Create Associated Token Account"
        );
        assert_eq!(
            format_ata_instruction(&create_idempotent),
            "Create Associated Token Account (Idempotent)"
        );
        assert_eq!(
            format_ata_instruction(&recover_nested),
            "Recover Nested Associated Token Account"
        );
    }

    #[test]
    fn test_parse_ata_instruction() {
        // Test parsing ATA instruction data
        let create_data = vec![0]; // Create instruction
        let create_idempotent_data = vec![1]; // CreateIdempotent instruction
        let recover_nested_data = vec![2]; // RecoverNested instruction

        let parsed_create = parse_ata_instruction(&create_data);
        assert!(parsed_create.is_ok());
        match parsed_create.unwrap() {
            AssociatedTokenAccountInstruction::Create => {
                // Correct
            }
            _ => panic!("Expected Create instruction"),
        }

        let parsed_idempotent = parse_ata_instruction(&create_idempotent_data);
        assert!(parsed_idempotent.is_ok());
        match parsed_idempotent.unwrap() {
            AssociatedTokenAccountInstruction::CreateIdempotent => {
                // Correct
            }
            _ => panic!("Expected CreateIdempotent instruction"),
        }

        let parsed_recover = parse_ata_instruction(&recover_nested_data);
        assert!(parsed_recover.is_ok());
        match parsed_recover.unwrap() {
            AssociatedTokenAccountInstruction::RecoverNested => {
                // Correct
            }
            _ => panic!("Expected RecoverNested instruction"),
        }
    }

    #[test]
    fn test_format_associated_token_instruction_expanded_fields() {
        let instruction = AssociatedTokenAccountInstruction::Create;
        let expanded = format_associated_token_instruction(
            &instruction,
            "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
        );

        assert_eq!(expanded.fields.len(), 2); // Program ID + Instruction

        // Check Program ID field
        match &expanded.fields[0].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Program ID");
                assert_eq!(text_v2.text, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
            }
            _ => panic!("Expected TextV2 field for Program ID"),
        }

        // Check Instruction field
        match &expanded.fields[1].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Instruction");
                assert_eq!(text_v2.text, "Create Associated Token Account");
            }
            _ => panic!("Expected TextV2 field for Instruction"),
        }
    }

    #[test]
    fn test_get_stake_pool_instruction_name() {
        // Test a few key stake pool instruction types
        use spl_stake_pool::instruction::StakePoolInstruction;

        let initialize = StakePoolInstruction::Initialize {
            fee: spl_stake_pool::state::Fee {
                numerator: 1,
                denominator: 100,
            },
            withdrawal_fee: spl_stake_pool::state::Fee {
                numerator: 1,
                denominator: 100,
            },
            deposit_fee: spl_stake_pool::state::Fee {
                numerator: 1,
                denominator: 100,
            },
            referral_fee: 0,
            max_validators: 100,
        };

        let deposit_sol = StakePoolInstruction::DepositSol(1000000000); // 1 SOL
        let withdraw_sol = StakePoolInstruction::WithdrawSol(500000000); // 0.5 SOL

        assert_eq!(get_stake_pool_instruction_name(&initialize), "Initialize");
        assert_eq!(get_stake_pool_instruction_name(&deposit_sol), "Deposit SOL");
        assert_eq!(
            get_stake_pool_instruction_name(&withdraw_sol),
            "Withdraw SOL"
        );
    }

    #[test]
    fn test_format_stake_pool_instruction() {
        use spl_stake_pool::instruction::StakePoolInstruction;

        let deposit_sol = StakePoolInstruction::DepositSol(1000000000); // 1 SOL
        let formatted = format_stake_pool_instruction(&deposit_sol);

        assert_eq!(formatted, "Stake Pool Instruction: Deposit SOL");
    }

    #[test]
    fn test_ata_instruction_integration() {
        // Create a minimal transaction with an ATA instruction
        use solana_sdk::{message::Message, pubkey::Pubkey};
        use spl_associated_token_account::get_associated_token_address;
        use std::str::FromStr;

        let _ata_program_id =
            Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();

        // Create proper accounts for ATA instruction
        let payer = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let _associated_token_address = get_associated_token_address(&wallet, &mint);

        // Create ATA Create instruction using the library
        let instruction =
            spl_associated_token_account::instruction::create_associated_token_account(
                &payer,
                &wallet,
                &mint,
                &spl_token::id(),
            );

        let message = Message::new(&[instruction], Some(&payer));
        let transaction = SolanaTransaction::new_unsigned(message);

        // Convert to visual sign payload
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Test ATA Transaction".to_string()),
        };

        let payload = convert_to_visual_sign_payload(transaction, false, options.transaction_name);

        // Find the ATA instruction in the payload
        let ata_instruction = payload
            .fields
            .iter()
            .find(|field| {
                if let SignablePayloadField::PreviewLayout { preview_layout, .. } = field {
                    if let Some(title) = &preview_layout.title {
                        title.text.contains("Create Associated Token Account")
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .expect("Should find ATA instruction");

        // Check that it has the expanded view
        if let SignablePayloadField::PreviewLayout { preview_layout, .. } = ata_instruction {
            if let Some(expanded) = &preview_layout.expanded {
                // Should have 2 fields: Program ID + Instruction
                assert_eq!(expanded.fields.len(), 2);

                // Check Program ID field
                match &expanded.fields[0].signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 } => {
                        assert_eq!(common.label, "Program ID");
                        assert_eq!(text_v2.text, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
                    }
                    _ => panic!("Expected TextV2 field for Program ID"),
                }

                // Check Instruction field
                match &expanded.fields[1].signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 } => {
                        assert_eq!(common.label, "Instruction");
                        assert_eq!(text_v2.text, "Create Associated Token Account");
                    }
                    _ => panic!("Expected TextV2 field for Instruction"),
                }
            }
        }
    }

    #[test]
    fn test_stake_pool_instruction_integration() {
        // Create a minimal transaction with a stake pool instruction
        use solana_sdk::{instruction::Instruction, message::Message, pubkey::Pubkey};
        use std::str::FromStr;

        // Use a stake pool program ID that starts with "SPoo1"
        let stake_pool_program_id =
            Pubkey::from_str("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy").unwrap();

        // Create sample instruction data for a DepositSol instruction
        // This is a simplified approach since StakePoolInstruction doesn't have try_to_vec directly
        let instruction_data = vec![0x0f, 0x00, 0xCA, 0x9A, 0x3B, 0x00, 0x00, 0x00, 0x00]; // Sample DepositSol data

        // Create minimal accounts for the instruction
        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // stake_pool
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // validator_list
            solana_sdk::instruction::AccountMeta::new_readonly(Pubkey::new_unique(), false), // deposit_authority
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), true), // user_sol_transfer
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // user_pool_token
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // pool_mint
            solana_sdk::instruction::AccountMeta::new_readonly(
                solana_sdk::system_program::id(),
                false,
            ), // system_program
            solana_sdk::instruction::AccountMeta::new_readonly(spl_token::id(), false), // token_program
        ];

        let instruction = Instruction {
            program_id: stake_pool_program_id,
            accounts,
            data: instruction_data,
        };

        let payer = Pubkey::new_unique();
        let message = Message::new(&[instruction], Some(&payer));
        let transaction = SolanaTransaction::new_unsigned(message);

        // Convert to visual sign payload
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Test Stake Pool Transaction".to_string()),
        };

        let payload = convert_to_visual_sign_payload(transaction, false, options.transaction_name);

        // Find the stake pool instruction in the payload - it should appear even if parsing fails
        let stake_pool_instruction = payload
            .fields
            .iter()
            .find(|field| {
                if let SignablePayloadField::PreviewLayout { common, .. } = field {
                    // Check if the fallback text contains the stake pool program ID prefix
                    common.fallback_text.contains("SPoo1")
                } else {
                    false
                }
            })
            .expect("Should find stake pool instruction");

        // Check that it has the expanded view
        if let SignablePayloadField::PreviewLayout { preview_layout, .. } = stake_pool_instruction {
            if let Some(expanded) = &preview_layout.expanded {
                // Should have at least 2 fields: Program ID + Raw Data (since parsing might fail)
                assert!(expanded.fields.len() >= 2);

                // Check that we have Program ID field
                let has_program_id_field =
                    expanded
                        .fields
                        .iter()
                        .any(|field| match &field.signable_payload_field {
                            SignablePayloadField::TextV2 { common, .. } => {
                                common.label == "Program ID"
                            }
                            _ => false,
                        });

                assert!(has_program_id_field, "Should have Program ID field");
            }
        }
    }

    #[test]
    fn test_parse_jupiter_swap_instruction() {
        // Test Route instruction with discriminator [0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5]
        let route_data = vec![
            0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5, // discriminator
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // padding/route plan
            0x40, 0x42, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, // in_amount: 1000000
            0x80, 0x84, 0x1e, 0x00, 0x00, 0x00, 0x00, 0x00, // quoted_out_amount: 2000000
        ];

        // Mock accounts for testing
        let accounts = vec![
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string(), // Jupiter program
            "11111111111111111111111111111111".to_string(),            // SOL
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(), // USDC mint
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(), // USDT mint
        ];

        let result = parse_jupiter_swap_instruction(&route_data, &accounts);
        assert!(result.is_ok());

        match result.unwrap() {
            JupiterSwapInstruction::Route {
                in_token,
                out_token,
                ..
            } => {
                assert!(in_token.is_some());
                assert!(out_token.is_some());

                let in_token = in_token.unwrap();
                let out_token = out_token.unwrap();

                assert_eq!(in_token.amount, 1000000);
                assert_eq!(out_token.amount, 2000000);
                assert_eq!(in_token.symbol, "USDC");
                assert_eq!(out_token.symbol, "USDT");
            }
            _ => panic!("Expected Route instruction"),
        }

        // Test with invalid data
        let invalid_data = vec![0x00, 0x01, 0x02];
        let invalid_result = parse_jupiter_swap_instruction(&invalid_data, &accounts);
        assert!(invalid_result.is_err());

        // Test with unknown discriminator
        let unknown_data = vec![
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // unknown discriminator
            0x40, 0x42, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x84, 0x1e, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x40, 0x42, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x84, 0x1e, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        let unknown_result = parse_jupiter_swap_instruction(&unknown_data, &accounts);
        assert!(unknown_result.is_ok());

        match unknown_result.unwrap() {
            JupiterSwapInstruction::Unknown => {
                // This is expected
            }
            _ => panic!("Expected Unknown instruction"),
        }
    }

    #[test]
    fn test_format_jupiter_swap_instruction() {
        let usdc_token = SwapTokenInfo {
            address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
            amount: 1000000,
            human_readable_amount: "1".to_string(),
        };

        let usdt_token = SwapTokenInfo {
            address: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
            symbol: "USDT".to_string(),
            name: "Tether USD".to_string(),
            decimals: 6,
            amount: 2000000,
            human_readable_amount: "2".to_string(),
        };

        let route = JupiterSwapInstruction::Route {
            in_token: Some(usdc_token.clone()),
            out_token: Some(usdt_token.clone()),
            slippage_bps: 50,
        };

        let formatted = format_jupiter_swap_instruction(&route);
        assert_eq!(formatted, "Jupiter Swap: 1 USDC → 2 USDT (slippage: 50bps)");

        let exact_out = JupiterSwapInstruction::ExactOutRoute {
            in_token: Some(usdc_token),
            out_token: Some(usdt_token),
            slippage_bps: 100,
        };

        let formatted_exact = format_jupiter_swap_instruction(&exact_out);
        assert_eq!(
            formatted_exact,
            "Jupiter Exact Out Swap: 1 USDC → 2 USDT (slippage: 100bps)"
        );

        let unknown = JupiterSwapInstruction::Unknown;
        let formatted_unknown = format_jupiter_swap_instruction(&unknown);
        assert_eq!(formatted_unknown, "Jupiter: Unknown Instruction");
    }

    #[test]
    fn test_jupiter_swap_expanded_fields() {
        let usdc_token = SwapTokenInfo {
            address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            decimals: 6,
            amount: 1000000,
            human_readable_amount: "1".to_string(),
        };

        let usdt_token = SwapTokenInfo {
            address: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
            symbol: "USDT".to_string(),
            name: "Tether USD".to_string(),
            decimals: 6,
            amount: 2000000,
            human_readable_amount: "2".to_string(),
        };

        let route = JupiterSwapInstruction::Route {
            in_token: Some(usdc_token),
            out_token: Some(usdt_token),
            slippage_bps: 50,
        };

        let program_id = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
        let data = vec![0x01, 0x02, 0x03];

        let fields = create_jupiter_swap_expanded_fields(&route, program_id, &data);

        // Should have: Program ID + Input Token + Input Amount + Input Token Name + Input Token Address +
        //             Output Token + Output Amount + Output Token Name + Output Token Address + Slippage + Raw Data
        assert!(fields.len() >= 5); // At least Program ID + tokens info + slippage + raw data

        // Check Program ID field (first)
        match &fields[0].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Program ID");
                assert_eq!(text_v2.text, program_id);
            }
            _ => panic!("Expected TextV2 field for Program ID"),
        }

        // Check that we have input token information
        let has_input_token = fields.iter().any(|field| {
            if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
            {
                common.label == "Input Token" && text_v2.text == "USDC"
            } else {
                false
            }
        });
        assert!(has_input_token, "Should have input token field");

        // Check that we have output token information
        let has_output_token = fields.iter().any(|field| {
            if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
            {
                common.label == "Output Token" && text_v2.text == "USDT"
            } else {
                false
            }
        });
        assert!(has_output_token, "Should have output token field");

        // Check slippage field
        let has_slippage = fields.iter().any(|field| {
            if let SignablePayloadField::Number { common, number } = &field.signable_payload_field {
                common.label == "Slippage" && number.number == "50"
            } else {
                false
            }
        });
        assert!(has_slippage, "Should have slippage field");
    }

    #[test]
    fn test_jupiter_swap_integration() {
        // Create a minimal transaction with a Jupiter swap instruction
        use solana_sdk::{message::Message, pubkey::Pubkey};
        use std::str::FromStr;

        let jupiter_program_id =
            Pubkey::from_str("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4").unwrap();

        // Create a mock Jupiter Route instruction
        let instruction_data = vec![
            0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5, // Route discriminator
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // route plan (simplified)
            0x40, 0x42, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, // in_amount: 1000000
            0x80, 0x84, 0x1e, 0x00, 0x00, 0x00, 0x00, 0x00, // quoted_out_amount: 2000000
        ];

        let accounts = vec![
            solana_sdk::instruction::AccountMeta::new_readonly(Pubkey::new_unique(), false), // token_program
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), true), // user_transfer_authority
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // user_source_token_account
            solana_sdk::instruction::AccountMeta::new(Pubkey::new_unique(), false), // user_destination_token_account
        ];

        let instruction = solana_sdk::instruction::Instruction {
            program_id: jupiter_program_id,
            accounts,
            data: instruction_data,
        };

        let payer = Pubkey::new_unique();
        let message = Message::new(&[instruction], Some(&payer));
        let transaction = solana_sdk::transaction::Transaction::new_unsigned(message);

        // Convert to visual sign payload
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Test Jupiter Swap Transaction".to_string()),
        };

        let payload = convert_to_visual_sign_payload(transaction, false, options.transaction_name);

        // Find the Jupiter instruction in the payload
        let jupiter_instruction = payload
            .fields
            .iter()
            .find(|field| {
                if let SignablePayloadField::PreviewLayout { preview_layout, .. } = field {
                    if let Some(title) = &preview_layout.title {
                        println!("Found instruction with title: {}", title.text);
                        title.text.contains("Jupiter")
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .expect("Should find Jupiter swap instruction");

        // Check that it has the expanded view with proper field types
        if let SignablePayloadField::PreviewLayout { preview_layout, .. } = jupiter_instruction {
            if let Some(expanded) = &preview_layout.expanded {
                // Should have at least: Program ID + token info fields + slippage + raw data
                assert!(expanded.fields.len() >= 5);

                // Check that we have Program ID field
                let has_program_id =
                    expanded
                        .fields
                        .iter()
                        .any(|field| match &field.signable_payload_field {
                            SignablePayloadField::TextV2 { common, text_v2 } => {
                                common.label == "Program ID" && text_v2.text.contains("JUP6")
                            }
                            _ => false,
                        });
                assert!(has_program_id, "Should have Program ID field");

                // Check that we have input token information
                let has_input_amount =
                    expanded
                        .fields
                        .iter()
                        .any(|field| match &field.signable_payload_field {
                            SignablePayloadField::AmountV2 { common, .. } => {
                                common.label == "Input Amount"
                            }
                            _ => false,
                        });
                assert!(has_input_amount, "Should have Input Amount field");

                // Check that we have slippage field
                let has_slippage =
                    expanded
                        .fields
                        .iter()
                        .any(|field| match &field.signable_payload_field {
                            SignablePayloadField::Number { common, .. } => {
                                common.label == "Slippage"
                            }
                            _ => false,
                        });
                assert!(has_slippage, "Should have Slippage field");
            }
        }
    }

    #[test]
    fn test_analyze_real_jupiter_data() {
        // Real Jupiter swap data that's not being recognized
        let hex_data =
            "c1209b3341d69c810002000000386400013d00640102a086010000000000164a000000000000320000";
        let data = hex::decode(hex_data).expect("Invalid hex data");

        println!("Data length: {}", data.len());
        println!("First 8 bytes (discriminator): {:?}", &data[0..8]);
        println!("First 8 bytes as hex: {:02x?}", &data[0..8]);

        // Mock accounts for testing
        let accounts = vec![
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string(),
            "11111111111111111111111111111111".to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
        ];

        // Test if our current parser recognizes it
        match parse_jupiter_swap_instruction(&data, &accounts) {
            Ok(instruction) => {
                println!("Parsed as: {:?}", instruction);
                // Should now be parsed as a Route instruction (not Unknown)
                match instruction {
                    JupiterSwapInstruction::Route { .. } => {
                        println!("✓ Successfully recognized as Jupiter Route instruction");
                    }
                    JupiterSwapInstruction::Unknown => {
                        panic!("Real Jupiter data should not be Unknown anymore!");
                    }
                    _ => {
                        println!("Parsed as different instruction type: {:?}", instruction);
                    }
                }
            }
            Err(e) => println!("Parse error: {}", e),
        }

        // Test against our known discriminators
        let discriminator = &data[0..8];
        match discriminator {
            [0xc1, 0x20, 0x9b, 0x33, 0x41, 0xd6, 0x9c, 0x81] => {
                println!("✓ Matches real-world Jupiter Route")
            }
            [0x2a, 0xad, 0xe3, 0x7a, 0x97, 0xcb, 0x17, 0xe5] => println!("Matches Route"),
            [0x2a, 0xb6, 0xd0, 0x0c, 0xa8, 0xdf, 0xd7, 0x4b] => println!("Matches ExactOutRoute"),
            [0x2a, 0xd4, 0xb6, 0x2f, 0xae, 0xaa, 0xf2, 0x3a] => {
                println!("Matches SharedAccountsRoute")
            }
            _ => println!("Unknown discriminator: {:02x?}", discriminator),
        }
    }

    #[test]
    fn test_end_to_end_real_jupiter_parsing() {
        // Test that a real Jupiter swap instruction is correctly parsed through the main parser
        let hex_data =
            "c1209b3341d69c810002000000386400013d00640102a086010000000000164a000000000000320000";
        let data = hex::decode(hex_data).expect("Invalid hex data");

        // Mock account addresses for testing
        let accounts = vec![
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4".to_string(), // Jupiter program
            "11111111111111111111111111111111".to_string(),            // SOL
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(), // USDC mint
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(), // USDT mint
            "SysvarRent111111111111111111111111111111111".to_string(), // Sysvar Rent
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(), // Token Program
        ];

        match parse_jupiter_swap_instruction(&data, &accounts) {
            Ok(parsed_instruction) => {
                let formatted = format_jupiter_swap_instruction(&parsed_instruction);
                println!("Parsed instruction: {}", formatted);

                // Should contain "Jupiter" but not "Unknown"
                assert!(
                    formatted.contains("Jupiter"),
                    "Should contain 'Jupiter': {}",
                    formatted
                );
                assert!(
                    !formatted.contains("Unknown"),
                    "Should not contain 'Unknown': {}",
                    formatted
                );

                // Should have token information
                match parsed_instruction {
                    JupiterSwapInstruction::Route {
                        in_token,
                        out_token,
                        ..
                    }
                    | JupiterSwapInstruction::ExactOutRoute {
                        in_token,
                        out_token,
                        ..
                    }
                    | JupiterSwapInstruction::SharedAccountsRoute {
                        in_token,
                        out_token,
                        ..
                    } => {
                        // At least one token should be recognized
                        assert!(
                            in_token.is_some() || out_token.is_some(),
                            "Should have at least one recognized token"
                        );
                    }
                    JupiterSwapInstruction::Unknown => {
                        panic!("Should not parse as Unknown instruction");
                    }
                }
            }
            Err(e) => {
                panic!("Failed to parse Jupiter instruction: {}", e);
            }
        }
    }

    #[test]
    fn test_format_token_amount() {
        // Test with USDC (6 decimals)
        assert_eq!(format_token_amount(1000000, 6), "1");
        assert_eq!(format_token_amount(1500000, 6), "1.5");
        assert_eq!(format_token_amount(1050000, 6), "1.05");
        assert_eq!(format_token_amount(1000001, 6), "1.000001");

        // Test with SOL (9 decimals)
        assert_eq!(format_token_amount(1000000000, 9), "1");
        assert_eq!(format_token_amount(1500000000, 9), "1.5");
        assert_eq!(format_token_amount(1050000000, 9), "1.05");

        // Test with BONK (5 decimals)
        assert_eq!(format_token_amount(100000, 5), "1");
        assert_eq!(format_token_amount(150000, 5), "1.5");

        // Test with 0 decimals
        assert_eq!(format_token_amount(1000, 0), "1000");

        // Test with small amounts
        assert_eq!(format_token_amount(1, 6), "0.000001");
        assert_eq!(format_token_amount(100, 6), "0.0001");
    }

    #[test]
    fn test_get_token_info() {
        // Test with known USDC
        let usdc_info = get_token_info("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", 1000000);
        assert_eq!(usdc_info.symbol, "USDC");
        assert_eq!(usdc_info.name, "USD Coin");
        assert_eq!(usdc_info.decimals, 6);
        assert_eq!(usdc_info.amount, 1000000);
        assert_eq!(usdc_info.human_readable_amount, "1");

        // Test with unknown token
        let unknown_info = get_token_info("UnknownTokenAddress123456789", 500000);
        assert_eq!(unknown_info.symbol, "Unkn...6789");
        assert!(unknown_info.name.contains("Unknown Token"));
        assert_eq!(unknown_info.decimals, 0);
        assert_eq!(unknown_info.amount, 500000);
        assert_eq!(unknown_info.human_readable_amount, "500000");

        // Test with SOL
        let sol_info = get_token_info("11111111111111111111111111111111", 2000000000);
        assert_eq!(sol_info.symbol, "SOL");
        assert_eq!(sol_info.name, "Solana");
        assert_eq!(sol_info.decimals, 9);
        assert_eq!(sol_info.human_readable_amount, "2");
    }

    #[test]
    fn test_token_lookup_table() {
        let tokens = get_token_lookup_table();

        // Test some key tokens are present
        assert!(tokens.contains_key("11111111111111111111111111111111")); // SOL
        assert!(tokens.contains_key("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")); // USDC
        assert!(tokens.contains_key("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")); // USDT

        // Verify SOL details
        let sol = tokens.get("11111111111111111111111111111111").unwrap();
        assert_eq!(sol.symbol, "SOL");
        assert_eq!(sol.name, "Solana");
        assert_eq!(sol.decimals, 9);

        // Verify USDC details
        let usdc = tokens
            .get("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
            .unwrap();
        assert_eq!(usdc.symbol, "USDC");
        assert_eq!(usdc.name, "USD Coin");
        assert_eq!(usdc.decimals, 6);
    }
}
*/
