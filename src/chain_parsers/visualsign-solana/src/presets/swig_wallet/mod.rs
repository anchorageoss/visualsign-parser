//! Swig wallet preset implementation for Solana

mod config;

use std::fmt;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    available_visualizers, visualize_with_any,
};
use config::SwigWalletConfig;
use solana_parser::solana::structs::SolanaAccount;
use solana_program::system_instruction::SystemInstruction;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
};
use solana_system_interface::program as system_program;
use spl_token::instruction::{AuthorityType as SplAuthorityType, TokenInstruction};
use std::convert::TryInto;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_text_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

static SWIG_WALLET_CONFIG: SwigWalletConfig = SwigWalletConfig;

pub struct SwigWalletVisualizer;

impl InstructionVisualizer for SwigWalletVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        // Convert 0-based index to 1-based instruction number for user-facing labels
        // (e.g., "Instruction 1" instead of "Instruction 0")
        let instruction_number = context.instruction_index() + 1;
        let decoded = parse_swig_instruction(&instruction.data, &instruction.accounts)
            .map_err(|err| VisualSignError::DecodeError(err.to_string()))?;

        let summary = decoded.summary();
        let mut expanded_fields = vec![
            make_text_field("Program ID", instruction.program_id.to_string())?,
            make_text_field("Instruction Type", decoded.name())?,
        ];
        expanded_fields.extend(build_variant_fields(&decoded)?);

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![make_text_field("Instruction", summary.clone())?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: summary.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: "Swig wallet instruction".to_string(),
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
                    label: format!("Instruction {instruction_number}"),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SWIG_WALLET_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Swig Wallet")
    }
}

#[derive(Debug, Clone)]
struct SignInstructionDecoded {
    role_id: u32,
    instruction_payload_len: usize,
    authority_payload: Vec<u8>,
    inner_instructions: Vec<DecodedInnerInstruction>,
}

#[derive(Debug, Clone)]
struct DecodedInnerInstruction {
    program_id: Option<Pubkey>,
    program_display: String,
    accounts: Vec<InnerAccountMeta>,
    data: Vec<u8>,
    description: String,
}

#[derive(Debug, Clone)]
struct InnerAccountMeta {
    pubkey: Option<Pubkey>,
    display: String,
    is_signer: bool,
    is_writable: bool,
}

#[derive(Debug, Clone)]
enum AuthorityUpdateDetails {
    ReplaceAll(Vec<u8>),
    AddActions(Vec<u8>),
    RemoveActionsByType(Vec<u8>),
    RemoveActionsByIndex(Vec<u16>),
    Legacy(Vec<u8>),
}

#[derive(Debug, Clone)]
enum SwigInstructionDecoded {
    CreateV1 {
        authority_type: u16,
        bump: u8,
        wallet_address_bump: u8,
        wallet_id: [u8; 32],
        authority_data: Vec<u8>,
        actions: Vec<u8>,
    },
    AddAuthorityV1 {
        acting_role_id: u32,
        new_authority_type: u16,
        num_actions: u8,
        authority_data: Vec<u8>,
        actions: Vec<u8>,
        authority_payload: Vec<u8>,
    },
    RemoveAuthorityV1 {
        acting_role_id: u32,
        authority_to_remove_id: u32,
        authority_payload: Vec<u8>,
    },
    UpdateAuthorityV1 {
        acting_role_id: u32,
        authority_to_update_id: u32,
        operation: AuthorityUpdateDetails,
        authority_payload: Vec<u8>,
    },
    SignV1(SignInstructionDecoded),
    SignV2(SignInstructionDecoded),
    CreateSessionV1 {
        role_id: u32,
        session_duration: u64,
        session_key: [u8; 32],
        authority_payload: Vec<u8>,
    },
    CreateSubAccountV1 {
        role_id: u32,
        sub_account_bump: u8,
        authority_payload: Vec<u8>,
    },
    WithdrawFromSubAccountV1 {
        role_id: u32,
        amount: u64,
        authority_payload: Vec<u8>,
    },
    SubAccountSignV1(SignInstructionDecoded),
    ToggleSubAccountV1 {
        target_role_id: u32,
        authority_role_id: u32,
        enabled: bool,
        authority_payload: Vec<u8>,
    },
    MigrateToWalletAddressV1 {
        wallet_address_bump: u8,
    },
    TransferAssetsV1 {
        role_id: u32,
        authority_payload: Vec<u8>,
    },
    Unknown {
        discriminator: u16,
        raw_data: Vec<u8>,
    },
}

impl SwigInstructionDecoded {
    fn name(&self) -> &'static str {
        match self {
            SwigInstructionDecoded::CreateV1 { .. } => "Create Wallet",
            SwigInstructionDecoded::AddAuthorityV1 { .. } => "Add Authority",
            SwigInstructionDecoded::RemoveAuthorityV1 { .. } => "Remove Authority",
            SwigInstructionDecoded::UpdateAuthorityV1 { .. } => "Update Authority",
            SwigInstructionDecoded::SignV1(_) => "Sign Transaction (v1)",
            SwigInstructionDecoded::SignV2(_) => "Sign Transaction (v2)",
            SwigInstructionDecoded::CreateSessionV1 { .. } => "Create Session",
            SwigInstructionDecoded::CreateSubAccountV1 { .. } => "Create Sub-Account",
            SwigInstructionDecoded::WithdrawFromSubAccountV1 { .. } => "Withdraw From Sub-Account",
            SwigInstructionDecoded::SubAccountSignV1(_) => "Sub-Account Sign",
            SwigInstructionDecoded::ToggleSubAccountV1 { .. } => "Toggle Sub-Account",
            SwigInstructionDecoded::MigrateToWalletAddressV1 { .. } => "Migrate Wallet",
            SwigInstructionDecoded::TransferAssetsV1 { .. } => "Transfer Assets",
            SwigInstructionDecoded::Unknown { .. } => "Unknown",
        }
    }

    fn summary(&self) -> String {
        match self {
            SwigInstructionDecoded::CreateV1 { authority_type, .. } => format!(
                "Swig: Create wallet ({})",
                authority_type_name(*authority_type)
            ),
            SwigInstructionDecoded::AddAuthorityV1 {
                acting_role_id,
                new_authority_type,
                ..
            } => format!(
                "Swig: Add authority role #{acting_role_id} ({})",
                authority_type_name(*new_authority_type)
            ),
            SwigInstructionDecoded::RemoveAuthorityV1 {
                acting_role_id,
                authority_to_remove_id,
                ..
            } => format!(
                "Swig: Remove authority #{authority_to_remove_id} (by role #{acting_role_id})"
            ),
            SwigInstructionDecoded::UpdateAuthorityV1 {
                acting_role_id,
                authority_to_update_id,
                ..
            } => format!(
                "Swig: Update authority #{authority_to_update_id} (by role #{acting_role_id})"
            ),
            SwigInstructionDecoded::SignV1(sign) => format!(
                "Swig: Sign v1 ({count} inner instruction(s), role #{role})",
                count = sign.inner_instructions.len(),
                role = sign.role_id
            ),
            SwigInstructionDecoded::SignV2(sign) => format!(
                "Swig: Sign v2 ({count} inner instruction(s), role #{role})",
                count = sign.inner_instructions.len(),
                role = sign.role_id
            ),
            SwigInstructionDecoded::CreateSessionV1 { role_id, .. } => {
                format!("Swig: Create session (role #{role_id})")
            }
            SwigInstructionDecoded::CreateSubAccountV1 { role_id, .. } => {
                format!("Swig: Create sub-account (role #{role_id})")
            }
            SwigInstructionDecoded::WithdrawFromSubAccountV1 {
                role_id, amount, ..
            } => {
                format!("Swig: Withdraw {amount} lamports from sub-account (role #{role_id})")
            }
            SwigInstructionDecoded::SubAccountSignV1(sign) => format!(
                "Swig: Sub-account sign ({count} inner instruction(s), role #{role})",
                count = sign.inner_instructions.len(),
                role = sign.role_id
            ),
            SwigInstructionDecoded::ToggleSubAccountV1 {
                target_role_id,
                enabled,
                ..
            } => {
                let state = if *enabled { "enable" } else { "disable" };
                format!("Swig: {state} sub-account role #{target_role_id}")
            }
            SwigInstructionDecoded::MigrateToWalletAddressV1 { .. } => {
                "Swig: Migrate wallet".to_string()
            }
            SwigInstructionDecoded::TransferAssetsV1 { role_id, .. } => {
                format!("Swig: Transfer assets (role #{role_id})")
            }
            SwigInstructionDecoded::Unknown { discriminator, .. } => {
                format!("Swig: Unknown instruction ({discriminator})")
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
enum SwigInstructionKind {
    CreateV1 = 0,
    AddAuthorityV1 = 1,
    RemoveAuthorityV1 = 2,
    UpdateAuthorityV1 = 3,
    SignV1 = 4,
    CreateSessionV1 = 5,
    CreateSubAccountV1 = 6,
    WithdrawFromSubAccountV1 = 7,
    SubAccountSignV1 = 9,
    ToggleSubAccountV1 = 10,
    SignV2 = 11,
    MigrateToWalletAddressV1 = 12,
    TransferAssetsV1 = 13,
}

impl TryFrom<u16> for SwigInstructionKind {
    type Error = SwigParseError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(SwigInstructionKind::CreateV1),
            1 => Ok(SwigInstructionKind::AddAuthorityV1),
            2 => Ok(SwigInstructionKind::RemoveAuthorityV1),
            3 => Ok(SwigInstructionKind::UpdateAuthorityV1),
            4 => Ok(SwigInstructionKind::SignV1),
            5 => Ok(SwigInstructionKind::CreateSessionV1),
            6 => Ok(SwigInstructionKind::CreateSubAccountV1),
            7 => Ok(SwigInstructionKind::WithdrawFromSubAccountV1),
            9 => Ok(SwigInstructionKind::SubAccountSignV1),
            10 => Ok(SwigInstructionKind::ToggleSubAccountV1),
            11 => Ok(SwigInstructionKind::SignV2),
            12 => Ok(SwigInstructionKind::MigrateToWalletAddressV1),
            13 => Ok(SwigInstructionKind::TransferAssetsV1),
            other => Err(SwigParseError::UnknownInstruction(other)),
        }
    }
}

#[derive(Debug)]
enum SwigParseError {
    DataTooShort(&'static str),
    InvalidFormat(&'static str),
    UnknownInstruction(u16),
}

impl fmt::Display for SwigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwigParseError::DataTooShort(ctx) => write!(f, "Instruction data too short: {ctx}"),
            SwigParseError::InvalidFormat(ctx) => write!(f, "Invalid instruction format: {ctx}"),
            SwigParseError::UnknownInstruction(code) => {
                write!(f, "Unknown Swig instruction discriminator {code}")
            }
        }
    }
}

