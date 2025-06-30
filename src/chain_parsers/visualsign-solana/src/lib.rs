use base64::engine::Engine;
use borsh::de::BorshDeserialize;
use solana_parser::solana::parser::parse_transaction;
use solana_program::system_instruction::SystemInstruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use spl_associated_token_account::instruction::AssociatedTokenAccountInstruction;
use spl_stake_pool::instruction::StakePoolInstruction;
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
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
            "11111111111111111111111111111111" => {
                match parse_system_instruction(&instruction.data) {
                    Ok(instruction_type) => format!("{:?}", instruction_type),
                    Err(err) => {
                        println!("Failed to parse system instruction: {}", err);
                        "Unknown Instruction".to_string()
                    }
                }
            }
            program_id if program_id.starts_with("AToken") => {
                // Decode associated token address
                match parse_ata_instruction(&instruction.data) {
                    Ok(instruction_type) => {
                        format!("Associated Token Address: {:?}", instruction_type)
                    }
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
                    Ok(instruction_type) => {
                        format!("Stake Pool Instruction: {:?}", instruction_type)
                    }
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

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                AnnotatedPayloadField {
                    static_annotation: None,
                    dynamic_annotation: None,
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: program_id.clone(),
                            label: "Program ID".to_string(),
                        },
                        text_v2: visualsign::SignablePayloadFieldTextV2 {
                            text: program_id.clone(),
                        },
                    },
                },
                // DO we need the accounts field here?
                AnnotatedPayloadField {
                    static_annotation: None,
                    dynamic_annotation: None,
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: hex::encode(&instruction.data),
                            label: "Data".to_string(),
                        },
                        text_v2: visualsign::SignablePayloadFieldTextV2 {
                            text: hex::encode(&instruction.data),
                        },
                    },
                },
            ],
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
