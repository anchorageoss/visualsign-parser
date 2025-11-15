use crate::fmt::{format_ether, format_gwei};
use alloy_consensus::{Transaction as _, TxType, TypedTransaction};
use alloy_rlp::{Buf, Decodable};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

pub mod chains;
pub mod context;
pub mod contracts;
pub mod fmt;
pub mod protocols;
pub mod registry;
pub mod token_metadata;
pub mod visualizer;

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum EthereumParserError {
    #[error("Unexpected trailing data: {0}")]
    UnexpectedTrailingData(String),
    #[error("Unexpected transaction type: {0}")]
    UnexpectedTransactionType(String),
    #[error("Unsupported transaction type: {0}")]
    UnsupportedTransactionType(String),
    #[error("Failed to decode transaction: {0}")]
    FailedToDecodeTransaction(String),
}

// Helper function to extract gas price from different transaction types
fn extract_gas_price(transaction: &TypedTransaction) -> u128 {
    match transaction {
        TypedTransaction::Legacy(tx) => tx.gas_price,
        TypedTransaction::Eip2930(tx) => tx.gas_price,
        TypedTransaction::Eip1559(tx) => tx.max_fee_per_gas,
        TypedTransaction::Eip4844(tx) => match tx {
            alloy_consensus::TxEip4844Variant::TxEip4844(inner_tx) => inner_tx.max_fee_per_gas,
            alloy_consensus::TxEip4844Variant::TxEip4844WithSidecar(sidecar_tx) => {
                sidecar_tx.tx.max_fee_per_gas
            }
        },
        TypedTransaction::Eip7702(tx) => tx.max_fee_per_gas,
    }
}

// Helper function to extract priority fee from transaction types that support it
fn extract_priority_fee(transaction: &TypedTransaction) -> Option<u128> {
    match transaction {
        TypedTransaction::Eip1559(tx) => Some(tx.max_priority_fee_per_gas),
        TypedTransaction::Eip4844(tx) => match tx {
            alloy_consensus::TxEip4844Variant::TxEip4844(inner_tx) => {
                Some(inner_tx.max_priority_fee_per_gas)
            }
            alloy_consensus::TxEip4844Variant::TxEip4844WithSidecar(sidecar_tx) => {
                Some(sidecar_tx.tx.max_priority_fee_per_gas)
            }
        },
        TypedTransaction::Eip7702(tx) => Some(tx.max_priority_fee_per_gas),
        TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => None,
    }
}

// Helper function to create priority fee field
fn create_priority_fee_field(max_priority_fee_per_gas: u128) -> SignablePayloadField {
    let priority_fee_text = format!("{} gwei", format_gwei(max_priority_fee_per_gas));
    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: priority_fee_text.clone(),
            label: "Max Priority Fee Per Gas".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: priority_fee_text,
        },
    }
}

/// Wrapper around Alloy's transaction type that implements the Transaction trait
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EthereumTransactionWrapper {
    transaction: TypedTransaction,
}

impl Transaction for EthereumTransactionWrapper {
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
        "Ethereum".to_string()
    }
}

impl EthereumTransactionWrapper {
    pub fn new(transaction: TypedTransaction) -> Self {
        Self { transaction }
    }
    pub fn inner(&self) -> &TypedTransaction {
        &self.transaction
    }
}

/// Converter that knows how to format Ethereum transactions for VisualSign
pub struct EthereumVisualSignConverter;

impl VisualSignConverter<EthereumTransactionWrapper> for EthereumVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: EthereumTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let transaction = transaction_wrapper.inner().clone();
        let is_supported = match transaction.tx_type() {
            TxType::Eip2930 | TxType::Eip4844 | TxType::Eip7702 => false,
            TxType::Legacy | TxType::Eip1559 => true,
        };
        if is_supported {
            return Ok(convert_to_visual_sign_payload(transaction, options));
        }
        Err(VisualSignError::DecodeError(format!(
            "Unsupported transaction type: {}",
            transaction.tx_type()
        )))
    }
}