impl std::error::Error for SwigParseError {}

fn parse_swig_instruction(
    data: &[u8],
    accounts: &[AccountMeta],
) -> Result<SwigInstructionDecoded, SwigParseError> {
    if data.len() < 2 {
        return Err(SwigParseError::DataTooShort("missing discriminator"));
    }

    let discriminator = u16::from_le_bytes([data[0], data[1]]);
    let kind = match SwigInstructionKind::try_from(discriminator) {
        Ok(kind) => kind,
        Err(SwigParseError::UnknownInstruction(_)) => {
            return Ok(SwigInstructionDecoded::Unknown {
                discriminator,
                raw_data: data.to_vec(),
            });
        }
        Err(err) => return Err(err),
    };

    match kind {
        SwigInstructionKind::CreateV1 => parse_create_v1(data),
        SwigInstructionKind::AddAuthorityV1 => parse_add_authority_v1(data),
        SwigInstructionKind::RemoveAuthorityV1 => parse_remove_authority_v1(data),
        SwigInstructionKind::UpdateAuthorityV1 => parse_update_authority_v1(data),
        SwigInstructionKind::SignV1 => {
            let sign = parse_sign_instruction(data, 8, accounts)?;
            Ok(SwigInstructionDecoded::SignV1(sign))
        }
        SwigInstructionKind::SignV2 => {
            let sign = parse_sign_instruction(data, 8, accounts)?;
            Ok(SwigInstructionDecoded::SignV2(sign))
        }
        SwigInstructionKind::CreateSessionV1 => parse_create_session_v1(data),
        SwigInstructionKind::CreateSubAccountV1 => parse_create_sub_account_v1(data),
        SwigInstructionKind::WithdrawFromSubAccountV1 => parse_withdraw_from_sub_account(data),
        SwigInstructionKind::SubAccountSignV1 => {
            let sign = parse_sign_instruction(data, 16, accounts)?;
            Ok(SwigInstructionDecoded::SubAccountSignV1(sign))
        }
        SwigInstructionKind::ToggleSubAccountV1 => parse_toggle_sub_account(data),
        SwigInstructionKind::MigrateToWalletAddressV1 => parse_migrate(data),
        SwigInstructionKind::TransferAssetsV1 => parse_transfer_assets(data),
    }
}

fn parse_create_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 40;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("create_v1 header"));
    }

    let authority_type = u16::from_le_bytes([data[2], data[3]]);
    let authority_data_len = u16::from_le_bytes([data[4], data[5]]) as usize;
    let bump = data[6];
    let wallet_address_bump = data[7];
    let mut wallet_id = [0u8; 32];
    wallet_id.copy_from_slice(&data[8..40]);

    if data.len() < HEADER_LEN + authority_data_len {
        return Err(SwigParseError::DataTooShort("create_v1 authority data"));
    }
    let authority_data = data[HEADER_LEN..HEADER_LEN + authority_data_len].to_vec();
    let actions = data[HEADER_LEN + authority_data_len..].to_vec();

    Ok(SwigInstructionDecoded::CreateV1 {
        authority_type,
        bump,
        wallet_address_bump,
        wallet_id,
        authority_data,
        actions,
    })
}

fn parse_add_authority_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("add_authority_v1 header"));
    }

    let authority_data_len = u16::from_le_bytes([data[2], data[3]]) as usize;
    let actions_data_len = u16::from_le_bytes([data[4], data[5]]) as usize;
    let new_authority_type = u16::from_le_bytes([data[6], data[7]]);
    let num_actions = data[8];
    let acting_role_id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    if data.len() < HEADER_LEN + authority_data_len + actions_data_len {
        return Err(SwigParseError::DataTooShort("add_authority_v1 payload"));
    }

    let authority_data = data[HEADER_LEN..HEADER_LEN + authority_data_len].to_vec();
    let actions = data
        [HEADER_LEN + authority_data_len..HEADER_LEN + authority_data_len + actions_data_len]
        .to_vec();
    let authority_payload = data[HEADER_LEN + authority_data_len + actions_data_len..].to_vec();

    Ok(SwigInstructionDecoded::AddAuthorityV1 {
        acting_role_id,
        new_authority_type,
        num_actions,
        authority_data,
        actions,
        authority_payload,
    })
}

fn parse_remove_authority_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("remove_authority_v1 header"));
    }

    let acting_role_id = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let authority_to_remove_id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::RemoveAuthorityV1 {
        acting_role_id,
        authority_to_remove_id,
        authority_payload,
    })
}

fn parse_update_authority_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("update_authority_v1 header"));
    }

    let actions_data_len = u16::from_le_bytes([data[2], data[3]]) as usize;
    let num_actions = data[4];
    let acting_role_id = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let authority_to_update_id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    if data.len() < HEADER_LEN + actions_data_len {
        return Err(SwigParseError::DataTooShort("update_authority_v1 payload"));
    }

    let operation_bytes = data[HEADER_LEN..HEADER_LEN + actions_data_len].to_vec();
    let authority_payload = data[HEADER_LEN + actions_data_len..].to_vec();
    let operation = parse_update_operation(num_actions, &operation_bytes)?;

    Ok(SwigInstructionDecoded::UpdateAuthorityV1 {
        acting_role_id,
        authority_to_update_id,
        operation,
        authority_payload,
    })
}

fn parse_update_operation(
    num_actions: u8,
    operation_bytes: &[u8],
) -> Result<AuthorityUpdateDetails, SwigParseError> {
    if num_actions == 0 {
        if operation_bytes.is_empty() {
            return Err(SwigParseError::InvalidFormat(
                "update_authority missing operation byte",
            ));
        }
        let op = operation_bytes[0];
        let payload = &operation_bytes[1..];
        Ok(match op {
            0 => AuthorityUpdateDetails::ReplaceAll(payload.to_vec()),
            1 => AuthorityUpdateDetails::AddActions(payload.to_vec()),
            2 => AuthorityUpdateDetails::RemoveActionsByType(payload.to_vec()),
            3 => {
                if payload.len() % 2 != 0 {
                    return Err(SwigParseError::InvalidFormat(
                        "remove_actions_by_index payload must be even",
                    ));
                }
                let mut indices = Vec::new();
                for chunk in payload.chunks_exact(2) {
                    indices.push(u16::from_le_bytes([chunk[0], chunk[1]]));
                }
                AuthorityUpdateDetails::RemoveActionsByIndex(indices)
            }
            _ => AuthorityUpdateDetails::Legacy(operation_bytes.to_vec()),
        })
    } else {
        Ok(AuthorityUpdateDetails::Legacy(operation_bytes.to_vec()))
    }
}

fn parse_sign_instruction(
    data: &[u8],
    header_len: usize,
    accounts: &[AccountMeta],
) -> Result<SignInstructionDecoded, SwigParseError> {
    if data.len() < header_len {
        return Err(SwigParseError::DataTooShort("sign header"));
    }

    let instruction_payload_len = u16::from_le_bytes([data[2], data[3]]) as usize;
    let role_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);

    if data.len() < header_len + instruction_payload_len {
        return Err(SwigParseError::DataTooShort("sign instruction payload"));
    }

    let payload_start = header_len;
    let payload_end = payload_start + instruction_payload_len;
    let instruction_payload = &data[payload_start..payload_end];
    let authority_payload = data[payload_end..].to_vec();
    let inner_instructions = decode_compact_instructions(instruction_payload, accounts)?;

    Ok(SignInstructionDecoded {
        role_id,
        instruction_payload_len,
        authority_payload,
        inner_instructions,
    })
}

fn parse_create_session_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 48;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("create_session header"));
    }

    let role_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let session_duration = u64::from_le_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);
    let mut session_key = [0u8; 32];
    session_key.copy_from_slice(&data[16..48]);
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::CreateSessionV1 {
        role_id,
        session_duration,
        session_key,
        authority_payload,
    })
}

fn parse_create_sub_account_v1(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("create_sub_account header"));
    }

    let role_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let sub_account_bump = data[8];
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::CreateSubAccountV1 {
        role_id,
        sub_account_bump,
        authority_payload,
    })
}

fn parse_withdraw_from_sub_account(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("withdraw_sub_account header"));
    }

    let role_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let amount = u64::from_le_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::WithdrawFromSubAccountV1 {
        role_id,
        amount,
        authority_payload,
    })
}

fn parse_toggle_sub_account(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 16;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("toggle_sub_account header"));
    }

    let enabled = data[2] != 0;
    let target_role_id = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let authority_role_id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::ToggleSubAccountV1 {
        target_role_id,
        authority_role_id,
        enabled,
        authority_payload,
    })
}

fn parse_migrate(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 8;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("migrate header"));
    }

    let wallet_address_bump = data[2];
    Ok(SwigInstructionDecoded::MigrateToWalletAddressV1 {
        wallet_address_bump,
    })
}

fn parse_transfer_assets(data: &[u8]) -> Result<SwigInstructionDecoded, SwigParseError> {
    const HEADER_LEN: usize = 8;
    if data.len() < HEADER_LEN {
        return Err(SwigParseError::DataTooShort("transfer_assets header"));
    }

    let role_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let authority_payload = data[HEADER_LEN..].to_vec();

    Ok(SwigInstructionDecoded::TransferAssetsV1 {
        role_id,
        authority_payload,
    })
}

