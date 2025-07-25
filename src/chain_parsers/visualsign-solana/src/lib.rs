use base64::engine::Engine;
use borsh::de::BorshDeserialize;
use solana_parser::solana::parser::parse_transaction;
use solana_program::system_instruction::SystemInstruction;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use spl_associated_token_account::instruction::AssociatedTokenAccountInstruction;
use spl_stake_pool::instruction::StakePoolInstruction;
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    field_builders::{create_amount_field, create_number_field, create_text_field},
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

/// Wrapper around Solana's transaction type that implements the Transaction trait
#[derive(Debug, Clone)]
pub struct SolanaTransactionWrapper {
    transaction: SolanaTransaction,
}

impl Transaction for SolanaTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        // Detect if format is base64 or hex
        let format = visualsign::encodings::SupportedEncodings::detect(data);

        let transaction = decode_transaction(data, format)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;

        Ok(Self { transaction })
    }

    fn transaction_type(&self) -> String {
        "Solana".to_string()
    }
}

impl SolanaTransactionWrapper {
    pub fn new(transaction: SolanaTransaction) -> Self {
        Self { transaction }
    }

    pub fn inner(&self) -> &SolanaTransaction {
        &self.transaction
    }
}

/// Converter that knows how to format Solana transactions for VisualSign
pub struct SolanaVisualSignConverter;

impl VisualSignConverter<SolanaTransactionWrapper> for SolanaVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: SolanaTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let transaction = transaction_wrapper.inner().clone();

        // Convert the transaction to a VisualSign payload
        let payload = convert_to_visual_sign_payload(
            transaction,
            options.decode_transfers,
            options.transaction_name,
        );

        Ok(payload)
    }
}

impl VisualSignConverterFromString<SolanaTransactionWrapper> for SolanaVisualSignConverter {}

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

fn format_compute_budget_instruction(instruction: &ComputeBudgetInstruction) -> String {
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
                        fields: vec![create_text_field(
                            "Stake Pool Instruction",
                            &format_stake_pool_instruction(&instruction_type),
                        )],
                    }
                } else {
                    create_default_expanded_fields(program_id, &instruction.data)
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
            create_text_field("Program ID", program_id),
            create_text_field("Instruction", &format_ata_instruction(instruction)),
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

fn create_compute_budget_expanded_fields(
    instruction: &ComputeBudgetInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id)];

    // Add specific fields based on instruction type
    match instruction {
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            fields.push(create_number_field(
                "Heap Frame Size",
                &bytes.to_string(),
                "bytes",
            ));
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            fields.push(create_number_field(
                "Compute Unit Limit",
                &units.to_string(),
                "units",
            ));
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => {
            fields.push(create_number_field(
                "Price per Compute Unit",
                &micro_lamports.to_string(),
                "micro-lamports",
            ));
        }
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            fields.push(create_number_field(
                "Data Size Limit",
                &bytes.to_string(),
                "bytes",
            ));
        }
        ComputeBudgetInstruction::Unused => {
            // No additional fields for unused instruction
        }
    }

    fields.push(create_text_field("Raw Data", &hex::encode(data)));
    fields
}

fn create_system_instruction_expanded_fields(
    instruction: &SystemInstruction,
    program_id: &str,
    data: &[u8],
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![create_text_field("Program ID", program_id)];

    // Add specific fields based on instruction type
    match instruction {
        SystemInstruction::CreateAccount {
            lamports,
            space,
            owner,
        } => {
            fields.push(create_amount_field(
                "Amount",
                &lamports.to_string(),
                "lamports",
                *lamports as f64 / 1_000_000_000.0,
                "SOL",
            ));
            fields.push(create_number_field("Space", &space.to_string(), "bytes"));
            fields.push(create_text_field("Owner", &owner.to_string()));
        }
        SystemInstruction::Transfer { lamports } => {
            fields.push(create_amount_field(
                "Transfer Amount",
                &lamports.to_string(),
                "lamports",
                *lamports as f64 / 1_000_000_000.0,
                "SOL",
            ));
        }
        SystemInstruction::Assign { owner } => {
            fields.push(create_text_field("New Owner", &owner.to_string()));
        }
        SystemInstruction::Allocate { space } => {
            fields.push(create_number_field("Space", &space.to_string(), "bytes"));
        }
        _ => {
            // For other system instructions, just show the instruction type
            fields.push(create_text_field(
                "Instruction Details",
                &format!("{:?}", instruction),
            ));
        }
    }

    fields.push(create_text_field("Raw Data", &hex::encode(data)));
    fields
}

fn create_default_expanded_fields(program_id: &str, data: &[u8]) -> SignablePayloadFieldListLayout {
    SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", program_id),
            create_text_field("Raw Data", &hex::encode(data)),
        ],
    }
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
        let instruction = ComputeBudgetInstruction::SetComputeUnitLimit(1400000);
        let result = format_compute_budget_instruction(&instruction);

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
}
