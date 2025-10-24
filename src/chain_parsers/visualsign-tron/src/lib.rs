use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

use anychain_tron::protocol::Tron::transaction;
use anychain_tron::protocol::balance_contract::TransferContract;
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use protobuf::Message;
use sha2::{Digest, Sha256};

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum TronParserError {
    #[error("Failed to decode transaction: {0}")]
    FailedToDecodeTransaction(String),
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<transaction::Raw, TronParserError> {
    let bytes = match encodings {
        SupportedEncodings::Hex => {
            let clean_hex = raw_transaction
                .strip_prefix("0x")
                .unwrap_or(raw_transaction);
            hex::decode(clean_hex).map_err(|e| {
                TronParserError::FailedToDecodeTransaction(format!("Failed to decode hex: {e}"))
            })?
        }
        SupportedEncodings::Base64 => b64.decode(raw_transaction).map_err(|e| {
            TronParserError::FailedToDecodeTransaction(format!("Failed to decode base64: {e}"))
        })?,
    };

    // Parse and return the Tron transaction
    transaction::Raw::parse_from_bytes(&bytes).map_err(|e| {
        TronParserError::FailedToDecodeTransaction(format!("Failed to parse Tron transaction: {e}"))
    })
}

// This module provides a parser and wrapper for Tron blockchain transactions,
// enabling their decoding and integration with the VisualSign framework.
/// Wrapper for Tron transactions
#[derive(Debug, Clone)]
pub struct TronTransactionWrapper {
    transaction: transaction::Raw,
}

impl Transaction for TronTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        let format = if data.starts_with("0x") {
            SupportedEncodings::Hex
        } else {
            visualsign::encodings::SupportedEncodings::detect(data)
        };
        let transaction = decode_transaction(data, format)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;
        Ok(Self { transaction })
    }

    fn transaction_type(&self) -> String {
        "Tron".to_string()
    }
}

impl TronTransactionWrapper {
    pub fn new(transaction: transaction::Raw) -> Self {
        Self { transaction }
    }

    pub fn inner(&self) -> &transaction::Raw {
        &self.transaction
    }
}

/// Converter for Tron transactions
pub struct TronVisualSignConverter;

impl VisualSignConverter<TronTransactionWrapper> for TronVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: TronTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        convert_to_visual_sign_payload(transaction_wrapper.inner().clone(), options)
    }
}

fn convert_to_visual_sign_payload(
    raw_data: transaction::Raw,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let chain_name = "Tron".to_string();

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: "Tron".to_string(),
            label: "Network".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text: chain_name },
    }];

    // Add timestamp field
    let timestamp_formatted = format_timestamp(raw_data.timestamp);
    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{} ({} ms)", timestamp_formatted, raw_data.timestamp),
            label: "Timestamp".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: format!("{} ({} ms)", timestamp_formatted, raw_data.timestamp),
        },
    });

    // Add expiration field
    let expiration_formatted = format_timestamp(raw_data.expiration);
    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{} ({} ms)", expiration_formatted, raw_data.expiration),
            label: "Expiration".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: format!("{} ({} ms)", expiration_formatted, raw_data.expiration),
        },
    });

    // Add fee limit field
    let fee_limit_trx = raw_data.fee_limit as f64 / 1_000_000.0;
    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{} SUN ({} TRX)", raw_data.fee_limit, fee_limit_trx),
            label: "Fee Limit".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: format!("{} SUN ({} TRX)", raw_data.fee_limit, fee_limit_trx),
        },
    });

    // Add ref block bytes field
    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: hex::encode(&raw_data.ref_block_bytes),
            label: "Ref Block".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: hex::encode(&raw_data.ref_block_bytes),
        },
    });

    // Add ref block hash field
    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: hex::encode(&raw_data.ref_block_hash),
            label: "Ref Block Hash".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: hex::encode(&raw_data.ref_block_hash),
        },
    });

    // Parse contracts
    for contract in raw_data.contract.iter() {
        if let Some(parameter) = contract.parameter.as_ref() {
            // Decode specific contract types
            match parameter.type_url.as_str() {
                "type.googleapis.com/protocol.TransferContract" => {
                    if let Ok(transfer) = TransferContract::parse_from_bytes(&parameter.value) {
                        // Add contract type field
                        fields.push(SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: "TransferContract (TRX Transfer)".to_string(),
                                label: "Contract Type".to_string(),
                            },
                            text_v2: SignablePayloadFieldTextV2 {
                                text: "TransferContract (TRX Transfer)".to_string(),
                            },
                        });

                        // Add from address field
                        let from_address = address_to_base58(&transfer.owner_address);
                        fields.push(SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: from_address.clone(),
                                label: "From".to_string(),
                            },
                            text_v2: SignablePayloadFieldTextV2 { text: from_address },
                        });

                        // Add to address field
                        let to_address = address_to_base58(&transfer.to_address);
                        fields.push(SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: to_address.clone(),
                                label: "To".to_string(),
                            },
                            text_v2: SignablePayloadFieldTextV2 { text: to_address },
                        });

                        // Add amount field
                        let amount_trx = transfer.amount as f64 / 1_000_000.0;
                        fields.push(SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: format!(
                                    "{} SUN ({} TRX)",
                                    transfer.amount, amount_trx
                                ),
                                label: "Amount".to_string(),
                            },
                            text_v2: SignablePayloadFieldTextV2 {
                                text: format!("{} SUN ({} TRX)", transfer.amount, amount_trx),
                            },
                        });
                    }
                }
                _ => {
                    // Unknown contract type
                    fields.push(SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: parameter.type_url.clone(),
                            label: "Contract Type".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: format!("{} (not fully decoded)", parameter.type_url),
                        },
                    });
                }
            }
        }
    }

    let title = options
        .transaction_name
        .unwrap_or_else(|| "Tron Transaction".to_string());

    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "TronTx".to_string(),
    ))
}

impl VisualSignConverterFromString<TronTransactionWrapper> for TronVisualSignConverter {}

// Public API functions
pub fn transaction_to_visual_sign(
    transaction: transaction::Raw,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = TronTransactionWrapper::new(transaction);
    let converter = TronVisualSignConverter;
    converter.to_visual_sign_payload(wrapper, options)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = TronVisualSignConverter;
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

// Helper function to convert Tron address bytes to base58 format
fn address_to_base58(address_bytes: &[u8]) -> String {
    // Add checksum
    let mut hasher = Sha256::new();
    hasher.update(address_bytes);
    let hash1 = hasher.finalize();

    let mut hasher = Sha256::new();
    hasher.update(hash1);
    let hash2 = hasher.finalize();

    let mut with_checksum = address_bytes.to_vec();
    with_checksum.extend_from_slice(&hash2[..4]);

    base58::ToBase58::to_base58(&with_checksum[..])
}

// Helper function to format Unix timestamp (milliseconds) to human-readable format
fn format_timestamp(timestamp_ms: i64) -> String {
    use chrono::{TimeZone, Utc};

    let datetime = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}