fn decode_compact_instructions(
    payload: &[u8],
    accounts: &[AccountMeta],
) -> Result<Vec<DecodedInnerInstruction>, SwigParseError> {
    if payload.is_empty() {
        return Ok(Vec::new());
    }

    let mut cursor = 0usize;
    let total = payload[cursor] as usize;
    cursor += 1;

    let mut instructions = Vec::with_capacity(total);
    for _ in 0..total {
        if cursor >= payload.len() {
            return Err(SwigParseError::InvalidFormat(
                "compact instruction truncated",
            ));
        }
        let program_index = payload[cursor];
        cursor += 1;
        let program_meta = resolve_account_meta(accounts, program_index as usize);

        if cursor >= payload.len() {
            return Err(SwigParseError::InvalidFormat("missing account count"));
        }
        let account_count = payload[cursor] as usize;
        cursor += 1;

        if cursor + account_count > payload.len() {
            return Err(SwigParseError::InvalidFormat(
                "account list exceeds payload",
            ));
        }

        let mut inner_accounts = Vec::with_capacity(account_count);
        for account_byte in &payload[cursor..cursor + account_count] {
            let idx = *account_byte as usize;
            let meta = resolve_account_meta(accounts, idx);
            inner_accounts.push(InnerAccountMeta {
                pubkey: meta.pubkey,
                display: meta.display,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            });
        }
        cursor += account_count;

        if cursor + 2 > payload.len() {
            return Err(SwigParseError::InvalidFormat("missing data length"));
        }
        let data_len = u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + data_len > payload.len() {
            return Err(SwigParseError::InvalidFormat("data slice out of range"));
        }
        let data_slice = payload[cursor..cursor + data_len].to_vec();
        cursor += data_len;

        let description = describe_inner_instruction(
            program_meta.pubkey.as_ref(),
            &program_meta.display,
            &data_slice,
            &inner_accounts,
        );
        instructions.push(DecodedInnerInstruction {
            program_id: program_meta.pubkey,
            program_display: program_meta.display,
            accounts: inner_accounts,
            data: data_slice,
            description,
        });
    }

    Ok(instructions)
}

struct ResolvedAccountMeta {
    pub pubkey: Option<Pubkey>,
    pub display: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

fn resolve_account_meta(accounts: &[AccountMeta], index: usize) -> ResolvedAccountMeta {
    if let Some(meta) = accounts.get(index) {
        ResolvedAccountMeta {
            pubkey: Some(meta.pubkey),
            display: meta.pubkey.to_string(),
            is_signer: meta.is_signer,
            is_writable: meta.is_writable,
        }
    } else {
        ResolvedAccountMeta {
            pubkey: None,
            display: format!("Unresolved account #{index} (lookup table)"),
            is_signer: false,
            is_writable: false,
        }
    }
}

fn describe_inner_instruction(
    program_id: Option<&Pubkey>,
    program_display: &str,
    data: &[u8],
    accounts: &[InnerAccountMeta],
) -> String {
    let fallback = || {
        let byte_len = data.len();
        format!("Program {program_display} ({byte_len} bytes)")
    };

    let Some(program_id) = program_id else {
        return fallback();
    };

    if let Some(instruction) = build_inner_instruction(program_id, accounts, data) {
        if let Some(summary) = visualize_inner_instruction(instruction) {
            return summary;
        }
    }

    if program_id == &spl_token::ID {
        if let Ok(ix) = TokenInstruction::unpack(data) {
            if let Some(summary) = format_token_instruction_summary(ix, accounts) {
                return summary;
            }
        }
    } else if program_id == &system_program::ID {
        if let Ok(SystemInstruction::Transfer { lamports }) =
            bincode::deserialize::<SystemInstruction>(data)
        {
            return format_native_sol_transfer(accounts, lamports);
        }
    }

    fallback()
}

fn build_inner_instruction(
    program_id: &Pubkey,
    accounts: &[InnerAccountMeta],
    data: &[u8],
) -> Option<Instruction> {
    let metas: Option<Vec<AccountMeta>> = accounts
        .iter()
        .map(|meta| {
            let pubkey = meta.pubkey?;
            let account_meta = if meta.is_writable {
                AccountMeta::new(pubkey, meta.is_signer)
            } else {
                AccountMeta::new_readonly(pubkey, meta.is_signer)
            };
            Some(account_meta)
        })
        .collect();

    metas.map(|accounts| Instruction {
        program_id: *program_id,
        accounts,
        data: data.to_vec(),
    })
}

fn visualize_inner_instruction(instruction: Instruction) -> Option<String> {
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizer_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|viz| viz.as_ref()).collect();

