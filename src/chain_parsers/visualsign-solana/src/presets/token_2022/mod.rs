//! Token 2022 preset implementation for Solana

mod config;

use crate::core::{
    AccountRef, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    resolve_program_display,
};
use crate::utils::format_token_amount;
use config::Token2022Config;
use spl_token_2022_interface::extension::transfer_fee::instruction::TransferFeeInstruction;
use spl_token_2022_interface::instruction::TokenInstruction;
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

// Standard Token instruction discriminators
const SET_AUTHORITY_DISCRIMINATOR: u8 = 6;
const TRANSFER_FEE_EXTENSION_DISCRIMINATOR: u8 = 26;

// Maximum decimals we are willing to honor on attacker-controlled
// `MintToChecked` / `BurnChecked` payloads. Real-world tokens cap out around
// 18 (ERC20 convention, and Solana's `Mint::decimals` is documented to fit in
// the same range); larger values are almost certainly malicious. Clamping
// here surfaces a clean decode error to the caller. `format_token_amount`
// in `utils` also guards against out-of-range `decimals` via `checked_pow`
// (historically `10_u64.pow(64)` wrapped to 0 and panicked on divide), but
// rejecting up front gives a better diagnostic than silently rendering the
// raw amount.
const MAX_TOKEN_DECIMALS: u8 = 18;

pub struct Token2022Visualizer;

