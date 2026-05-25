//! Token 2022 preset implementation for Solana

mod config;

use crate::core::{
    AccountRef, InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
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

fn resolve_program_id(context: &VisualizerContext) -> String {
    match context.program_id() {
        ProgramRef::Resolved(pk) => pk.to_string(),
        ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
    }
}

// Token 2022 extension instruction discriminators
const PAUSABLE_EXTENSION_DISCRIMINATOR: u8 = 44;
const PAUSABLE_PAUSE_DISCRIMINATOR: u8 = 1;
const PAUSABLE_RESUME_DISCRIMINATOR: u8 = 2;

// Standard Token instruction discriminators
const SET_AUTHORITY_DISCRIMINATOR: u8 = 6;

// Maximum decimals we are willing to honor on attacker-controlled
// `MintToChecked` / `BurnChecked` payloads. Real-world tokens cap out around
// 18 (ERC20 convention, and Solana's `Mint::decimals` is documented to fit in
// the same range); larger values are almost certainly malicious and would
// cause `10_u64.pow(decimals)` to overflow.
const MAX_TOKEN_DECIMALS: u8 = 18;

pub struct Token2022Visualizer;

impl InstructionVisualizer for Token2022Visualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        // Build AccountMeta shim for the parser (which expects &[AccountMeta]).
        // Unresolved accounts are rejected rather than substituted with
        // Pubkey::default(), which would render as a valid-looking address.
        let accounts: Vec<AccountMeta> = (0..context.num_accounts())
            .map(|i| match context.account(i) {
                Some(AccountRef::Resolved(pk)) => Ok(AccountMeta::new_readonly(*pk, false)),
                Some(AccountRef::Unresolved { raw_index }) => Err(VisualSignError::DecodeError(
                    format!("token_2022: unresolved account index {raw_index} at position {i}"),
                )),
                None => Err(VisualSignError::DecodeError(format!(
                    "token_2022: missing account at position {i}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Parse the Token 2022 instruction
        let token_2022_instruction = parse_token_2022_instruction(context.data(), &accounts)
            .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        // Generate proper preview layout
        create_token_2022_preview_layout(&token_2022_instruction, context)
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
    CloseAccount {
        account: String,
        destination: String,
        owner: String,
    },
    SetAuthority {
        account: String,
        authority_type: u8,
        authority_type_name: String,
        current_authority: String,
        new_authority: Option<String>,
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

    // Check for SetAuthority instruction
    if is_set_authority_instruction(data) {
        if accounts.len() < 2 {
            return Err("Invalid setAuthority: insufficient accounts".to_string());
        }

        // Parse authority type (data[1])
        if data.len() < 2 {
            return Err("Invalid setAuthority: insufficient data".to_string());
        }
        let authority_type = data[1];

        // Parse new authority (Option<Pubkey>)
        // Format: [discriminator, authority_type, option_flag, pubkey_bytes...]
        // option_flag: 0 = None, 1 = Some
        let new_authority = if data.len() >= 3 {
            match data[2] {
                0 => None, // None
                1 => {
                    // Some - pubkey is 32 bytes starting at data[3]
                    if data.len() >= 35 {
                        let pubkey_bytes = &data[3..35];
                        let pubkey = solana_sdk::pubkey::Pubkey::try_from(pubkey_bytes)
                            .map_err(|e| format!("Invalid pubkey in setAuthority: {e}"))?;
                        Some(pubkey.to_string())
                    } else {
                        return Err("Invalid setAuthority: incomplete pubkey data".to_string());
                    }
                }
                _ => return Err("Invalid setAuthority: invalid option flag".to_string()),
            }
        } else {
            None
        };

        let authority_type_name = get_authority_type_name(authority_type);

        return Ok(Token2022Instruction::SetAuthority {
            account: accounts[0].pubkey.to_string(),
            authority_type,
            authority_type_name,
            current_authority: accounts[1].pubkey.to_string(),
            new_authority,
        });
    }

    // Try to parse as standard TokenInstruction first
    if let Ok(sdk_instruction) = TokenInstruction::unpack(data) {
        match sdk_instruction {
            TokenInstruction::MintToChecked { amount, decimals } => {
                if accounts.len() < 3 {
                    return Err("Invalid mintToChecked: insufficient accounts".to_string());
                }
                if decimals > MAX_TOKEN_DECIMALS {
                    return Err(format!(
                        "Invalid mintToChecked: decimals {decimals} exceeds maximum supported value {MAX_TOKEN_DECIMALS}"
                    ));
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
                if decimals > MAX_TOKEN_DECIMALS {
                    return Err(format!(
                        "Invalid burnChecked: decimals {decimals} exceeds maximum supported value {MAX_TOKEN_DECIMALS}"
                    ));
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
            TokenInstruction::CloseAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid closeAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::CloseAccount {
                    account: accounts[0].pubkey.to_string(),
                    destination: accounts[1].pubkey.to_string(),
                    owner: accounts[2].pubkey.to_string(),
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

// Check if the instruction is a SetAuthority instruction
fn is_set_authority_instruction(data: &[u8]) -> bool {
    !data.is_empty() && data[0] == SET_AUTHORITY_DISCRIMINATOR
}

// Map authority type discriminator to human-readable name
fn get_authority_type_name(authority_type: u8) -> String {
    match authority_type {
        0 => "MintTokens".to_string(),
        1 => "FreezeAccount".to_string(),
        2 => "AccountOwner".to_string(),
        3 => "CloseAccount".to_string(),
        4 => "TransferFeeConfig".to_string(),
        5 => "WithheldWithdraw".to_string(),
        6 => "CloseMint".to_string(),
        7 => "InterestRate".to_string(),
        8 => "PermanentDelegate".to_string(),
        9 => "ConfidentialTransferMint".to_string(),
        10 => "TransferHookProgramId".to_string(),
        11 => "ConfidentialTransferFeeConfig".to_string(),
        12 => "MetadataPointer".to_string(),
        13 => "GroupPointer".to_string(),
        14 => "GroupMemberPointer".to_string(),
        15 => "ScaledUiAmount".to_string(),
        16 => "Pause".to_string(),
        _ => format!("Unknown({authority_type})"),
    }
}

fn create_token_2022_preview_layout(
    parsed: &Token2022Instruction,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
            ];

            (title, condensed, expanded)
        }

        Token2022Instruction::SetAuthority {
            account,
            authority_type,
            authority_type_name,
            current_authority,
            new_authority,
        } => {
            let new_authority_display = new_authority
                .clone()
                .unwrap_or_else(|| "None (Remove Authority)".to_string());
            let title = format!("Set Authority: {authority_type_name}");

            let condensed = vec![
                create_text_field("Action", "Set Authority")?,
                create_text_field("Authority Type", authority_type_name)?,
            ];

            let expanded = vec![
                create_text_field("Instruction", "Set Authority")?,
                create_text_field("Account", account)?,
                create_text_field("Authority Type", authority_type_name)?,
                create_number_field("Authority Type ID", &authority_type.to_string(), "")?,
                create_text_field("Current Authority", current_authority)?,
                create_text_field("New Authority", &new_authority_display)?,
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::CloseAccount {
            account,
            destination,
            owner,
        } => {
            let title = "Close Account".to_string();

            let condensed = vec![create_text_field("Action", "Close Account")?];

            let expanded = vec![
                create_text_field("Instruction", "Close Account")?,
                create_text_field("Token Account", account)?,
                create_text_field("Destination", destination)?,
                create_text_field("Owner", owner)?,
                create_text_field("Program ID", &resolve_program_id(context))?,
                create_raw_data_field(context.data(), None)?,
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
                fallback_text: format!(
                    "Token 2022: {title}\nProgram ID: {}",
                    resolve_program_id(context)
                ),
            },
            preview_layout,
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    mod fixture_test;

    fn dummy_account_metas(n: usize) -> Vec<AccountMeta> {
        (0..n)
            .map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false))
            .collect()
    }

    fn mint_to_checked_bytes(amount: u64, decimals: u8) -> Vec<u8> {
        // SPL Token MintToChecked layout:
        //   [discriminator: 14, amount: u64 LE, decimals: u8]
        let mut data = Vec::with_capacity(10);
        data.push(14u8);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(decimals);
        data
    }

    fn burn_checked_bytes(amount: u64, decimals: u8) -> Vec<u8> {
        // SPL Token BurnChecked layout:
        //   [discriminator: 15, amount: u64 LE, decimals: u8]
        let mut data = Vec::with_capacity(10);
        data.push(15u8);
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(decimals);
        data
    }

    /// Regression for PRS-221: a crafted `MintToChecked` instruction with
    /// `decimals = 100` used to cause the parser to compute `10_u64.pow(100)`,
    /// which wraps to `0` and triggers a divide-by-zero panic in
    /// `format_token_amount`. The fix is to reject any `decimals` value above
    /// `MAX_TOKEN_DECIMALS` at parse time, surfacing a clean decode error
    /// instead of crashing the worker.
    #[test]
    fn test_mint_to_checked_rejects_out_of_range_decimals() {
        let accounts = dummy_account_metas(3);
        for decimals in [19u8, 20, 64, 100, 200, u8::MAX] {
            let data = mint_to_checked_bytes(1_000_000, decimals);
            let msg = match parse_token_2022_instruction(&data, &accounts) {
                Ok(_) => panic!("decimals={decimals} should be rejected, got Ok"),
                Err(msg) => msg,
            };
            assert!(
                msg.contains("exceeds maximum supported value"),
                "unexpected error for decimals={decimals}: {msg}"
            );
        }
    }

    #[test]
    fn test_burn_checked_rejects_out_of_range_decimals() {
        let accounts = dummy_account_metas(3);
        for decimals in [19u8, 20, 64, 100, 200, u8::MAX] {
            let data = burn_checked_bytes(1_000_000, decimals);
            let msg = match parse_token_2022_instruction(&data, &accounts) {
                Ok(_) => panic!("decimals={decimals} should be rejected, got Ok"),
                Err(msg) => msg,
            };
            assert!(
                msg.contains("exceeds maximum supported value"),
                "unexpected error for decimals={decimals}: {msg}"
            );
        }
    }

    #[test]
    fn test_mint_to_checked_accepts_typical_decimals() {
        let accounts = dummy_account_metas(3);
        for decimals in [0u8, 6, 9, 18] {
            let data = mint_to_checked_bytes(1_000_000, decimals);
            if let Err(msg) = parse_token_2022_instruction(&data, &accounts) {
                panic!("decimals={decimals} should parse, got Err({msg})");
            }
        }
    }
}