    let sender = SolanaAccount {
        account_key: instruction
            .accounts
            .first()
            .map(|meta| meta.pubkey.to_string())
            .unwrap_or_else(|| instruction.program_id.to_string()),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    visualize_with_any(&visualizer_refs, &context)
        .and_then(|result| result.ok())
        .and_then(|viz_result| match viz_result.kind {
            VisualizerKind::Payments("UnknownProgram") => None,
            _ => summarize_visualized_field(&viz_result.field),
        })
}

fn summarize_visualized_field(field: &AnnotatedPayloadField) -> Option<String> {
    use SignablePayloadField::*;

    match &field.signable_payload_field {
        PreviewLayout {
            common,
            preview_layout,
        } => preview_layout
            .title
            .as_ref()
            .map(|title| title.text.clone())
            .filter(|text| !text.is_empty())
            .or_else(|| fallback_summary(common)),
        Text { common, text } => {
            if text.text.is_empty() {
                fallback_summary(common)
            } else {
                Some(text.text.clone())
            }
        }
        TextV2 { common, text_v2 } => {
            if text_v2.text.is_empty() {
                fallback_summary(common)
            } else {
                Some(text_v2.text.clone())
            }
        }
        Number { common, number } => {
            if number.number.is_empty() {
                fallback_summary(common)
            } else {
                Some(number.number.clone())
            }
        }
        Amount { common, amount } => {
            if amount.amount.is_empty() {
                fallback_summary(common)
            } else {
                Some(amount.amount.clone())
            }
        }
        AmountV2 { common, amount_v2 } => {
            if amount_v2.amount.is_empty() {
                fallback_summary(common)
            } else {
                Some(amount_v2.amount.clone())
            }
        }
        Address { common, address } => {
            if address.address.is_empty() {
                fallback_summary(common)
            } else {
                Some(address.address.clone())
            }
        }
        AddressV2 { common, address_v2 } => {
            if address_v2.address.is_empty() {
                fallback_summary(common)
            } else {
                Some(address_v2.address.clone())
            }
        }
        ListLayout { common, .. } | Divider { common, .. } | Unknown { common, .. } => {
            fallback_summary(common)
        }
    }
}

fn fallback_summary(common: &SignablePayloadFieldCommon) -> Option<String> {
    if common.fallback_text.is_empty() {
        None
    } else {
        Some(common.fallback_text.clone())
    }
}

fn format_token_instruction_summary(
    ix: TokenInstruction,
    accounts: &[InnerAccountMeta],
) -> Option<String> {
    let account_label = |idx: usize, fallback: &str| {
        if idx == 0 {
            accounts
                .first()
                .map(|meta| meta.display.clone())
                .unwrap_or_else(|| fallback.to_string())
        } else {
            accounts
                .get(idx)
                .map(|meta| meta.display.clone())
                .unwrap_or_else(|| fallback.to_string())
        }
    };

    match ix {
        TokenInstruction::Transfer { amount } => {
            let from = account_label(0, "Source");
            let to = account_label(1, "Destination");
            let owner = account_label(2, "Owner");
            Some(format!(
                "From: {from}\nTo: {to}\nOwner: {owner}\nAmount: {amount}"
            ))
        }
        TokenInstruction::TransferChecked { amount, decimals } => {
            let from = account_label(0, "Source");
            let mint = account_label(1, "Mint");
            let to = account_label(2, "Destination");
            let owner = account_label(3, "Owner");
            Some(format!(
                "From: {from}\nTo: {to}\nOwner: {owner}\nAmount: {amount}\nMint: {mint}\nDecimals: {decimals}"
            ))
        }
        TokenInstruction::MintTo { amount } => {
            let mint = account_label(0, "Mint");
            let destination = account_label(1, "Destination");
            let authority = account_label(2, "Authority");
            Some(format!(
                "Mint: {mint}\nDestination: {destination}\nAuthority: {authority}\nAmount: {amount}"
            ))
        }
        TokenInstruction::MintToChecked { amount, decimals } => {
            let destination = account_label(1, "Destination");
            let mint = account_label(0, "Mint");
            let authority = account_label(2, "Authority");
            Some(format!(
                "Mint: {mint}\nDestination: {destination}\nAuthority: {authority}\nAmount: {amount}\nDecimals: {decimals}"
            ))
        }
        TokenInstruction::Burn { amount } => {
            let source = account_label(0, "Source");
            let mint = account_label(1, "Mint");
            let authority = account_label(2, "Authority");
            Some(format!(
                "Source: {source}\nMint: {mint}\nAuthority: {authority}\nAmount: {amount}"
            ))
        }
        TokenInstruction::BurnChecked { amount, decimals } => {
            let source = account_label(0, "Source");
            let mint = account_label(1, "Mint");
            let authority = account_label(2, "Authority");
            Some(format!(
                "Source: {source}\nMint: {mint}\nAuthority: {authority}\nAmount: {amount}\nDecimals: {decimals}"
            ))
        }
        TokenInstruction::Approve { amount } => {
            let owner = account_label(0, "Owner");
            let delegate = account_label(1, "Delegate");
            Some(format!(
                "Owner: {owner}\nDelegate: {delegate}\nAmount: {amount}"
            ))
        }
        TokenInstruction::ApproveChecked { amount, decimals } => Some(format!(
            "SPL Token: approve checked for {amount} ({decimals} decimals)"
        )),
        TokenInstruction::SetAuthority {
            authority_type,
            new_authority,
        } => {
            let authority_type = match authority_type {
                SplAuthorityType::AccountOwner => "AccountOwner",
                SplAuthorityType::CloseAccount => "CloseAccount",
                SplAuthorityType::FreezeAccount => "FreezeAccount",
                SplAuthorityType::MintTokens => "MintTokens",
            };
            let target = new_authority
                .map(|pk| pk.to_string())
                .unwrap_or_else(|| "None".to_string());
            let account = account_label(0, "Account");
            Some(format!(
                "Account: {account}\nAuthority Type: {authority_type}\nNew Authority: {target}"
            ))
        }
        TokenInstruction::CloseAccount => {
            let account = account_label(0, "Account");
            let destination = account_label(1, "Destination");
            let authority = account_label(2, "Authority");
            Some(format!(
                "Account: {account}\nDestination: {destination}\nAuthority: {authority}"
            ))
        }
        TokenInstruction::SyncNative => {
            let account = account_label(0, "Account");
            Some(format!("Account: {account}\nAction: Sync Native"))
        }
        _ => None,
    }
}

fn format_native_sol_transfer(accounts: &[InnerAccountMeta], lamports: u64) -> String {
    let from = accounts
        .first()
        .map(|meta| meta.display.clone())
        .unwrap_or_else(|| "Source".to_string());
    let to = accounts
        .get(1)
        .map(|meta| meta.display.clone())
        .unwrap_or_else(|| "Destination".to_string());
    format!("From: {from}\nTo: {to}\nAmount: {lamports}")
}

fn make_text_field(
    label: &str,
    value: impl Into<String>,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let text = value.into();
    create_text_field(label, &text)
}

fn build_variant_fields(
    decoded: &SwigInstructionDecoded,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    match decoded {
        SwigInstructionDecoded::CreateV1 {
            authority_type,
            bump,
            wallet_address_bump,
            wallet_id,
            authority_data,
            actions,
        } => {
            let mut fields = vec![
                make_text_field("Authority Type", authority_type_name(*authority_type))?,
                make_text_field("Wallet PDA Bump", bump.to_string())?,
                make_text_field("Wallet Address Bump", wallet_address_bump.to_string())?,
                make_text_field("Wallet ID (hex)", hex::encode(wallet_id))?,
                make_text_field(
                    "Authority Data Length",
                    format_byte_length(authority_data.len()),
                )?,
            ];
            if let Some(authority_details) =
                decode_authority_details(*authority_type, authority_data)
            {
                for (label, value) in authority_details {
                    fields.push(make_text_field(&label, value)?);
                }
            }
            if !authority_data.is_empty() {
                fields.push(make_text_field(
                    "Authority Data (hex)",
                    format_hex(authority_data),
                )?);
            }
            fields.push(make_text_field(
                "Actions Summary",
                format_actions_summary(actions),
            )?);
            for (label, value) in action_detail_rows("Actions", actions) {
                fields.push(make_text_field(&label, value)?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::AddAuthorityV1 {
            acting_role_id,
            new_authority_type,
            num_actions,
            authority_data,
            actions,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Acting Role", acting_role_id.to_string())?,
                make_text_field(
                    "New Authority Type",
                    authority_type_name(*new_authority_type),
                )?,
                make_text_field("Declared Action Count", num_actions.to_string())?,
                make_text_field(
                    "Authority Data Length",
                    format_byte_length(authority_data.len()),
                )?,
            ];
            if let Some(authority_details) =
                decode_authority_details(*new_authority_type, authority_data)
            {
                for (label, value) in authority_details {
                    fields.push(make_text_field(&label, value)?);
                }
            }
            if !authority_data.is_empty() {
                fields.push(make_text_field(
                    "Authority Data (hex)",
                    format_hex(authority_data),
                )?);
            }
            fields.push(make_text_field(
                "Actions Summary",
                format_actions_summary(actions),
            )?);
            for (label, value) in action_detail_rows("Actions", actions) {
                fields.push(make_text_field(&label, value)?);
            }
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::RemoveAuthorityV1 {
            acting_role_id,
            authority_to_remove_id,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Acting Role", acting_role_id.to_string())?,
                make_text_field("Authority To Remove", authority_to_remove_id.to_string())?,
            ];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::UpdateAuthorityV1 {
            acting_role_id,
            authority_to_update_id,
            operation,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Acting Role", acting_role_id.to_string())?,
                make_text_field("Authority To Update", authority_to_update_id.to_string())?,
            ];
            for (label, value) in authority_update_rows(operation) {
                fields.push(make_text_field(&label, value)?);
            }
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::SignV1(sign)
        | SwigInstructionDecoded::SignV2(sign)
        | SwigInstructionDecoded::SubAccountSignV1(sign) => build_sign_fields(sign),
        SwigInstructionDecoded::CreateSessionV1 {
            role_id,
            session_duration,
            session_key,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Role ID", role_id.to_string())?,
                make_text_field("Session Duration", format!("{session_duration} slots"))?,
                make_text_field("Session Key (hex)", hex::encode(session_key))?,
            ];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::CreateSubAccountV1 {
            role_id,
            sub_account_bump,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Role ID", role_id.to_string())?,
                make_text_field("Sub-Account Bump", sub_account_bump.to_string())?,
            ];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::WithdrawFromSubAccountV1 {
            role_id,
            amount,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Role ID", role_id.to_string())?,
                make_text_field("Amount", format_lamports_with_sol(*amount))?,
            ];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::ToggleSubAccountV1 {
            target_role_id,
            authority_role_id,
            enabled,
            authority_payload,
        } => {
            let mut fields = vec![
                make_text_field("Target Role", target_role_id.to_string())?,
                make_text_field("Authority Role", authority_role_id.to_string())?,
                make_text_field("Enabled", enabled.to_string())?,
            ];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::MigrateToWalletAddressV1 {
            wallet_address_bump,
        } => Ok(vec![make_text_field(
            "Wallet Address Bump",
            wallet_address_bump.to_string(),
        )?]),
        SwigInstructionDecoded::TransferAssetsV1 {
            role_id,
            authority_payload,
        } => {
            let mut fields = vec![make_text_field("Role ID", role_id.to_string())?];
            if !authority_payload.is_empty() {
                fields.push(make_text_field(
                    "Authority Payload (hex)",
                    format_hex(authority_payload),
                )?);
            }
            Ok(fields)
        }
        SwigInstructionDecoded::Unknown {
            discriminator,
            raw_data,
        } => Ok(vec![
            make_text_field("Discriminator", discriminator.to_string())?,
            make_text_field("Raw Data (hex)", format_hex(raw_data))?,
        ]),
    }
}

fn build_sign_fields(
    sign: &SignInstructionDecoded,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        make_text_field("Role ID", sign.role_id.to_string())?,
        make_text_field(
            "Instruction Payload Length",
            format_byte_length(sign.instruction_payload_len),
        )?,
        make_text_field(
            "Inner Instruction Count",
            sign.inner_instructions.len().to_string(),
        )?,
    ];

    if !sign.authority_payload.is_empty() {
        fields.push(make_text_field(
            "Authority Payload (hex)",
            format_hex(&sign.authority_payload),
        )?);
        if let Ok(mut decoded_fields) = decode_authority_payload(&sign.authority_payload) {
            fields.append(&mut decoded_fields);
        }
    }

    for (idx, inner) in sign.inner_instructions.iter().enumerate() {
        fields.push(make_text_field(
            &format!("Inner Instruction {} Summary", idx + 1),
            inner.description.clone(),
        )?);
        fields.push(make_text_field(
            &format!("Inner Instruction {} Program", idx + 1),
            inner.program_display.clone(),
        )?);
        if inner.program_id.is_none() {
            fields.push(make_text_field(
                &format!("Inner Instruction {} Program Resolution", idx + 1),
                "Program account supplied via address lookup table; full details require on-chain lookup"
                    .to_string(),
            )?);
        }
        fields.push(make_text_field(
            &format!("Inner Instruction {} Accounts", idx + 1),
            format_inner_accounts(&inner.accounts),
        )?);
        let unresolved_accounts: Vec<String> = inner
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, account)| account.pubkey.is_none())
            .map(|(i, account)| format!("{}: {}", i + 1, account.display))
            .collect();
        if !unresolved_accounts.is_empty() {
            fields.push(make_text_field(
                &format!("Inner Instruction {} Account Resolution", idx + 1),
                format!(
                    "Unresolved lookup-table accounts:\n{}",
                    unresolved_accounts.join("\n")
                ),
            )?);
        }
        if !inner.data.is_empty() {
            fields.push(make_text_field(
                &format!("Inner Instruction {} Data (hex)", idx + 1),
                format_hex(&inner.data),
            )?);
        }
    }

    Ok(fields)
}

fn format_inner_accounts(accounts: &[InnerAccountMeta]) -> String {
    if accounts.is_empty() {
        return "(none)".to_string();
    }

    accounts
        .iter()
        .enumerate()
        .map(|(idx, meta)| {
            let mut flags = Vec::new();
            if meta.is_signer {
                flags.push("signer");
            }
            if meta.is_writable {
                flags.push("writable");
            }
            let flag_str = if flags.is_empty() {
                String::new()
            } else {
                format!(" ({})", flags.join(", "))
            };
            format!("{}: {}{}", idx + 1, meta.display, flag_str)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn authority_type_name(value: u16) -> &'static str {
    match value {
        1 => "Ed25519",
        2 => "Ed25519 Session",
        3 => "Secp256k1",
        4 => "Secp256k1 Session",
        5 => "Secp256r1",
        6 => "Secp256r1 Session",
        _ => "Unknown",
    }
}

fn permission_name(value: u16) -> &'static str {
    match value {
        1 => "SolLimit",
        2 => "SolRecurringLimit",
        3 => "Program",
        4 => "ProgramScope",
        5 => "TokenLimit",
        6 => "TokenRecurringLimit",
        7 => "All",
        8 => "ManageAuthority",
        9 => "SubAccount",
        10 => "TokenDestinationLimit",
        11 => "TokenRecurringDestinationLimit",
        12 => "SolDestinationLimit",
        13 => "SolRecurringDestinationLimit",
        14 => "StakeLimit",
        15 => "StakeRecurringLimit",
        16 => "StakeAll",
        _ => "Unknown",
    }
}

fn count_actions(bytes: &[u8]) -> usize {
    const HEADER_LEN: usize = 8;
    let mut cursor = 0usize;
    let mut count = 0usize;

    while cursor + HEADER_LEN <= bytes.len() {
        let length = u16::from_le_bytes([bytes[cursor + 2], bytes[cursor + 3]]) as usize;
        let total = HEADER_LEN + length;
        if cursor + total > bytes.len() {
            break;
        }
        cursor += total;
        count += 1;
    }

    count
}

fn format_byte_length(len: usize) -> String {
    format!("{len} bytes")
}

fn format_actions_summary(bytes: &[u8]) -> String {
    let count = count_actions(bytes);
    let len = bytes.len();
    format!("{len} bytes (~{count} action(s))")
}

fn decode_authority_details(authority_type: u16, data: &[u8]) -> Option<Vec<(String, String)>> {
    match authority_type {
        1 => {
            let array: [u8; 32] = data.try_into().ok()?;
            let pubkey = Pubkey::new_from_array(array);
            Some(vec![(
                "Authority Public Key".to_string(),
                pubkey.to_string(),
            )])
        }
        2 => {
            if data.len() != 72 {
                return None;
            }
            let root_bytes: [u8; 32] = data[0..32].try_into().ok()?;
            let session_bytes: [u8; 32] = data[32..64].try_into().ok()?;
            let max_session_length = u64::from_le_bytes(data[64..72].try_into().ok()?);
            let root = Pubkey::new_from_array(root_bytes);
            let session = Pubkey::new_from_array(session_bytes);
            Some(vec![
                ("Root Authority Public Key".to_string(), root.to_string()),
                ("Initial Session Key".to_string(), session.to_string()),
                (
                    "Max Session Length (slots)".to_string(),
                    max_session_length.to_string(),
                ),
            ])
        }
        3 => {
            if !(data.len() == 33 || data.len() == 64 || data.len() == 65) {
                return None;
            }
            let mut fields = Vec::new();
            match data.len() {
                33 => {
                    fields.push((
                        "Secp256k1 Public Key (compressed hex)".to_string(),
                        hex::encode(data),
                    ));
                }
                64 => {
                    fields.push((
                        "Secp256k1 Public Key (uncompressed hex)".to_string(),
                        hex::encode(data),
                    ));
                    if let Some(address) = derive_eth_address_from_uncompressed(data) {
                        fields.push(("Derived EVM Address".to_string(), address));
                    }
                }
                65 => {
                    if data[0] != 0x04 {
                        return None;
                    }
                    fields.push((
                        "Secp256k1 Public Key (uncompressed hex)".to_string(),
                        hex::encode(&data[1..]),
                    ));
                    if let Some(address) = derive_eth_address_from_uncompressed(&data[1..]) {
                        fields.push(("Derived EVM Address".to_string(), address));
                    }
                }
                _ => {}
            }
            Some(fields)
        }
        4 => {
            if data.len() != 104 {
                return None;
            }
            let public_key = &data[0..64];
            let session_key = &data[64..96];
            let max_session_length = u64::from_le_bytes(data[96..104].try_into().ok()?);
            let mut fields = vec![(
                "Secp256k1 Public Key (uncompressed hex)".to_string(),
                hex::encode(public_key),
            )];
            if let Some(address) = derive_eth_address_from_uncompressed(public_key) {
                fields.push(("Derived EVM Address".to_string(), address));
            }
            fields.extend([
                ("Session Key (hex)".to_string(), hex::encode(session_key)),
                (
                    "Max Session Length (slots)".to_string(),
                    max_session_length.to_string(),
                ),
            ]);
            Some(fields)
        }
        5 => {
            if data.len() != 33 {
                return None;
            }
            Some(vec![(
                "Secp256r1 Public Key (compressed hex)".to_string(),
                hex::encode(data),
            )])
        }
        6 => {
            if data.len() != 80 {
                return None;
            }
            let public_key = &data[0..33];
            let session_key = &data[40..72];
            let max_session_length = u64::from_le_bytes(data[72..80].try_into().ok()?);
            Some(vec![
                (
                    "Secp256r1 Public Key (compressed hex)".to_string(),
                    hex::encode(public_key),
                ),
                ("Session Key (hex)".to_string(), hex::encode(session_key)),
                (
                    "Max Session Length (slots)".to_string(),
                    max_session_length.to_string(),
                ),
            ])
        }
        _ => None,
    }
}

fn decode_actions(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    let mut cursor = 0usize;
    let mut results = Vec::new();
    let mut index = 1usize;
    while cursor + 8 <= bytes.len() {
        let start = cursor;
        let permission = u16::from_le_bytes(bytes[start..start + 2].try_into().ok()?);
        let length = u16::from_le_bytes(bytes[start + 2..start + 4].try_into().ok()?) as usize;
        let boundary = u32::from_le_bytes(bytes[start + 4..start + 8].try_into().ok()?) as usize;
        let data_start = start + 8;
        let data_end = data_start.checked_add(length)?;
        if data_end > bytes.len() {
            return None;
        }
        let description = describe_action(permission, &bytes[data_start..data_end])?;
        results.push(format!("Action {index}: {description}"));
        cursor = boundary;
        if cursor > bytes.len() || cursor < data_end {
            return None;
        }
        index += 1;
    }
    if cursor != bytes.len() {
        return None;
    }
    Some(results)
}

fn action_detail_rows(label: &str, bytes: &[u8]) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(action_lines) = decode_actions(bytes) {
        let text = if action_lines.is_empty() {
            "(none)".to_string()
        } else {
            action_lines.join("\n")
        };
        rows.push((label.to_string(), text));
    } else {
        rows.push((
            label.to_string(),
            format!("{} (unable to decode)", format_actions_summary(bytes)),
        ));
    }
    if !bytes.is_empty() {
        rows.push((format!("{label} (hex)"), format_hex(bytes)));
    }
    rows
}

fn authority_update_rows(details: &AuthorityUpdateDetails) -> Vec<(String, String)> {
    match details {
        AuthorityUpdateDetails::ReplaceAll(bytes) => {
            let mut rows = vec![
                ("Operation Type".to_string(), "Replace actions".to_string()),
                ("Actions Summary".to_string(), format_actions_summary(bytes)),
            ];
            rows.extend(action_detail_rows("Updated Actions", bytes));
            rows
        }
        AuthorityUpdateDetails::AddActions(bytes) => {
            let mut rows = vec![
                ("Operation Type".to_string(), "Add actions".to_string()),
                ("Actions Summary".to_string(), format_actions_summary(bytes)),
            ];
            rows.extend(action_detail_rows("Actions To Add", bytes));
            rows
        }
        AuthorityUpdateDetails::RemoveActionsByType(types) => {
            let names = if types.is_empty() {
                "(none)".to_string()
            } else {
                types
                    .iter()
                    .map(|value| permission_name(*value as u16))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut rows = vec![
                (
                    "Operation Type".to_string(),
                    "Remove actions by type".to_string(),
                ),
                ("Action Types".to_string(), names),
            ];
            if !types.is_empty() {
                rows.push(("Action Type Bytes (hex)".to_string(), format_hex(types)));
            }
            rows
        }
        AuthorityUpdateDetails::RemoveActionsByIndex(indices) => {
            let list = if indices.is_empty() {
                "(none)".to_string()
            } else {
                indices
                    .iter()
                    .map(|idx| idx.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut rows = vec![
                (
                    "Operation Type".to_string(),
                    "Remove actions by index".to_string(),
                ),
                ("Action Indices".to_string(), list),
            ];
            if !indices.is_empty() {
                let mut raw = Vec::with_capacity(indices.len() * 2);
                for idx in indices {
                    raw.extend_from_slice(&idx.to_le_bytes());
                }
                rows.push(("Action Indices (hex)".to_string(), format_hex(&raw)));
            }
            rows
        }
        AuthorityUpdateDetails::Legacy(bytes) => {
            let mut rows = vec![
                ("Operation Type".to_string(), "Legacy payload".to_string()),
                ("Actions Summary".to_string(), format_actions_summary(bytes)),
            ];
            rows.extend(action_detail_rows("Legacy Actions", bytes));
            rows
        }
    }
}

fn describe_action(permission: u16, data: &[u8]) -> Option<String> {
    match permission {
        1 => {
            if data.len() != 8 {
                return None;
            }
            let amount = u64::from_le_bytes(data.try_into().ok()?);
            Some(format!("SOL limit: {}", format_lamports_with_sol(amount)))
        }
        2 => {
            if data.len() != 32 {
                return None;
            }
            let amount = u64::from_le_bytes(data[0..8].try_into().ok()?);
            let window = u64::from_le_bytes(data[8..16].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[16..24].try_into().ok()?);
            let current = u64::from_le_bytes(data[24..32].try_into().ok()?);
            Some(format!(
                "SOL recurring limit: {} per {} slot(s); current {} (last reset {})",
                format_lamports_with_sol(amount),
                window,
                format_lamports_with_sol(current),
                last_reset
            ))
        }
        3 => {
            if data.len() != 32 {
                return None;
            }
            let program = Pubkey::new_from_array(data.try_into().ok()?);
            Some(format!("Program access: {program}"))
        }
        4 => {
            if data.len() < 144 {
                return None;
            }
            let current_amount = u128::from_le_bytes(data[0..16].try_into().ok()?);
            let limit = u128::from_le_bytes(data[16..32].try_into().ok()?);
            let window = u64::from_le_bytes(data[32..40].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[40..48].try_into().ok()?);
            let program = Pubkey::new_from_array(data[48..80].try_into().ok()?);
            let target = Pubkey::new_from_array(data[80..112].try_into().ok()?);
            let scope_type = u64::from_le_bytes(data[112..120].try_into().ok()?);
            let numeric_type = u64::from_le_bytes(data[120..128].try_into().ok()?);
            let balance_start = u64::from_le_bytes(data[128..136].try_into().ok()?);
            let balance_end = u64::from_le_bytes(data[136..144].try_into().ok()?);
            Some(format!(
                "Program scope ({}) for program {} target {}; limit {} (current {}), window {} slot(s), last reset {}, numeric {}, balance field bytes {}..{}",
                program_scope_type_name(scope_type),
                program,
                target,
                limit,
                current_amount,
                window,
                last_reset,
                numeric_type_name(numeric_type),
                balance_start,
                balance_end
            ))
        }
        5 => {
            if data.len() != 40 {
                return None;
            }
            let mint = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let amount = u64::from_le_bytes(data[32..40].try_into().ok()?);
            Some(format!("Token limit: mint {mint} remaining {amount}"))
        }
        6 => {
            if data.len() != 64 {
                return None;
            }
            let mint = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let window = u64::from_le_bytes(data[32..40].try_into().ok()?);
            let limit = u64::from_le_bytes(data[40..48].try_into().ok()?);
            let current = u64::from_le_bytes(data[48..56].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[56..64].try_into().ok()?);
            Some(format!(
                "Token recurring limit: mint {mint} limit {limit} per {window} slot(s); current {current} (last reset {last_reset})"
            ))
        }
        7 => Some("All permissions (full access)".to_string()),
        8 => Some("Manage authority (add/remove/update authorities)".to_string()),
        9 => {
            if data.len() != 72 {
                return None;
            }
            let target_bytes: [u8; 32] = data[0..32].try_into().ok()?;
            let bump = data[32];
            let enabled = data[33] != 0;
            let role_id = u32::from_le_bytes(data[36..40].try_into().ok()?);
            let swig_id = Pubkey::new_from_array(data[40..72].try_into().ok()?);
            let target = if target_bytes.iter().all(|b| *b == 0) {
                "Uninitialized (assigned on creation)".to_string()
            } else {
                Pubkey::new_from_array(target_bytes).to_string()
            };
            Some(format!(
                "Sub-account permission: target {target}, role {role_id}, bump {bump}, enabled {enabled}, swig id {swig_id}"
            ))
        }
        10 => {
            if data.len() != 72 {
                return None;
            }
            let mint = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let destination = Pubkey::new_from_array(data[32..64].try_into().ok()?);
            let amount = u64::from_le_bytes(data[64..72].try_into().ok()?);
            Some(format!(
                "Token destination limit: mint {mint} destination {destination} remaining {amount}"
            ))
        }
        11 => {
            if data.len() != 96 {
                return None;
            }
            let mint = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let destination = Pubkey::new_from_array(data[32..64].try_into().ok()?);
            let recurring_amount = u64::from_le_bytes(data[64..72].try_into().ok()?);
            let window = u64::from_le_bytes(data[72..80].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[80..88].try_into().ok()?);
            let current = u64::from_le_bytes(data[88..96].try_into().ok()?);
            Some(format!(
                "Token recurring destination limit: mint {mint} destination {destination} limit {recurring_amount} per {window} slot(s); current {current} (last reset {last_reset})"
            ))
        }
        12 => {
            if data.len() != 40 {
                return None;
            }
            let destination = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let amount = u64::from_le_bytes(data[32..40].try_into().ok()?);
            Some(format!(
                "SOL destination limit: destination {} remaining {}",
                destination,
                format_lamports_with_sol(amount)
            ))
        }
        13 => {
            if data.len() != 64 {
                return None;
            }
            let destination = Pubkey::new_from_array(data[0..32].try_into().ok()?);
            let recurring_amount = u64::from_le_bytes(data[32..40].try_into().ok()?);
            let window = u64::from_le_bytes(data[40..48].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[48..56].try_into().ok()?);
            let current = u64::from_le_bytes(data[56..64].try_into().ok()?);
            Some(format!(
                "SOL recurring destination limit: destination {} limit {} per {} slot(s); current {} (last reset {})",
                destination,
                format_lamports_with_sol(recurring_amount),
                window,
                format_lamports_with_sol(current),
                last_reset
            ))
        }
        14 => {
            if data.len() != 8 {
                return None;
            }
            let amount = u64::from_le_bytes(data.try_into().ok()?);
            Some(format!("Stake limit: {}", format_lamports_with_sol(amount)))
        }
        15 => {
            if data.len() != 32 {
                return None;
            }
            let amount = u64::from_le_bytes(data[0..8].try_into().ok()?);
            let window = u64::from_le_bytes(data[8..16].try_into().ok()?);
            let last_reset = u64::from_le_bytes(data[16..24].try_into().ok()?);
            let current = u64::from_le_bytes(data[24..32].try_into().ok()?);
            Some(format!(
                "Stake recurring limit: {} per {} slot(s); current {} (last reset {})",
                format_lamports_with_sol(amount),
                window,
                format_lamports_with_sol(current),
                last_reset
            ))
        }
        16 => Some("Stake all permissions (unrestricted staking)".to_string()),
        17 => Some("Program access: all programs (unrestricted CPI)".to_string()),
        18 => Some("Program access: curated set of well-known programs".to_string()),
        19 => Some("All operations except authority management".to_string()),
        _ => {
            let len = data.len();
            Some(format!("Unknown permission {permission} ({len} bytes)"))
        }
    }
}

fn derive_eth_address_from_uncompressed(uncompressed: &[u8]) -> Option<String> {
    if uncompressed.len() != 64 {
        return None;
    }
    let hash = solana_sdk::keccak::hash(uncompressed);
    let address = hex::encode(&hash.0[12..32]);
    Some(format!("0x{address}"))
}

fn format_lamports_with_sol(lamports: u64) -> String {
    let mut sol = format!("{:.9}", lamports_as_sol(lamports));
    let has_decimal = sol.contains('.');
    if has_decimal {
        while sol.ends_with('0') {
            sol.pop();
        }
        if sol.ends_with('.') {
            sol.pop();
        }
    }
    format!("{lamports} lamports (~{sol} SOL)")
}

fn program_scope_type_name(value: u64) -> &'static str {
    match value {
        0 => "basic",
        1 => "fixed limit",
        2 => "recurring limit",
        _ => "unknown",
    }
}

fn numeric_type_name(value: u64) -> &'static str {
    match value {
        0 => "u8",
        1 => "u32",
        2 => "u64",
        3 => "u128",
        _ => "unknown",
    }
}

fn format_hex(data: &[u8]) -> String {
    hex::encode(data)
}

fn lamports_as_sol(amount: u64) -> f64 {
    amount as f64 / LAMPORTS_PER_SOL as f64
}

fn decode_authority_payload(payload: &[u8]) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    if payload.len() < 13 {
        return Err(VisualSignError::DecodeError(
            "Authority payload too short".to_string(),
        ));
    }

    let mut slot_bytes = [0u8; 8];
    slot_bytes.copy_from_slice(&payload[..8]);
    let slot = u64::from_le_bytes(slot_bytes);

    let mut counter_bytes = [0u8; 4];
    counter_bytes.copy_from_slice(&payload[8..12]);
    let counter = u32::from_le_bytes(counter_bytes);

    let instruction_account_index = payload[12];

    let reserved = if payload.len() >= 17 {
        &payload[13..17]
    } else {
        &[]
    };

    let mut fields = vec![
        make_text_field("Authority Slot", slot.to_string())?,
        make_text_field("Authority Counter", counter.to_string())?,
        make_text_field(
            "Authority Instruction Account Index",
            instruction_account_index.to_string(),
        )?,
    ];

    if !reserved.is_empty() && reserved.iter().any(|&b| b != 0) {
        fields.push(make_text_field(
            "Authority Reserved (hex)",
            hex::encode(reserved),
        )?);
    }

    if payload.len() <= 17 {
        return Ok(fields);
    }

    let extra = &payload[17..];
    if extra.len() < 2 {
        return Ok(fields);
    }

    let mut kind_bytes = [0u8; 2];
    kind_bytes.copy_from_slice(&extra[..2]);
    let auth_kind = u16::from_le_bytes(kind_bytes);

    match auth_kind {
        1 => {
            fields.push(make_text_field(
                "Authority Authentication Kind",
                "WebAuthn",
            )?);
            decode_webauthn_payload(extra, &mut fields)?;
        }
        _ => {
            fields.push(make_text_field(
                "Authority Authentication Kind",
                format!("Unknown ({auth_kind})"),
            )?);
        }
    }

    Ok(fields)
}

fn decode_webauthn_payload(
    payload: &[u8],
    fields: &mut Vec<AnnotatedPayloadField>,
) -> Result<(), VisualSignError> {
    use base64::Engine;

    if payload.len() < 6 {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload too short".to_string(),
        ));
    }