impl InstructionVisualizer for Token2022Visualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        // Unresolved accounts are rejected rather than substituted with
        // "unresolved(N)". Authority fields (pause_authority, mint_authority,
        // freeze_authority, etc.) are security-critical; showing an unresolvable
        // address would let a user approve an instruction whose authority they
        // cannot verify.
        let accounts: Vec<String> = (0..context.num_accounts())
            .map(|i| match context.account(i) {
                Some(AccountRef::Resolved(pk)) => Ok(pk.to_string()),
                Some(AccountRef::Unresolved { raw_index }) => Err(VisualSignError::DecodeError(
                    format!("token_2022: unresolved account index {raw_index} at position {i}"),
                )),
                None => Err(VisualSignError::DecodeError(format!(
                    "token_2022: missing account at position {i}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Instructions this preset does not decode (extensions such as
        // confidential transfers, checked transfers carrying transfer-hook or
        // multisig accounts, malformed data) degrade to a raw layout titled
        // "Token 2022 (unparsed)" instead of failing the whole transaction.
        match parse_token_2022_instruction(context.data(), &accounts) {
            Ok(token_2022_instruction) => {
                create_token_2022_preview_layout(&token_2022_instruction, context)
            }
            Err(reason) => create_unparsed_preview_layout(&reason, context),
        }
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&TOKEN_2022_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Token2022")
    }
}

enum Token2022Instruction {
    /// Deprecated in Token-2022 (no mint or decimals), still decodable.
    Transfer {
        amount: u64,
        source: String,
        destination: String,
        authority: String,
        signers: Vec<String>,
    },
    /// `TransferChecked`, or `TransferCheckedWithFee` with `fee`.
    TransferChecked {
        amount: u64,
        decimals: u8,
        fee: Option<u64>,
        source: String,
        mint: String,
        destination: String,
        authority: String,
    },
    Approve {
        amount: u64,
        decimals: Option<u8>,
        source: String,
        mint: Option<String>,
        delegate: String,
        owner: String,
        signers: Vec<String>,
    },
    Revoke {
        source: String,
        owner: String,
        signers: Vec<String>,
    },
    MintTo {
        amount: u64,
        mint: String,
        account: String,
        mint_authority: String,
        signers: Vec<String>,
    },
    Burn {
        amount: u64,
        account: String,
        mint: String,
        authority: String,
        signers: Vec<String>,
    },
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
    accounts: &[String],
) -> Result<Token2022Instruction, String> {
    // Check for Pause instruction
    if is_pause_instruction(data) {
        if accounts.len() < 2 {
            return Err("Invalid pause: insufficient accounts".to_string());
        }

        return Ok(Token2022Instruction::Pause {
            mint: accounts[0].clone(),
            pause_authority: accounts[1].clone(),
        });
    }

    // Check for Resume instruction
    if is_resume_instruction(data) {
        if accounts.len() < 2 {
            return Err("Invalid resume: insufficient accounts".to_string());
        }

        return Ok(Token2022Instruction::Resume {
            mint: accounts[0].clone(),
            pause_authority: accounts[1].clone(),
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
                        let pubkey = solana_pubkey::Pubkey::try_from(pubkey_bytes)
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
            account: accounts[0].clone(),
            authority_type,
            authority_type_name,
            current_authority: accounts[1].clone(),
            new_authority,
        });
    }

    if data.first() == Some(&TRANSFER_FEE_EXTENSION_DISCRIMINATOR) {
        return match TransferFeeInstruction::unpack(&data[1..]) {
            Ok(TransferFeeInstruction::TransferCheckedWithFee {
                amount,
                decimals,
                fee,
            }) => checked_transfer(amount, decimals, Some(fee), accounts),
            _ => Err("Unsupported Token 2022 transfer fee instruction".to_string()),
        };
    }

    // Try to parse as standard TokenInstruction first
    if let Ok(sdk_instruction) = TokenInstruction::unpack(data) {
        #[allow(deprecated)]
        match sdk_instruction {
            TokenInstruction::Transfer { amount } => {
                let [source, destination, authority] = required(accounts, "transfer")?;
                return Ok(Token2022Instruction::Transfer {
                    amount,
                    source,
                    destination,
                    authority,
                    signers: accounts[3..].to_vec(),
                });
            }
            TokenInstruction::TransferChecked { amount, decimals } => {
                return checked_transfer(amount, decimals, None, accounts);
            }
            TokenInstruction::Approve { amount } => {
                let [source, delegate, owner] = required(accounts, "approve")?;
                return Ok(Token2022Instruction::Approve {
                    amount,
                    decimals: None,
                    source,
                    mint: None,
                    delegate,
                    owner,
                    signers: accounts[3..].to_vec(),
                });
            }
            TokenInstruction::ApproveChecked { amount, decimals } => {
                check_decimals("approveChecked", decimals)?;
                let [source, mint, delegate, owner] = required(accounts, "approveChecked")?;
                return Ok(Token2022Instruction::Approve {
                    amount,
                    decimals: Some(decimals),
                    source,
                    mint: Some(mint),
                    delegate,
                    owner,
                    signers: accounts[4..].to_vec(),
                });
            }
            TokenInstruction::Revoke => {
                let [source, owner] = required(accounts, "revoke")?;
                return Ok(Token2022Instruction::Revoke {
                    source,
                    owner,
                    signers: accounts[2..].to_vec(),
                });
            }
            TokenInstruction::MintTo { amount } => {
                let [mint, account, mint_authority] = required(accounts, "mintTo")?;
                return Ok(Token2022Instruction::MintTo {
                    amount,
                    mint,
                    account,
                    mint_authority,
                    signers: accounts[3..].to_vec(),
                });
            }
            TokenInstruction::Burn { amount } => {
                let [account, mint, authority] = required(accounts, "burn")?;
                return Ok(Token2022Instruction::Burn {
                    amount,
                    account,
                    mint,
                    authority,
                    signers: accounts[3..].to_vec(),
                });
            }
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
                    mint: accounts[0].clone(),
                    account: accounts[1].clone(),
                    mint_authority: accounts[2].clone(),
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
                    account: accounts[0].clone(),
                    mint: accounts[1].clone(),
                    authority: accounts[2].clone(),
                });
            }
            TokenInstruction::FreezeAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid freezeAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::Freeze {
                    account: accounts[0].clone(),
                    mint: accounts[1].clone(),
                    freeze_authority: accounts[2].clone(),
                });
            }
            TokenInstruction::ThawAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid thawAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::Thaw {
                    account: accounts[0].clone(),
                    mint: accounts[1].clone(),
                    freeze_authority: accounts[2].clone(),
                });
            }
            TokenInstruction::CloseAccount => {
                if accounts.len() < 3 {
                    return Err("Invalid closeAccount: insufficient accounts".to_string());
                }

                return Ok(Token2022Instruction::CloseAccount {
                    account: accounts[0].clone(),
                    destination: accounts[1].clone(),
                    owner: accounts[2].clone(),
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

/// The first `N` accounts, or an error naming the instruction.
fn required<const N: usize>(accounts: &[String], name: &str) -> Result<[String; N], String> {
    accounts
        .get(..N)
        .and_then(|a| <[String; N]>::try_from(a.to_vec()).ok())
        .ok_or_else(|| format!("Invalid {name}: insufficient accounts"))
}

fn check_decimals(name: &str, decimals: u8) -> Result<(), String> {
    if decimals > MAX_TOKEN_DECIMALS {
        return Err(format!(
            "Invalid {name}: decimals {decimals} exceeds maximum supported value {MAX_TOKEN_DECIMALS}"
        ));
    }
    Ok(())
}

/// `TransferChecked` / `TransferCheckedWithFee`: source, mint, destination,
/// authority. Further accounts are either multisig signers or the extra
/// accounts of a transfer hook, which run another program's code; the
/// instruction alone cannot tell them apart, so they are not decoded.
fn checked_transfer(
    amount: u64,
    decimals: u8,
    fee: Option<u64>,
    accounts: &[String],
) -> Result<Token2022Instruction, String> {
    check_decimals("transferChecked", decimals)?;
    let [source, mint, destination, authority] = required(accounts, "transferChecked")?;
    if accounts.len() > 4 {
        return Err(format!(
            "transferChecked with {} extra accounts (transfer hook or multisig signers) is not decoded",
            accounts.len() - 4
        ));
    }
    Ok(Token2022Instruction::TransferChecked {
        amount,
        decimals,
        fee,
        source,
        mint,
        destination,
        authority,
    })
}

/// Raw layout for an instruction this preset does not decode.
fn create_unparsed_preview_layout(
    reason: &str,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let title = "Token 2022 (unparsed)".to_string();
    let condensed = vec![create_text_field("Instruction", &title)?];
    let expanded = vec![
        create_text_field("Instruction", &title)?,
        create_text_field("Reason", reason)?,
        create_text_field("Program ID", &resolve_program_display(context))?,
        create_raw_data_field(context.data(), None)?,
    ];
    preview_field(title, condensed, expanded, context)
}

fn signer_fields(signers: &[String]) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    signers
        .iter()
        .enumerate()
        .map(|(i, s)| create_text_field(&format!("Multisig Signer {}", i + 1), s))
        .collect()
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
    let program = resolve_program_display(context);
    let (title, condensed_fields, expanded_fields) = match parsed {
        Token2022Instruction::Transfer {
            amount,
            source,
            destination,
            authority,
            signers,
        } => {
            let title = format!("Transfer: {amount} base units");
            let condensed = vec![
                create_text_field("Action", "Transfer")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", "Transfer")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_text_field("Decimals", "not in instruction (deprecated Transfer)")?,
                create_text_field("Source", source)?,
                create_text_field("Destination", destination)?,
                create_text_field("Owner/Delegate", authority)?,
            ];
            expanded.extend(signer_fields(signers)?);
            expanded.push(create_text_field("Program ID", &program)?);
            expanded.push(create_raw_data_field(context.data(), None)?);
            (title, condensed, expanded)
        }
        Token2022Instruction::TransferChecked {
            amount,
            decimals,
            fee,
            source,
            mint,
            destination,
            authority,
        } => {
            let formatted = format_token_amount(*amount, *decimals);
            let name = if fee.is_some() {
                "Transfer Checked With Fee"
            } else {
                "Transfer Checked"
            };
            let title = format!("{name}: {formatted} tokens");
            let condensed = vec![
                create_text_field("Action", name)?,
                create_text_field("Amount", &formatted)?,
                create_text_field("Destination", destination)?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", name)?,
                create_text_field("Amount", &formatted)?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_number_field("Decimals", &decimals.to_string(), "")?,
            ];
            if let Some(fee) = fee {
                expanded.push(create_text_field(
                    "Fee",
                    &format_token_amount(*fee, *decimals),
                )?);
            }
            expanded.extend([
                create_text_field("Mint", mint)?,
                create_text_field("Source", source)?,
                create_text_field("Destination", destination)?,
                create_text_field("Owner/Delegate", authority)?,
                create_text_field("Program ID", &program)?,
                create_raw_data_field(context.data(), None)?,
            ]);
            (title, condensed, expanded)
        }
        Token2022Instruction::Approve {
            amount,
            decimals,
            source,
            mint,
            delegate,
            owner,
            signers,
        } => {
            let shown = match decimals {
                Some(d) => format_token_amount(*amount, *d),
                None => format!("{amount} base units"),
            };
            let name = if decimals.is_some() {
                "Approve Checked"
            } else {
                "Approve"
            };
            let title = format!("{name}: {shown}");
            let condensed = vec![
                create_text_field("Action", name)?,
                create_text_field("Amount", &shown)?,
                create_text_field("Delegate", delegate)?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", name)?,
                create_text_field("Amount", &shown)?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
            ];
            if let (Some(d), Some(m)) = (decimals, mint) {
                expanded.push(create_number_field("Decimals", &d.to_string(), "")?);
                expanded.push(create_text_field("Mint", m)?);
            }
            expanded.extend([
                create_text_field("Source", source)?,
                create_text_field("Delegate", delegate)?,
                create_text_field("Owner", owner)?,
            ]);
            expanded.extend(signer_fields(signers)?);
            expanded.push(create_text_field("Program ID", &program)?);
            expanded.push(create_raw_data_field(context.data(), None)?);
            (title, condensed, expanded)
        }
        Token2022Instruction::Revoke {
            source,
            owner,
            signers,
        } => {
            let title = "Revoke Delegate".to_string();
            let condensed = vec![create_text_field("Action", "Revoke")?];
            let mut expanded = vec![
                create_text_field("Instruction", "Revoke")?,
                create_text_field("Source", source)?,
                create_text_field("Owner", owner)?,
            ];
            expanded.extend(signer_fields(signers)?);
            expanded.push(create_text_field("Program ID", &program)?);
            expanded.push(create_raw_data_field(context.data(), None)?);
            (title, condensed, expanded)
        }
        Token2022Instruction::MintTo {
            amount,
            mint,
            account,
            mint_authority,
            signers,
        } => {
            let title = format!("Mint To: {amount} base units");
            let condensed = vec![
                create_text_field("Action", "Mint To")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", "Mint To")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_text_field("Mint", mint)?,
                create_text_field("Destination Account", account)?,
                create_text_field("Mint Authority", mint_authority)?,
            ];
            expanded.extend(signer_fields(signers)?);
            expanded.push(create_text_field("Program ID", &program)?);
            expanded.push(create_raw_data_field(context.data(), None)?);
            (title, condensed, expanded)
        }
        Token2022Instruction::Burn {
            amount,
            account,
            mint,
            authority,
            signers,
        } => {
            let title = format!("Burn: {amount} base units");
            let condensed = vec![
                create_text_field("Action", "Burn")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", "Burn")?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_text_field("Token Account", account)?,
                create_text_field("Mint", mint)?,
                create_text_field("Authority", authority)?,
            ];
            expanded.extend(signer_fields(signers)?);
            expanded.push(create_text_field("Program ID", &program)?);
            expanded.push(create_raw_data_field(context.data(), None)?);
            (title, condensed, expanded)
        }
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
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
                create_text_field("Program ID", &resolve_program_display(context))?,
                create_raw_data_field(context.data(), None)?,
            ];

            (title, condensed, expanded)
        }
    };

    preview_field(title, condensed_fields, expanded_fields, context)
}

