use alloy_consensus::{Transaction as _, TxType, TypedTransaction};
use alloy_primitives::{U256, utils::format_units};
use alloy_rlp::{Buf, Decodable};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

pub mod chains;
fn trim_trailing_zeros(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

// Helper function to format wei to ether
fn format_ether(wei: U256) -> String {
    trim_trailing_zeros(format_units(wei, 18).unwrap_or_else(|_| wei.to_string()))
}

/// Wrapper around Alloy's transaction type that implements the Transaction trait
#[derive(Debug, Clone)]
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
            "Unsupported variant: {}",
            transaction.tx_type()
        )))
    }
}

impl VisualSignConverterFromString<EthereumTransactionWrapper> for EthereumVisualSignConverter {}
fn decode_transaction_bytes(
    mut buf: &[u8],
) -> Result<TypedTransaction, Box<dyn std::error::Error>> {
    let tx = if buf.is_empty() {
        Err("Failed to decode transaction: input too short".into())
    } else if buf[0] == 0 || (buf[0] > 0x7f && buf[0] < 0xc0) {
        Err(format!(
            "Failed to decode transaction: unexpected tx type {}",
            buf[0]
        )
        .into())
    } else if buf[0] <= 0x7f {
        let ty = buf[0]
            .try_into()
            .map_err(|e| format!("Failed to decode transaction: {e}"))?;
        buf.advance(1); // Skip type byte
        match ty {
            TxType::Eip1559 => Ok(TypedTransaction::Eip1559(
                alloy_consensus::TxEip1559::decode(&mut buf)
                    .map_err(|e| format!("Failed to decode transaction: {}", e))?,
            )),
            TxType::Eip2930 => Err("Unsupported variant eip-2930".into()),
            TxType::Eip4844 => Err("Unsupported variant eip-4844".into()),
            TxType::Eip7702 => Err("Unsupported variant eip-7702".into()),
            TxType::Legacy => Err("Unexpected legacy variant".into()),
        }
    } else {
        Ok(TypedTransaction::Legacy(
            alloy_consensus::TxLegacy::decode(&mut buf)
                .map_err(|e| format!("Failed to decode transaction: {}", e))?,
        ))
    };
    if tx.is_ok() && !buf.is_empty() {
        return Err(Box::<dyn std::error::Error>::from(format!(
            "Unexpected trailing data after transaction: {}",
            hex::encode(buf)
        )));
    }
    tx
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<TypedTransaction, Box<dyn std::error::Error>> {
    let bytes = match encodings {
        SupportedEncodings::Hex => {
            let clean_hex = raw_transaction
                .strip_prefix("0x")
                .unwrap_or(raw_transaction);
            hex::decode(clean_hex).map_err(|e| format!("Failed to decode hex: {}", e))?
        }
        SupportedEncodings::Base64 => b64
            .decode(raw_transaction)
            .map_err(|e| format!("Failed to decode base64: {}", e))?,
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
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: to.to_string(),
                label: "To".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: to.to_string(),
            },
        });
    }
    fields.extend([
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} ETH", format_ether(transaction.value())),
                label: "Value".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{} ETH", format_ether(transaction.value())),
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

    if let Some(gas_price) = transaction.gas_price() {
        let gas_price_text = format!("{} ETH", format_ether(U256::from(gas_price)));
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: gas_price_text.clone(),
                label: "Gas Price".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: gas_price_text,
            },
        });
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
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("0x{}", hex::encode(input)),
                label: "Input Data".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("0x{}", hex::encode(input)),
            },
        });
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
                        fallback_text: "0.00000002 ETH".to_string(),
                        label: "Gas Price".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "0.00000002 ETH".to_string(),
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
        };
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        assert_eq!(payload.title, "Custom Transaction Title");
    }

    #[test]
    fn test_transaction_wrapper_from_string() {
        // Test with empty string
        let result = EthereumTransactionWrapper::from_string("");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode transaction: input too short", msg);
        } else {
            panic!("Expected decode error for empty string");
        }

        // Test with invalid hex data
        let result = EthereumTransactionWrapper::from_string("invalid_hex_data");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode base64: Invalid symbol 95, offset 7.", msg);
        } else {
            panic!("Expected decode error for invalid hex data");
        }

        // Test with malformed hex (odd length)
        let result = EthereumTransactionWrapper::from_string("0x123");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode hex: Odd number of digits", msg);
        } else {
            panic!("Expected decode error for malformed hex (odd length)");
        }

        // Test with valid hex prefix but invalid RLP data
        let result = EthereumTransactionWrapper::from_string("0x1234567890abcdef");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!(
                "Failed to decode transaction: Unexpected type flag. Got 18.",
                msg
            );
        } else {
            panic!("Expected decode error for valid hex prefix but invalid RLP data");
        }

        // Test with valid base64 but invalid RLP data
        let result = EthereumTransactionWrapper::from_string("aGVsbG8gd29ybGQ=");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!(
                "Failed to decode transaction: Unexpected type flag. Got 104.",
                msg
            );
        } else {
            panic!("Expected decode error for valid base64 but invalid RLP data");
        }

        // Test with unknown transaction type
        let unknown_type_tx = "05f86401808504a817c800825208940000000000000000000000000000000000000000880de0b6b3a764000080c0";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", unknown_type_tx));
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!(
                "Failed to decode transaction: Unexpected type flag. Got 5.",
                msg
            );
        } else {
            panic!("Expected decode error for unknown transaction type");
        }

        // Test with corrupted typed transaction (invalid RLP after type byte)
        let corrupted_typed_tx = "02ff";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", corrupted_typed_tx));
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode transaction: input too short", msg);
        } else {
            panic!("Expected decode error for corrupted typed transaction");
        }

        // Test with valid transaction type but insufficient data
        let insufficient_data = "02";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", insufficient_data));
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode transaction: input too short", msg);
        } else {
            panic!("Expected decode error for valid transaction type but insufficient data");
        }

        // Test with whitespace in input (should fail due to invalid format)
        let result = EthereumTransactionWrapper::from_string(" 0x1234 ");
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert_eq!("Failed to decode base64: Invalid symbol 32, offset 0.", msg);
        } else {
            panic!("Expected decode error for whitespace in input");
        }

        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 20_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });
        // Encode the transaction to RLP bytes
        let mut encoded = Vec::new();
        tx.encode_for_signing(&mut encoded);
        let hex_string = format!("0x{}", hex::encode(&encoded));

        // Test parsing the encoded transaction
        let result = EthereumTransactionWrapper::from_string(&hex_string);
        if let Err(res) = result {
            panic!("Failed to parse encoded transaction: {}", res);
        }

        let wrapper = result.unwrap();
        assert_eq!(wrapper.transaction_type(), "Ethereum");

        // Compare the decoded transaction with the original
        if let (TypedTransaction::Legacy(original), TypedTransaction::Legacy(decoded)) =
            (&tx, wrapper.inner())
        {
            assert_eq!(original.chain_id, decoded.chain_id);
            assert_eq!(original.nonce, decoded.nonce);
            assert_eq!(original.gas_price, decoded.gas_price);
            assert_eq!(original.gas_limit, decoded.gas_limit);
            assert_eq!(original.to, decoded.to);
            assert_eq!(original.value, decoded.value);
            assert_eq!(original.input, decoded.input);
        } else {
            panic!("Expected both transactions to be Legacy type");
        }

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

        // Encode EIP-1559 transaction
        let mut eip1559_encoded = Vec::new();
        eip1559_tx.encode_for_signing(&mut eip1559_encoded);
        let eip1559_hex = format!("0x{}", hex::encode(&eip1559_encoded));
        println!("EIP-1559 Hex: {}", eip1559_hex);

        let eip1559_result = EthereumTransactionWrapper::from_string(&eip1559_hex);
        if let Err(res) = eip1559_result {
            panic!("Failed to parse encoded transaction: {}", res);
        }

        let eip1559_wrapper = eip1559_result.unwrap();
        // Compare the decoded EIP-1559 transaction with the original
        if let (TypedTransaction::Eip1559(original), TypedTransaction::Eip1559(decoded)) =
            (&eip1559_tx, eip1559_wrapper.inner())
        {
            assert_eq!(original.chain_id, decoded.chain_id);
            assert_eq!(original.nonce, decoded.nonce);
            assert_eq!(original.gas_limit, decoded.gas_limit);
            assert_eq!(original.max_fee_per_gas, decoded.max_fee_per_gas);
            assert_eq!(
                original.max_priority_fee_per_gas,
                decoded.max_priority_fee_per_gas
            );
            assert_eq!(original.to, decoded.to);
            assert_eq!(original.value, decoded.value);
            assert_eq!(original.access_list, decoded.access_list);
            assert_eq!(original.input, decoded.input);
        } else {
            panic!("Expected both transactions to be EIP-1559 type");
        }

        // Test with EIP-2930 transaction (unsupported)
        let eip2930_encoded = vec![0x01, 0x12, 0x34]; // EIP-2930 type prefix with dummy data
        let eip2930_hex = format!("0x{}", hex::encode(&eip2930_encoded));
        let eip2930_result = EthereumTransactionWrapper::from_string(&eip2930_hex);
        assert!(eip2930_result.is_err());
        if let Err(TransactionParseError::DecodeError(msg)) = eip2930_result {
            assert!(msg.contains("Unsupported variant eip-2930"));
        } else {
            panic!("Expected decode error for unsupported EIP-2930 transaction");
        }

        // Test with EIP-4844 transaction (unsupported)
        let eip4844_encoded = vec![0x03, 0x56, 0x78]; // EIP-4844 type prefix with dummy data
        let eip4844_hex = format!("0x{}", hex::encode(&eip4844_encoded));
        let eip4844_result = EthereumTransactionWrapper::from_string(&eip4844_hex);
        assert!(eip4844_result.is_err());
        if let Err(TransactionParseError::DecodeError(msg)) = eip4844_result {
            assert!(msg.contains("Unsupported variant eip-4844"));
        } else {
            panic!("Expected decode error for unsupported EIP-4844 transaction");
        }

        // Test with EIP-7702 transaction (unsupported)
        let eip7702_encoded = vec![0x04, 0x9a, 0xbc]; // EIP-7702 type prefix with dummy data
        let eip7702_hex = format!("0x{}", hex::encode(&eip7702_encoded));
        let eip7702_result = EthereumTransactionWrapper::from_string(&eip7702_hex);
        assert!(eip7702_result.is_err());
        if let Err(TransactionParseError::DecodeError(msg)) = eip7702_result {
            assert!(msg.contains("Unsupported variant eip-7702"));
        } else {
            panic!("Expected decode error for unsupported EIP-7702 transaction");
        }
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
        let test_tx = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";
        let options = VisualSignOptions::default();
        let tx = EthereumTransactionWrapper::from_string(test_tx).unwrap();

        let result = transaction_to_visual_sign(tx.inner().clone(), options);

        match result {
            Ok(payload) => {
                assert_eq!(payload.title, "Ethereum Transaction");
            }
            Err(e) => {
                eprintln!("Public API failed with error: {:?}", e);
                panic!("Public API should work but got error: {:?}", e);
            }
        }
    }
}