    let auth_len =
        u16::from_le_bytes(payload[2..4].try_into().map_err(|_| {
            VisualSignError::DecodeError("WebAuthn payload missing auth length".into())
        })?) as usize;
    if payload.len() < 4 + auth_len + 4 + 2 + 2 + 2 {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload truncated".to_string(),
        ));
    }

    let authenticator_data = &payload[4..4 + auth_len];
    let mut offset = 4 + auth_len;

    let field_order = &payload[offset..offset + 4];
    offset += 4;

    if payload.len() < offset + 2 {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload missing origin length".to_string(),
        ));
    }
    let origin_len = u16::from_le_bytes(payload[offset..offset + 2].try_into().map_err(|_| {
        VisualSignError::DecodeError("WebAuthn payload missing origin length".into())
    })?) as usize;
    offset += 2;

    if payload.len() < offset + 2 {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload missing huffman tree length".to_string(),
        ));
    }
    let huffman_tree_len =
        u16::from_le_bytes(payload[offset..offset + 2].try_into().map_err(|_| {
            VisualSignError::DecodeError("WebAuthn payload missing huffman tree length".into())
        })?) as usize;
    offset += 2;

    if payload.len() < offset + 2 {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload missing encoded length".to_string(),
        ));
    }
    let huffman_encoded_len =
        u16::from_le_bytes(payload[offset..offset + 2].try_into().map_err(|_| {
            VisualSignError::DecodeError("WebAuthn payload missing huffman encoded length".into())
        })?) as usize;
    offset += 2;

    if payload.len() < offset + huffman_tree_len + huffman_encoded_len {
        return Err(VisualSignError::DecodeError(
            "WebAuthn payload missing huffman data".to_string(),
        ));
    }

    let huffman_tree = &payload[offset..offset + huffman_tree_len];
    let huffman_encoded =
        &payload[offset + huffman_tree_len..offset + huffman_tree_len + huffman_encoded_len];

    let origin = decode_huffman_origin(huffman_tree, huffman_encoded, origin_len)?;

    fields.push(make_text_field(
        "WebAuthn Authenticator Data (base64)",
        base64::engine::general_purpose::STANDARD.encode(authenticator_data),
    )?);
    fields.push(make_text_field(
        "WebAuthn Field Order",
        format_web_authn_field_order(field_order),
    )?);
    fields.push(make_text_field("WebAuthn Origin", origin)?);
    fields.push(make_text_field(
        "WebAuthn Huffman Tree Length",
        huffman_tree_len.to_string(),
    )?);
    fields.push(make_text_field(
        "WebAuthn Huffman Encoded Length",
        huffman_encoded_len.to_string(),
    )?);

    Ok(())
}