fn preview_field(
    title: String,
    condensed_fields: Vec<AnnotatedPayloadField>,
    expanded_fields: Vec<AnnotatedPayloadField>,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
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
                    resolve_program_display(context)
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
    use solana_pubkey::Pubkey;
    mod fixture_test;

    fn dummy_account_strings(n: usize) -> Vec<String> {
        (0..n).map(|_| Pubkey::new_unique().to_string()).collect()
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

    /// Regression: a crafted `MintToChecked` instruction with
    /// `decimals = 100` used to cause the parser to compute `10_u64.pow(100)`,
    /// which wraps to `0` and triggers a divide-by-zero panic in
    /// `format_token_amount`. The fix is to reject any `decimals` value above
    /// `MAX_TOKEN_DECIMALS` at parse time, surfacing a clean decode error
    /// instead of crashing the worker.
    #[test]
    fn test_mint_to_checked_rejects_out_of_range_decimals() {
        let accounts = dummy_account_strings(3);
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
        let accounts = dummy_account_strings(3);
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
        let accounts = dummy_account_strings(3);
        for decimals in [0u8, 6, 9, 18] {
            let data = mint_to_checked_bytes(1_000_000, decimals);
            if let Err(msg) = parse_token_2022_instruction(&data, &accounts) {
                panic!("decimals={decimals} should parse, got Err({msg})");
            }
        }
    }

    fn amount_decimals(tag: u8, amount: u64, decimals: u8) -> Vec<u8> {
        let mut data = vec![tag];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(decimals);
        data
    }

    /// Visualize `data` with `n` accounts and return the preview title and
    /// expanded rows as `label: text`.
    fn visualize(data: Vec<u8>, accounts: &[Pubkey]) -> (String, Vec<String>) {
        let mut keys = vec![spl_token_2022_interface::id()];
        keys.extend_from_slice(accounts);
        let compiled = solana_message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: (1..=accounts.len() as u8).collect(),
            data,
        };
        let sender = solana_parser::solana::structs::SolanaAccount {
            account_key: accounts[0].to_string(),
            signer: false,
            writable: false,
        };
        let idl = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, &compiled, &keys, &idl, 0);
        let field = Token2022Visualizer.visualize_tx_commands(&context).unwrap();
        let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            field.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        let rows = preview_layout
            .expanded
            .unwrap()
            .fields
            .iter()
            .map(|f| {
                let f = &f.signable_payload_field;
                format!("{}: {}", f.label(), f.fallback_text())
            })
            .collect();
        (preview_layout.title.unwrap().text, rows)
    }

    fn keys(n: u8) -> Vec<Pubkey> {
        (1..=n).map(|i| Pubkey::new_from_array([i; 32])).collect()
    }

    #[test]
    fn transfer_checked_shows_mint_amount_decimals_source_destination() {
        let k = keys(4);
        let (title, rows) = visualize(amount_decimals(12, 1_500_000, 6), &k);
        assert_eq!(title, "Transfer Checked: 1.5 tokens");
        for row in [
            "Instruction: Transfer Checked".to_string(),
            "Amount: 1.5".to_string(),
            "Raw Amount: 1500000".to_string(),
            "Decimals: 6".to_string(),
            format!("Source: {}", k[0]),
            format!("Mint: {}", k[1]),
            format!("Destination: {}", k[2]),
            format!("Owner/Delegate: {}", k[3]),
        ] {
            assert!(rows.contains(&row), "{row} missing in {rows:#?}");
        }
    }

    #[test]
    fn transfer_checked_with_fee_shows_fee() {
        let k = keys(4);
        let mut data = vec![26, 1];
        data.extend_from_slice(&2_000_000u64.to_le_bytes());
        data.push(6);
        data.extend_from_slice(&10_000u64.to_le_bytes());
        let (title, rows) = visualize(data, &k);
        assert_eq!(title, "Transfer Checked With Fee: 2 tokens");
        assert!(rows.contains(&"Fee: 0.01".to_string()), "{rows:#?}");
        assert!(
            rows.contains(&format!("Destination: {}", k[2])),
            "{rows:#?}"
        );
    }

    #[test]
    fn approve_checked_and_revoke() {
        let k = keys(4);
        let (title, rows) = visualize(amount_decimals(13, 5_000_000, 6), &k);
        assert_eq!(title, "Approve Checked: 5");
        for row in [
            format!("Source: {}", k[0]),
            format!("Mint: {}", k[1]),
            format!("Delegate: {}", k[2]),
            format!("Owner: {}", k[3]),
        ] {
            assert!(rows.contains(&row), "{row} missing in {rows:#?}");
        }
        let (title, rows) = visualize(vec![5], &k[..2]);
        assert_eq!(title, "Revoke Delegate");
        assert!(rows.contains(&format!("Owner: {}", k[1])), "{rows:#?}");
    }

    #[test]
    fn deprecated_transfer_and_multisig_signers() {
        // Transfer with a multisig owner and two signer accounts.
        let k = keys(5);
        let mut data = vec![3];
        data.extend_from_slice(&42u64.to_le_bytes());
        let (title, rows) = visualize(data, &k);
        assert_eq!(title, "Transfer: 42 base units");
        assert!(
            rows.contains(&format!("Destination: {}", k[1])),
            "{rows:#?}"
        );
        assert!(
            rows.contains(&format!("Multisig Signer 1: {}", k[3])),
            "{rows:#?}"
        );
        assert!(
            rows.contains(&format!("Multisig Signer 2: {}", k[4])),
            "{rows:#?}"
        );
    }

    #[test]
    fn mint_to_and_burn() {
        let k = keys(3);
        let mut data = vec![7];
        data.extend_from_slice(&9u64.to_le_bytes());
        assert_eq!(visualize(data, &k).0, "Mint To: 9 base units");
        let mut data = vec![8];
        data.extend_from_slice(&9u64.to_le_bytes());
        assert_eq!(visualize(data, &k).0, "Burn: 9 base units");
    }

    #[test]
    fn transfer_checked_with_extra_accounts_is_unparsed() {
        // A transfer hook's extra accounts (or multisig signers) follow the
        // authority; they cannot be told apart, so nothing is decoded.
        let (title, rows) = visualize(amount_decimals(12, 1, 0), &keys(6));
        assert_eq!(title, "Token 2022 (unparsed)");
        assert!(
            rows.iter().any(|r| r.contains("2 extra accounts")),
            "{rows:#?}"
        );
    }

    #[test]
    fn confidential_transfer_and_unknown_are_unparsed() {
        // 27 = ConfidentialTransferExtension, 0xfe = no such instruction.
        for data in [vec![27, 7, 0, 0], vec![0xfe]] {
            assert_eq!(visualize(data, &keys(4)).0, "Token 2022 (unparsed)");
        }
    }
}