impl VisualSignConverterFromString<EthereumTransactionWrapper> for EthereumVisualSignConverter {}
fn decode_transaction_bytes(mut buf: &[u8]) -> Result<TypedTransaction, EthereumParserError> {
    let tx = if buf.is_empty() {
        Err(EthereumParserError::FailedToDecodeTransaction(
            "Input too short".to_string(),
        ))
    } else if buf[0] == 0 || (buf[0] > 0x7f && buf[0] < 0xc0) {
        Err(EthereumParserError::FailedToDecodeTransaction(format!(
            "Unexpected type flag {}.",
            buf[0]
        )))
    } else if buf[0] <= 0x7f {
        let ty: TxType = match buf[0].try_into() {
            Ok(t) => t,
            Err(e) => {
                return Err(EthereumParserError::FailedToDecodeTransaction(
                    e.to_string(),
                ));
            }
        };
        buf.advance(1); // Skip type byte
        match ty {
            TxType::Eip1559 => Ok(TypedTransaction::Eip1559(
                alloy_consensus::TxEip1559::decode(&mut buf)
                    .map_err(|e| EthereumParserError::FailedToDecodeTransaction(e.to_string()))?,
            )),
            TxType::Eip2930 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-2930".to_string(),
            )),
            TxType::Eip4844 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-4844".to_string(),
            )),
            TxType::Eip7702 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-7702".to_string(),
            )),
            TxType::Legacy => Err(EthereumParserError::UnexpectedTransactionType(
                "legacy".to_string(), // This shouldn't happen
            )),
        }
    } else {
        Ok(TypedTransaction::Legacy(
            alloy_consensus::TxLegacy::decode(&mut buf)
                .map_err(|e| EthereumParserError::FailedToDecodeTransaction(e.to_string()))?,
        ))
    };
    if tx.is_ok() && !buf.is_empty() {
        return Err(EthereumParserError::UnexpectedTrailingData(hex::encode(
            buf,
        )));
    }
    tx
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<TypedTransaction, EthereumParserError> {
    let bytes = match encodings {
        SupportedEncodings::Hex => {
            let clean_hex = raw_transaction
                .strip_prefix("0x")
                .unwrap_or(raw_transaction);
            hex::decode(clean_hex).map_err(|e| {
                EthereumParserError::FailedToDecodeTransaction(format!("Failed to decode hex: {e}"))
            })?
        }
        SupportedEncodings::Base64 => b64.decode(raw_transaction).map_err(|e| {
            EthereumParserError::FailedToDecodeTransaction(format!("Failed to decode base64: {e}"))
        })?,
    };
    decode_transaction_bytes(&bytes)
}

fn convert_to_visual_sign_payload(
    transaction: TypedTransaction,
    options: VisualSignOptions,
) -> SignablePayload {
    // Extract chain ID to determine the network
    let chain_id = transaction.chain_id();

    let chain_name = chains::get_chain_name(chain_id);

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: chain_name.clone(),
            label: "Network".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text: chain_name },
    }];
    if let Some(to) = transaction.to() {
        fields.push(SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: to.to_string(),
                label: "To".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: to.to_string(),
                name: "To".to_string(),
                asset_label: "Test Asset".to_string(),
                memo: None,
                badge_text: None,
            },
        });
    }
    fields.extend([
        SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} ETH", format_ether(transaction.value())),
                label: "Value".to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: format_ether(transaction.value()),
                abbreviation: Some("ETH".to_string()),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{}", transaction.gas_limit()),
                label: "Gas Limit".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{}", transaction.gas_limit()),
            },
        },
    ]);

    // Handle gas pricing based on transaction type
    let gas_price_text = format!("{} gwei", format_gwei(extract_gas_price(&transaction)));

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: gas_price_text.clone(),
            label: "Gas Price".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: gas_price_text,
        },
    });

    // Add priority fee for EIP-1559, EIP-4844, and EIP-7702 transactions
    if let Some(priority_fee) = extract_priority_fee(&transaction) {
        fields.push(create_priority_fee_field(priority_fee));
    }

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{}", transaction.nonce()),
            label: "Nonce".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: format!("{}", transaction.nonce()),
        },
    });

    // Add contract call data if present
    let input = transaction.input();
    if !input.is_empty() {
        let mut input_fields: Vec<SignablePayloadField> = Vec::new();
        if options.decode_transfers {
            if let Some(field) = (contracts::erc20::ERC20Visualizer {}).visualize_tx_commands(input)
            {
                input_fields.push(field);
            }
        }
        if let Some(field) =
            (contracts::uniswap::UniswapV4Visualizer {}).visualize_tx_commands(input)
        {
            input_fields.push(field);
        }
        if input_fields.is_empty() {
            input_fields.push(SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("0x{}", hex::encode(input)),
                    label: "Input Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("0x{}", hex::encode(input)),
                },
            });
        }
        fields.append(&mut input_fields);
    }

    let title = options
        .transaction_name
        .unwrap_or_else(|| "Ethereum Transaction".to_string());
    SignablePayload::new(0, title, None, fields, "EthereumTx".to_string())
}