fn decode_huffman_origin(
    tree_data: &[u8],
    encoded_data: &[u8],
    decoded_len: usize,
) -> Result<String, VisualSignError> {
    const NODE_SIZE: usize = 3;
    const LEAF_NODE: u8 = 0;
    const BIT_MASKS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

    if tree_data.is_empty() || tree_data.len() % NODE_SIZE != 0 {
        return Err(VisualSignError::DecodeError(
            "Invalid WebAuthn Huffman tree".to_string(),
        ));
    }

    let node_count = tree_data.len() / NODE_SIZE;
    let root_index = node_count - 1;
    let mut current_node_index = root_index;
    let mut decoded = Vec::with_capacity(decoded_len);

    'outer: for &byte in encoded_data.iter() {
        for mask in BIT_MASKS {
            if decoded.len() == decoded_len {
                break 'outer;
            }

            let bit = (byte & mask) != 0;

            let node_offset = current_node_index * NODE_SIZE;
            let node_type = tree_data[node_offset];

            if node_type == LEAF_NODE {
                return Err(VisualSignError::DecodeError(
                    "Unexpected leaf node in Huffman tree traversal".to_string(),
                ));
            }

            let left_or_char = tree_data[node_offset + 1];
            let right = tree_data[node_offset + 2];
            current_node_index = if bit {
                right as usize
            } else {
                left_or_char as usize
            };

            if current_node_index >= node_count {
                return Err(VisualSignError::DecodeError(
                    "Huffman traversal exceeded node count".to_string(),
                ));
            }

            let next_offset = current_node_index * NODE_SIZE;
            if tree_data[next_offset] == LEAF_NODE {
                decoded.push(tree_data[next_offset + 1]);
                current_node_index = root_index;
            }
        }
    }

    if decoded.len() != decoded_len {
        return Err(VisualSignError::DecodeError(
            "Decoded origin length mismatch".to_string(),
        ));
    }

    String::from_utf8(decoded).map_err(|_| {
        VisualSignError::DecodeError("WebAuthn origin contained invalid UTF-8".to_string())
    })
}

