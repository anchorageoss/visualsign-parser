//! Token 2022 preset implementation for Solana

mod confidential_transfer;
mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use crate::utils::format_token_amount;
use confidential_transfer::ConfidentialTransferIx;
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

// Standard Token instruction discriminators
const SET_AUTHORITY_DISCRIMINATOR: u8 = 6;

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
    ConfidentialWithdraw(ConfidentialTransferIx),
    ConfidentialTransfer(ConfidentialTransferIx),
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

    // Check for ConfidentialTransfer extension (Withdraw / Transfer sub-instructions)
    if data.first().copied()
        == Some(confidential_transfer::CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR)
    {
        let account_keys: Vec<String> = accounts.iter().map(|a| a.pubkey.to_string()).collect();
        if let Some(ix) =
            confidential_transfer::try_decode_confidential_transfer(data, &account_keys)?
        {
            return Ok(match ix {
                ConfidentialTransferIx::Withdraw { .. } => {
                    Token2022Instruction::ConfidentialWithdraw(ix)
                }
                ConfidentialTransferIx::Transfer { .. } => {
                    Token2022Instruction::ConfidentialTransfer(ix)
                }
            });
        }
        // Other CT sub-instructions fall through to the unsupported path below.
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
                create_text_field("Program ID", &instruction.program_id.to_string())?,
                create_raw_data_field(&instruction.data, Some(hex::encode(&instruction.data)))?,
            ];

            (title, condensed, expanded)
        }
        Token2022Instruction::ConfidentialWithdraw(ConfidentialTransferIx::Withdraw {
            source_token_account,
            mint,
            owner,
            amount,
            decimals,
            new_decryptable_available_balance,
            equality_proof_context_account,
            range_proof_context_account,
        }) => {
            let formatted_amount = format_token_amount(*amount, *decimals);
            let title = format!("Confidential Withdraw: {formatted_amount} tokens");
            let condensed = vec![
                create_text_field("Action", "Confidential Withdraw")?,
                create_text_field("Amount", &formatted_amount)?,
            ];
            let mut expanded = vec![
                create_text_field("Instruction", "Confidential Withdraw")?,
                create_text_field("Amount", &formatted_amount)?,
                create_number_field("Raw Amount", &amount.to_string(), "")?,
                create_number_field("Decimals", &decimals.to_string(), "")?,
                create_text_field("Mint", mint)?,
                create_text_field("Source Token Account", source_token_account)?,
                create_text_field("Owner", owner)?,
            ];
            if let Some(a) = equality_proof_context_account {
                expanded.push(create_text_field("Equality Proof Context Account", a)?);
            }
            if let Some(a) = range_proof_context_account {
                expanded.push(create_text_field("Range Proof Context Account", a)?);
            }
            // Opaque / fallback at the bottom.
            expanded.push(create_text_field(
                "New Decryptable Available Balance (encrypted; wallet decrypts)",
                new_decryptable_available_balance,
            )?);
            expanded.push(create_text_field(
                "Program ID",
                &instruction.program_id.to_string(),
            )?);
            expanded.push(create_raw_data_field(
                &instruction.data,
                Some(hex::encode(&instruction.data)),
            )?);
            (title, condensed, expanded)
        }
        Token2022Instruction::ConfidentialTransfer(ConfidentialTransferIx::Transfer {
            source_token_account,
            mint,
            destination_token_account,
            owner,
            new_source_decryptable_available_balance,
            auditor_configured,
            equality_proof_context_account,
            validity_proof_context_account,
            range_proof_context_account,
        }) => {
            let decoded = context.confidential_decoded_amount();
            let title = match decoded {
                Some(a) => format!("Confidential Transfer: {a} tokens"),
                None => "Confidential Transfer".to_string(),
            };
            let mut condensed = vec![create_text_field("Action", "Confidential Transfer")?];
            if let Some(a) = decoded {
                condensed.push(create_text_field(
                    "Amount (wallet-decoded)",
                    &a.to_string(),
                )?);
            }
            let mut expanded = vec![create_text_field("Instruction", "Confidential Transfer")?];
            if let Some(a) = decoded {
                expanded.push(create_text_field(
                    "Amount (wallet-decoded)",
                    &a.to_string(),
                )?);
            }
            expanded.push(create_text_field("Mint", mint)?);
            expanded.push(create_text_field(
                "Source Token Account",
                source_token_account,
            )?);
            expanded.push(create_text_field(
                "Destination Token Account",
                destination_token_account,
            )?);
            expanded.push(create_text_field("Owner", owner)?);
            if let Some(a) = equality_proof_context_account {
                expanded.push(create_text_field("Equality Proof Context Account", a)?);
            }
            if let Some(a) = validity_proof_context_account {
                expanded.push(create_text_field("Validity Proof Context Account", a)?);
            }
            if let Some(a) = range_proof_context_account {
                expanded.push(create_text_field("Range Proof Context Account", a)?);
            }
            // Opaque / fallback at the bottom.
            expanded.push(create_text_field(
                "Auditor Configured",
                if *auditor_configured { "yes" } else { "no" },
            )?);
            expanded.push(create_text_field(
                "New Source Decryptable Available Balance (encrypted; wallet decrypts)",
                new_source_decryptable_available_balance,
            )?);
            expanded.push(create_text_field(
                "Program ID",
                &instruction.program_id.to_string(),
            )?);
            expanded.push(create_raw_data_field(
                &instruction.data,
                Some(hex::encode(&instruction.data)),
            )?);
            (title, condensed, expanded)
        }
        // `ConfidentialWithdraw` and `ConfidentialTransfer` are constructed
        // exclusively from the matching `ConfidentialTransferIx` variant in
        // `parse_token_2022_instruction`; the mismatched combinations below
        // are unreachable in practice but must be handled since the wrapped
        // enum has two variants.
        Token2022Instruction::ConfidentialWithdraw(ConfidentialTransferIx::Transfer { .. })
        | Token2022Instruction::ConfidentialTransfer(ConfidentialTransferIx::Withdraw { .. }) => {
            return Err(VisualSignError::DecodeError(
                "confidential_transfer: mismatched instruction variant".to_string(),
            ));
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use bytemuck::bytes_of;
    use solana_sdk::pubkey::Pubkey;
    use spl_token_2022_interface::extension::confidential_transfer::instruction::{
        ConfidentialTransferInstruction, TransferInstructionData, WithdrawInstructionData,
    };
    use std::str::FromStr;

    mod fixture_test;

    fn meta(pubkey_seed: u8) -> AccountMeta {
        let mut bytes = [0u8; 32];
        bytes[0] = pubkey_seed;
        AccountMeta {
            pubkey: Pubkey::new_from_array(bytes),
            is_signer: false,
            is_writable: false,
        }
    }

    #[test]
    fn withdraw_renders_amount_in_title() {
        let d = WithdrawInstructionData {
            amount: 1_500_000u64.into(),
            decimals: 6,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        let mut data = vec![
            confidential_transfer::CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR,
            ConfidentialTransferInstruction::Withdraw as u8,
        ];
        data.extend_from_slice(bytes_of(&d));
        // Accounts: [src, mint, equality_ctx, range_ctx, owner]
        let accounts: Vec<AccountMeta> = (0..5).map(meta).collect();

        let ix = parse_token_2022_instruction(&data, &accounts).unwrap();
        assert!(matches!(ix, Token2022Instruction::ConfidentialWithdraw(_)));

        let instruction = solana_sdk::instruction::Instruction {
            program_id: Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap(),
            accounts,
            data,
        };
        let sender = solana_parser::solana::structs::SolanaAccount {
            account_key: "11111111111111111111111111111111".to_string(),
            signer: false,
            writable: false,
        };
        let instructions = vec![instruction.clone()];
        let idl_registry = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, 0, &instructions, &idl_registry, None);

        let field = create_token_2022_preview_layout(&ix, &instruction, &context).unwrap();
        let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            field.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        let title = preview_layout.title.unwrap().text;
        assert_eq!(title, "Confidential Withdraw: 1.5 tokens");
    }

    #[test]
    fn transfer_with_wallet_decoded_amount_shows_amount_in_title() {
        let d = TransferInstructionData {
            new_source_decryptable_available_balance: Default::default(),
            transfer_amount_auditor_ciphertext_lo: Default::default(),
            transfer_amount_auditor_ciphertext_hi: Default::default(),
            equality_proof_instruction_offset: 0,
            ciphertext_validity_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        let mut data = vec![
            confidential_transfer::CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR,
            ConfidentialTransferInstruction::Transfer as u8,
        ];
        data.extend_from_slice(bytes_of(&d));
        // Accounts: [src, mint, dest, eqctx, validityctx, rngctx, owner]
        let accounts: Vec<AccountMeta> = (0..7).map(meta).collect();

        let ix = parse_token_2022_instruction(&data, &accounts).unwrap();
        assert!(matches!(ix, Token2022Instruction::ConfidentialTransfer(_)));

        let instruction = solana_sdk::instruction::Instruction {
            program_id: Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap(),
            accounts,
            data,
        };
        let sender = solana_parser::solana::structs::SolanaAccount {
            account_key: "11111111111111111111111111111111".to_string(),
            signer: false,
            writable: false,
        };
        let instructions = vec![instruction.clone()];
        let idl_registry = crate::idl::IdlRegistry::new();

        // With a wallet-supplied decoded amount, the title and condensed
        // fields include it.
        let context_with_amount =
            VisualizerContext::new(&sender, 0, &instructions, &idl_registry, Some(42_000u64));
        let field_with_amount =
            create_token_2022_preview_layout(&ix, &instruction, &context_with_amount).unwrap();
        let SignablePayloadField::PreviewLayout {
            preview_layout: layout_with_amount,
            ..
        } = field_with_amount.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        assert_eq!(
            layout_with_amount.title.unwrap().text,
            "Confidential Transfer: 42000 tokens"
        );
        let condensed_with_amount = layout_with_amount.condensed.unwrap().fields;
        let has_condensed_amount =
            condensed_with_amount
                .iter()
                .any(|f| match &f.signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 } => {
                        common.label == "Amount (wallet-decoded)" && text_v2.text == "42000"
                    }
                    _ => false,
                });
        assert!(
            has_condensed_amount,
            "expected condensed wallet-decoded amount field"
        );

        // With no wallet-supplied amount, the title has no amount, and no
        // amount field is present.
        let context_without_amount =
            VisualizerContext::new(&sender, 0, &instructions, &idl_registry, None);
        let field_without_amount =
            create_token_2022_preview_layout(&ix, &instruction, &context_without_amount).unwrap();
        let SignablePayloadField::PreviewLayout {
            preview_layout: layout_without_amount,
            ..
        } = field_without_amount.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        assert_eq!(
            layout_without_amount.title.unwrap().text,
            "Confidential Transfer"
        );
        let expanded_without_amount = layout_without_amount.expanded.unwrap().fields;
        let has_amount_field = expanded_without_amount.iter().any(|f| {
            matches!(&f.signable_payload_field, SignablePayloadField::TextV2 { common, .. } if common.label == "Amount (wallet-decoded)")
        });
        assert!(
            !has_amount_field,
            "did not expect a wallet-decoded amount field when context has none"
        );
    }

    #[test]
    fn withdraw_encrypted_fields_render_at_bottom_of_expanded_list() {
        let d = WithdrawInstructionData {
            amount: 10u64.into(),
            decimals: 0,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        let mut data = vec![
            confidential_transfer::CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR,
            ConfidentialTransferInstruction::Withdraw as u8,
        ];
        data.extend_from_slice(bytes_of(&d));
        let accounts: Vec<AccountMeta> = (0..5).map(meta).collect();
        let ix = parse_token_2022_instruction(&data, &accounts).unwrap();

        let instruction = solana_sdk::instruction::Instruction {
            program_id: Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap(),
            accounts,
            data,
        };
        let sender = solana_parser::solana::structs::SolanaAccount {
            account_key: "11111111111111111111111111111111".to_string(),
            signer: false,
            writable: false,
        };
        let instructions = vec![instruction.clone()];
        let idl_registry = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, 0, &instructions, &idl_registry, None);

        let field = create_token_2022_preview_layout(&ix, &instruction, &context).unwrap();
        let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            field.signable_payload_field
        else {
            panic!("expected PreviewLayout");
        };
        let expanded = preview_layout.expanded.unwrap().fields;
        let encrypted_label = "New Decryptable Available Balance (encrypted; wallet decrypts)";
        let encrypted_idx = expanded
            .iter()
            .position(|f| matches!(&f.signable_payload_field, SignablePayloadField::TextV2 { common, .. } if common.label == encrypted_label))
            .expect("expected encrypted balance field to be present");
        // The encrypted/opaque field must be the last human-labeled field,
        // i.e. only the Program ID and raw data fields (also opaque/fallback)
        // may come after it.
        assert!(
            encrypted_idx >= expanded.len() - 3,
            "expected encrypted field near bottom of expanded list, got index {encrypted_idx} of {}",
            expanded.len()
        );
    }
}