// Public API functions for ease of use
pub fn transaction_to_visual_sign(
    transaction: TypedTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = EthereumTransactionWrapper::new(transaction);
    let converter = EthereumVisualSignConverter;
    converter.to_visual_sign_payload(wrapper, options)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = EthereumVisualSignConverter;
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxLegacy, TypedTransaction};
    use alloy_primitives::{Address, Bytes, ChainId, U256};
    use visualsign::SignablePayloadFieldAddressV2;

    fn unsigned_to_hex(tx: &TypedTransaction) -> String {
        let mut encoded = Vec::new();
        tx.encode_for_signing(&mut encoded);
        format!("0x{}", hex::encode(&encoded))
    }

    #[test]
    fn test_transaction_to_visual_sign_basic() {
        // Create a dummy Ethereum transaction
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 42,
            gas_price: 20_000_000_000u128, // 20 gwei
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(
                "0x000000000000000000000000000000000000dead"
                    .parse()
                    .unwrap(),
            ),
            value: U256::from(1000000000000000000u64), // 1 ETH
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let expected_payload = SignablePayload::new(
            0,
            "Ethereum Transaction".to_string(),
            None,
            vec![
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Ethereum Mainnet".to_string(),
                        label: "Network".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Ethereum Mainnet".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "0x000000000000000000000000000000000000dEaD".to_string(),
                        label: "To".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "0x000000000000000000000000000000000000dEaD".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "1 ETH".to_string(),
                        label: "Value".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "1 ETH".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "21000".to_string(),
                        label: "Gas Limit".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "21000".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "20 gwei".to_string(),
                        label: "Gas Price".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "20 gwei".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "42".to_string(),
                        label: "Nonce".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "42".to_string(),
                    },
                },
            ],
            "EthereumTx".to_string(),
        );

        // Compare individual fields since SignablePayload doesn't implement PartialEq
        assert_eq!(expected_payload.title, payload.title);
        assert_eq!(expected_payload.version, payload.version);
        assert_eq!(expected_payload.subtitle, payload.subtitle);
        assert_eq!(expected_payload.fields.len(), payload.fields.len());
        assert_eq!(expected_payload.payload_type, payload.payload_type);

        for (expected_field, actual_field) in
            expected_payload.fields.iter().zip(payload.fields.iter())
        {
            assert_eq!(expected_field.label(), actual_field.label());
            if let (
                SignablePayloadField::TextV2 {
                    text_v2: expected_text,
                    ..
                },
                SignablePayloadField::TextV2 {
                    text_v2: actual_text,
                    ..
                },
            ) = (expected_field, actual_field)
            {
                assert_eq!(expected_text.text, actual_text.text);
            }
        }
    }

    #[test]
    fn test_transaction_with_input_data() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 1,
            gas_price: 1_000_000_000u128,
            gas_limit: 50000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::from(vec![0x12, 0x34, 0x56, 0x78]),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        // Check that input data field is present
        assert!(payload.fields.iter().any(|f| f.label() == "Input Data"));
        let input_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Input Data")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = input_field {
            assert_eq!(text_v2.text, "0x12345678");
        }
    }

    #[test]
    fn test_transaction_with_custom_title() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Custom Transaction Title".to_string()),
            metadata: None,
        };
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        assert_eq!(payload.title, "Custom Transaction Title");
    }

    #[test]
    fn test_transaction_wrapper_from_string() {
        // Test with empty string
        assert_eq!(
            EthereumTransactionWrapper::from_string(""),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Input too short".to_string()
            )),
        );
        // Test with invalid hex data
        assert_eq!(
            EthereumTransactionWrapper::from_string("invalid_hex_data"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode base64: Invalid symbol 95, offset 7.".to_string()
            )),
        );
        // Test with malformed hex (odd length)
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x123"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode hex: Odd number of digits"
                    .to_string()
            )),
        );
        // Test with valid hex prefix but invalid RLP data
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x1234567890abcdef"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 18.".to_string()
            )),
        );
        // Test with valid base64 but invalid RLP data
        assert_eq!(
            EthereumTransactionWrapper::from_string("aGVsbG8gd29ybGQ="),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 104.".to_string()
            )),
        );
        // Test with unknown transaction type
        assert_eq!(
            EthereumTransactionWrapper::from_string(
                "0x05f86401808504a817c800825208940000000000000000000000000000000000000000880de0b6b3a764000080c0"
            ),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 5.".to_string()
            )),
        );
        // Test with corrupted typed transaction (invalid RLP after type byte)
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x02ff"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: input too short".to_string()
            )),
        );
        // Test with valid transaction type but insufficient data
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x02"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: input too short".to_string()
            )),
        );
        // Test with whitespace in input (should fail due to invalid format)
        assert_eq!(
            EthereumTransactionWrapper::from_string(" 0x1234 "),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode base64: Invalid symbol 32, offset 0.".to_string()
            )),
        );
        // Test with legacy transaction
        let legacy_tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 20_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&legacy_tx)),
            Ok(EthereumTransactionWrapper::new(legacy_tx.clone())),
        );
        // Test with EIP-1559 transaction
        let eip1559_tx = TypedTransaction::Eip1559(alloy_consensus::TxEip1559 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip1559_tx)),
            Ok(EthereumTransactionWrapper::new(eip1559_tx.clone())),
        );
        // Test with EIP-2930 transaction (unsupported)
        let eip2930_tx = TypedTransaction::Eip2930(alloy_consensus::TxEip2930 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            gas_price: 20_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip2930_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-2930".to_string()
            ))
        );
        // Test with EIP-4844 transaction (unsupported)
        let eip4844_tx = TypedTransaction::Eip4844(alloy_consensus::TxEip4844Variant::TxEip4844(
            alloy_consensus::TxEip4844 {
                chain_id: ChainId::from(1u64),
                nonce: 1,
                gas_limit: 21000,
                max_fee_per_gas: 30_000_000_000u128,
                max_priority_fee_per_gas: 2_000_000_000u128,
                to: Address::ZERO,
                value: U256::from(1000000000000000000u64),
                access_list: Default::default(),
                input: Bytes::new(),
                blob_versioned_hashes: Default::default(),
                max_fee_per_blob_gas: 10_000_000_000u128,
            },
        ));
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip4844_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-4844".to_string()
            ))
        );
        // Test with EIP-7702 transaction (unsupported)
        let eip7702_tx = TypedTransaction::Eip7702(alloy_consensus::TxEip7702 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: Address::ZERO,
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
            authorization_list: Default::default(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip7702_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-7702".to_string()
            ))
        );
    }

    #[test]
    fn test_transaction_wrapper_type() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let wrapper = EthereumTransactionWrapper::new(tx);
        assert_eq!(wrapper.transaction_type(), "Ethereum");
    }

    #[test]
    fn test_zero_value_transaction() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let value_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Value")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = value_field {
            assert!(text_v2.text.contains("0"));
            assert!(text_v2.text.contains("ETH"));
        }
    }
    #[test]
    fn test_transaction_to_visual_sign_public_api() {
        // Test the public API function
        let tx = TypedTransaction::Eip1559(alloy_consensus::TxEip1559 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            transaction_string_to_visual_sign(
                &unsigned_to_hex(&tx),
                VisualSignOptions {
                    decode_transfers: true,
                    transaction_name: Some("Test Transaction".to_string()),
                    metadata: None,
                }
            ),
            Ok(SignablePayload::new(
                0,
                "Test Transaction".to_string(),
                None,
                vec![
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "Ethereum Mainnet".to_string(),
                            label: "Network".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "Ethereum Mainnet".to_string(),
                        },
                    },
                    SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "0x0000000000000000000000000000000000000000".to_string(),
                            label: "To".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: "0x0000000000000000000000000000000000000000".to_string(),
                            name: "To".to_string(),
                            asset_label: "Test Asset".to_string(),
                            memo: None,
                            badge_text: None,
                        },
                    },
                    SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "1 ETH".to_string(),
                            label: "Value".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: "1".to_string(),
                            abbreviation: Some("ETH".to_string()),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "21000".to_string(),
                            label: "Gas Limit".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "21000".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "30 gwei".to_string(),
                            label: "Gas Price".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "30 gwei".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "2 gwei".to_string(),
                            label: "Max Priority Fee Per Gas".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "2 gwei".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "1".to_string(),
                            label: "Nonce".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "1".to_string(),
                        },
                    },
                ],
                "EthereumTx".to_string()
            ))
        );
    }
}