fn format_web_authn_field_order(order: &[u8]) -> String {
    let names: Vec<&'static str> = order
        .iter()
        .filter_map(|value| match value {
            1 => Some("type"),
            2 => Some("challenge"),
            3 => Some("origin"),
            4 => Some("crossOrigin"),
            _ => None,
        })
        .collect();

    if names.is_empty() {
        "None".to_string()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{SolanaTransactionWrapper, SolanaVisualSignConverter};
    use solana_sdk::pubkey::Pubkey;
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
    use visualsign::{AnnotatedPayloadField, SignablePayload, SignablePayloadField};

    // Solana transaction: 2DW4HoMiC1qFXoF9ksBxQ5Krv3HgBRuZirHSmjb6e7FXLUn2hAMNXHVRj2zevYwMGfDML2Tgo35jcjgwKwX2E3Qz
    // https://solscan.io/tx/2DW4HoMiC1qFXoF9ksBxQ5Krv3HgBRuZirHSmjb6e7FXLUn2hAMNXHVRj2zevYwMGfDML2Tgo35jcjgwKwX2E3Qz?cluster=mainnet
    const CREATE_TX_B64: &str = "ATzMIEG1yDEeRQYF/Krn+0CHxBQEQsJker4QPA/zFBhbdXU8KBMyVgILu5eJ/VSMvPrEINgZMFB4ZA+V6OLZZA8BAAIFAnNQvf2dRJZKrUR078b287OV2H7ZFkZl9v9RFaK9yuizIGdX3cLn7eryN4Yx59YrilkKP0umbOBzbpxogP3zc+5DIPYFmiXQjONcOBj2j006EPDgUOTP5El+dWbyKWqPAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAANDOlC4efFBuIY3w198cUvr9w1KeSNZ00dskx1tUzMvn6KHoxw7zyLVCanW1CDNmuEXpkhyL2++YhlyQxdRfXQAQQEAQACA1AAAAEAIAD//XeaujJIZskU5W9h7pnutkAF8/fHTm3GtgtlSTwFxNqQAnNQvf2dRJZKrUR078b287OV2H7ZFkZl9v9RFaK9yugHAAAACAAAAA==";

    // Solana transaction: 3sBFrK5C2XP1RKuDdA7imBY96nVcA9Szobkk2ToXfxJyz8j3bxJaDLg9Z4f6aMfFPVedEBvVLCJ67atoXd1Teb9A
    // https://solscan.io/tx/3sBFrK5C2XP1RKuDdA7imBY96nVcA9Szobkk2ToXfxJyz8j3bxJaDLg9Z4f6aMfFPVedEBvVLCJ67atoXd1Teb9A?cluster=mainnet
    const SECP256R1_SIGN_TX_B64: &str = "AY9N+YfToyBTLx3lWDZlYOqU1lCH9bNhVli84aXrYYtBUioruJbS9Jki2vpD9AdUYPpTUIPKxwMh8p9p63DYWAkBAAcMsVw7hEKoFGL6pxm8eRWjxPFdOENnSBkgnBxuVQ9HaeFngf8DWW3R8p6nTK/wat4ty3cE2ZheMQ4+XjnZqlnojM8g8fiAQA/JO0BuYicMdkX5p7bX2ffMWCO4N/JC98q9PYJbNOPNKKI4Uwnik5ChVfa7is9F0MriOXQ0gdWE/oUJhATLbIA65wFLJmFPtECLqPGH6En0Ggv/Z8UJPUFUfQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwUxCPOxG0q6NvCa3co89OgkhESBc+Ura3xzVEz/gOsG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqQMGRm/lIRcy/+ytunLDm+e8jOW7xfcSayxDmzpAAAAADQzpQuHnxQbiGN8NffHFL6/cNSnkjWdNHbJMdbVMzL4Gkg3sL+pxtbcjgU10LakDHIPnX9t5XVaOdUeAIAAAAAan1RcYe9FmNdrUBFX9wsDBJMaPIVZ1pdu6y18IAAAAB/9uEN36lcOETkxiol3o9Sx2V1UPzjkZe1lYT44c/2ADCAAFAoCWmAAKALYBAQAxAP//EAD//3EARQD//wMFMQjzsRtKujbwmt3KPPToJIREgXPlK2t8c1RM/4DrCKnM5mIuNQiD/fAIt7RtS/40z5n+FIV/hCJCYB418XwGCZ/Z5udUJnItvZrFXRzp1eg/R75uj5ZRU2QBGbwj5cRWdMDYcUwDLdHxuNt4gy70IbmZExW8OFWRpuGg+GZQtR0AAAAAJM6hNXAjKKUvdQw0JGjEwotdmCMqyTHVDTBGz5NBHJoJCAECBQYHAwQL1QELABEAAQAAAAEEAwUGAQkAA0BCDwAAAAAA5DeMFgAAAAAKAAAABwAAAAABACUAVnTA2HFMAy3R8bjbeIMu9CG5mRMVvDhVkabhoPhmULUdAAAAAAECAwQdAGkADwAAZQAAYwAAYQABAQIAaAAAbwABBAUBAwYALgAAZwAAdgABCQoBCAsBBwwBAA0AcAAAZAABDxAAcwAAYgAAOgABExQBEhUBERYAdAAAbQAAdwABGRoBGBsALwAALQABHR4BHB8BFyABDiFWZFX7hH7LfJpbQlxTSPA=";

    fn convert_example_to_payload(base64_tx: &str, description: &str) -> SignablePayload {
        let tx_wrapper =
            SolanaTransactionWrapper::from_string(base64_tx).expect("example transaction invalid");
        SolanaVisualSignConverter
            .to_visual_sign_payload(
                tx_wrapper,
                VisualSignOptions {
                    decode_transfers: true,
                    transaction_name: Some(description.to_string()),
                },
            )
            .expect("visualization should succeed")
    }

    fn assert_text_field(fields: &[AnnotatedPayloadField], label: &str, expected: &str) {
        let actual = fields
            .iter()
            .find_map(|field| match &field.signable_payload_field {
                SignablePayloadField::TextV2 { common, text_v2 }
                    if common.label == label && !text_v2.text.is_empty() =>
                {
                    Some(text_v2.text.clone())
                }
                SignablePayloadField::Number { common, number }
                    if common.label == label && !number.number.is_empty() =>
                {
                    Some(number.number.clone())
                }
                _ => None,
            });

        let actual = actual.unwrap_or_else(|| {
            panic!(
                "Expected text field with label '{label}', available labels: {:?}",
                fields
                    .iter()
                    .filter_map(|f| match &f.signable_payload_field {
                        SignablePayloadField::TextV2 { common, .. } => Some(common.label.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            )
        });

        assert_eq!(
            actual, expected,
            "Field '{label}' did not match expected value"
        );
    }

    #[test]
    fn test_swig_create_transaction_example_decodes() {
        let payload = convert_example_to_payload(CREATE_TX_B64, "Swig Create Example");

        payload
            .validate_charset()
            .expect("payload should contain ASCII characters only");
        let json = payload
            .to_validated_json()
            .expect("payload should serialise to JSON");

        assert!(
            json.contains("Swig: Create wallet"),
            "Expected create wallet summary in payload JSON: {json}"
        );
        assert!(
            json.contains("swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB"),
            "Expected Swig program ID to be present in JSON: {json}"
        );

        assert_eq!(
            payload.fields.len(),
            3,
            "Expected three top-level fields (network, instruction, accounts)"
        );

        // Network field
        match &payload.fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Network");
                assert_eq!(text_v2.text, "Solana");
            }
            other => panic!("Expected network TextV2 field, got {other:?}"),
        }

        // Instruction field
        let instruction_layout = match &payload.fields[1] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Instruction 1");
                preview_layout
            }
            other => panic!("Expected PreviewLayout for instruction, got {other:?}"),
        };

        let title = instruction_layout
            .title
            .as_ref()
            .expect("instruction preview should have title");
        assert_eq!(title.text, "Swig: Create wallet (Ed25519)");

        let condensed_fields = &instruction_layout
            .condensed
            .as_ref()
            .expect("condensed layout missing")
            .fields;
        assert_text_field(
            condensed_fields,
            "Instruction",
            "Swig: Create wallet (Ed25519)",
        );

        let expanded_fields = &instruction_layout
            .expanded
            .as_ref()
            .expect("expanded layout missing")
            .fields;

        assert_text_field(
            expanded_fields,
            "Program ID",
            "swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB",
        );
        assert_text_field(expanded_fields, "Instruction Type", "Create Wallet");
        assert_text_field(expanded_fields, "Authority Type", "Ed25519");
        assert_text_field(expanded_fields, "Wallet PDA Bump", "255");
        assert_text_field(expanded_fields, "Wallet Address Bump", "253");
        assert_text_field(
            expanded_fields,
            "Wallet ID (hex)",
            "779aba324866c914e56f61ee99eeb64005f3f7c74e6dc6b60b65493c05c4da90",
        );
        assert_text_field(expanded_fields, "Authority Data Length", "32 bytes");
        let authority_bytes =
            hex::decode("027350bdfd9d44964aad4474efc6f6f3b395d87ed9164665f6ff5115a2bdcae8")
                .expect("valid hex");
        let authority_pubkey =
            Pubkey::new_from_array(authority_bytes.try_into().expect("32 bytes"));
        let authority_pubkey_str = authority_pubkey.to_string();
        assert_text_field(
            expanded_fields,
            "Authority Public Key",
            &authority_pubkey_str,
        );
        assert_text_field(
            expanded_fields,
            "Authority Data (hex)",
            "027350bdfd9d44964aad4474efc6f6f3b395d87ed9164665f6ff5115a2bdcae8",
        );
        assert_text_field(expanded_fields, "Actions Summary", "8 bytes (~1 action(s))");
        assert_text_field(
            expanded_fields,
            "Actions",
            "Action 1: All permissions (full access)",
        );
        assert_text_field(expanded_fields, "Actions (hex)", "0700000008000000");

        // Accounts field
        match &payload.fields[2] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Accounts");
                let subtitle = preview_layout
                    .subtitle
                    .as_ref()
                    .expect("accounts preview should have subtitle");
                assert_eq!(subtitle.text, "5 accounts");
            }
            other => panic!("Expected accounts PreviewLayout, got {other:?}"),
        }
    }

    #[test]
    fn test_swig_sign_v2_secp256r1_transaction_example_decodes() {
        let payload =
            convert_example_to_payload(SECP256R1_SIGN_TX_B64, "Swig SignV2 Secp256r1 Example");

        payload
            .validate_charset()
            .expect("payload should contain ASCII characters only");
        let json = payload
            .to_validated_json()
            .expect("payload should serialise to JSON");

        assert!(
            json.contains("Swig: Sign v2"),
            "Expected sign v2 summary in payload JSON: {json}"
        );

        assert!(
            json.to_lowercase().contains("secp256r1"),
            "Expected secp256r1 authority details in JSON payload: {json}"
        );

        assert!(
            json.contains("Secp256r1SigVerify"),
            "Expected secp256r1 verification program to be represented in JSON: {json}"
        );

        assert_eq!(
            payload.fields.len(),
            5,
            "Expected five top-level fields (network + 3 instructions + accounts)"
        );

        // Instruction 1 - Compute budget
        let compute_layout = match &payload.fields[1] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Instruction 1");
                preview_layout
            }
            other => panic!("Expected compute budget preview layout, got {other:?}"),
        };

        assert_eq!(
            compute_layout
                .title
                .as_ref()
                .expect("compute layout title")
                .text,
            "Set Compute Unit Limit: 10000000 units"
        );

        let compute_expanded = &compute_layout
            .expanded
            .as_ref()
            .expect("compute expanded missing")
            .fields;

        assert_text_field(
            compute_expanded,
            "Program ID",
            "ComputeBudget111111111111111111111111111111",
        );
        assert_text_field(compute_expanded, "Compute Unit Limit", "10000000");
        assert_text_field(compute_expanded, "Raw Data", "0280969800");

        // Instruction 2 - secp256r1 verification
        let secp_layout = match &payload.fields[2] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Instruction 2");
                preview_layout
            }
            other => panic!("Expected secp256r1 preview layout, got {other:?}"),
        };

        let secp_expanded = &secp_layout
            .expanded
            .as_ref()
            .expect("secp expanded missing")
            .fields;

        assert_text_field(
            secp_expanded,
            "Program ID",
            "Secp256r1SigVerify1111111111111111111111111",
        );
        assert_text_field(
            secp_expanded,
            "Instruction Data",
            "01003100ffff1000ffff71004500ffff03053108f3b11b4aba36f09addca3cf4e82484448173e52b6b7c73544cff80eb08a9cce6622e350883fdf008b7b46d4bfe34cf99fe14857f842242601e35f17c06099fd9e6e75426722dbd9ac55d1ce9d5e83f47be6e8f965153640119bc23e5c45674c0d8714c032dd1f1b8db78832ef421b9991315bc385591a6e1a0f86650b51d0000000024cea135702328a52f750c342468c4c28b5d98232ac931d50d3046cf93411c9a",
        );

        // Instruction 3 - Swig sign v2
        let swig_layout = match &payload.fields[3] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Instruction 3");
                preview_layout
            }
            other => panic!("Expected swig preview layout, got {other:?}"),
        };

        let swig_expanded = &swig_layout
            .expanded
            .as_ref()
            .expect("swig expanded missing")
            .fields;

        assert_text_field(
            swig_expanded,
            "Program ID",
            "swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB",
        );
        assert_text_field(swig_expanded, "Instruction Type", "Sign Transaction (v2)");
        assert_text_field(swig_expanded, "Role ID", "1");
        assert_text_field(swig_expanded, "Instruction Payload Length", "17 bytes");
        assert_text_field(swig_expanded, "Inner Instruction Count", "1");
        assert_text_field(
            swig_expanded,
            "Authority Payload (hex)",
            "e4378c16000000000a0000000700000000010025005674c0d8714c032dd1f1b8db78832ef421b9991315bc385591a6e1a0f86650b51d00000000010203041d0069000f00006500006300006100010102006800006f00010405010306002e0000670000760001090a01080b01070c01000d007000006400010f10007300006200003a00011314011215011116007400006d0000770001191a01181b002f00002d00011d1e011c1f011720010e21566455fb847ecb7c9a5b425c5348f0",
        );
        assert_text_field(
            swig_expanded,
            "Inner Instruction 1 Summary",
            "From: 597A8Cxf3rrD1svuCYD8fpoXrwyBVgyCkAC2X8ZY1WWp\nTo: e9RnUuELJyhgRqEjtVRN4T1Zxu8t4YXr5q4PHhvvwi4\nOwner: EwYXm44uBFw4SPmyKsRBiTemWan13ikp1q3w84CUBtAg\nAmount: 1000000",
        );
        assert_text_field(
            swig_expanded,
            "Inner Instruction 1 Program",
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        );
        assert_text_field(
            swig_expanded,
            "Inner Instruction 1 Accounts",
            "1: 597A8Cxf3rrD1svuCYD8fpoXrwyBVgyCkAC2X8ZY1WWp\n2: e9RnUuELJyhgRqEjtVRN4T1Zxu8t4YXr5q4PHhvvwi4\n3: EwYXm44uBFw4SPmyKsRBiTemWan13ikp1q3w84CUBtAg",
        );
        assert_text_field(
            swig_expanded,
            "Inner Instruction 1 Data (hex)",
            "0340420f0000000000",
        );

        // Accounts preview
        let accounts_layout = match &payload.fields[4] {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Accounts");
                preview_layout
            }
            other => panic!("Expected accounts preview layout, got {other:?}"),
        };

        assert_eq!(
            accounts_layout
                .subtitle
                .as_ref()
                .expect("accounts subtitle")
                .text,
            "12 accounts"
        );

        let accounts_condensed = &accounts_layout
            .condensed
            .as_ref()
            .expect("accounts condensed missing")
            .fields;
        assert_text_field(accounts_condensed, "Signers", "1 Signer");
        assert_text_field(accounts_condensed, "Writable", "4 Writable");
        assert_text_field(accounts_condensed, "Read Only", "7 Read Only");
    }

    #[test]
    fn test_decode_actions_helper_outputs_full_details() {
        let action_bytes = vec![0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00];
        let decoded = super::decode_actions(&action_bytes).expect("decode succeeds");
        assert_eq!(
            decoded,
            vec!["Action 1: All permissions (full access)".to_string()]
        );

        let rows = super::action_detail_rows("Test Actions", &action_bytes);
        assert_eq!(rows.len(), 2, "expected text + hex rows");
        assert_eq!(rows[0].0, "Test Actions");
        assert_eq!(rows[0].1, "Action 1: All permissions (full access)");
        assert_eq!(rows[1].0, "Test Actions (hex)");
        assert_eq!(rows[1].1, hex::encode(&action_bytes));
    }

    #[test]
    fn test_authority_update_rows_show_replacement_details() {
        let action_bytes = vec![0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00];
        let rows = super::authority_update_rows(&super::AuthorityUpdateDetails::ReplaceAll(
            action_bytes.clone(),
        ));
        assert!(
            rows.iter()
                .any(|(label, value)| label == "Operation Type" && value == "Replace actions")
        );
        assert!(rows.iter().any(|(label, value)| {
            label == "Updated Actions (hex)" && *value == hex::encode(&action_bytes)
        }));
    }
}
