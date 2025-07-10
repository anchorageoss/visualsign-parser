use alloy_consensus::{Transaction as _, TypedTransaction};
use alloy_primitives::U256;
use alloy_rlp::Decodable;
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use hex;
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

// Helper function to format wei to ether
fn format_ether(wei: U256) -> String {
    let eth_in_wei = U256::from(1_000_000_000_000_000_000u64); // 10^18
    let eth_part = wei / eth_in_wei;
    let wei_part = wei % eth_in_wei;

    if wei_part == U256::ZERO {
        format!("{}", eth_part)
    } else {
        // Convert to decimal representation
        let decimal_str = format!("{:018}", wei_part);
        let trimmed = decimal_str.trim_end_matches('0');
        if trimmed.is_empty() {
            format!("{}", eth_part)
        } else {
            format!("{}.{}", eth_part, trimmed)
        }
    }
}

// Helper function to format wei to gwei
fn format_gwei(wei: U256) -> String {
    let gwei_in_wei = U256::from(1_000_000_000u64); // 10^9
    let gwei_part = wei / gwei_in_wei;
    let remainder = wei % gwei_in_wei;

    if remainder == U256::ZERO {
        format!("{}", gwei_part)
    } else {
        let decimal_str = format!("{:09}", remainder);
        let trimmed = decimal_str.trim_end_matches('0');
        format!("{}.{}", gwei_part, trimmed)
    }
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
        let payload = convert_to_visual_sign_payload(
            transaction,
            options.decode_transfers,
            options.transaction_name,
        );
        Ok(payload)
    }
}

impl VisualSignConverterFromString<EthereumTransactionWrapper> for EthereumVisualSignConverter {}

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

    // Check if it's a typed transaction (EIP-2718)
    // Typed transactions start with a transaction type byte (0x01, 0x02, 0x03)
    if bytes.is_empty() || bytes[0] > 0x7f {
        // This is a legacy transaction (pre-EIP-2718)
        // Legacy transactions are RLP-encoded directly without a type prefix
        let tx = alloy_consensus::TxLegacy::decode(&mut bytes.as_slice())
            .map_err(|e| format!("Failed to decode legacy transaction: {}", e))?;
        return Ok(TypedTransaction::Legacy(tx));
    }
    match bytes[0] {
        0x01 => {
            // EIP-2930: Optional access lists
            let tx = alloy_consensus::TxEip2930::decode(&mut &bytes[1..])
                .map_err(|e| format!("Failed to decode EIP-2930 transaction: {}", e))?;
            return Ok(TypedTransaction::Eip2930(tx));
        }
        0x02 => {
            // EIP-1559: Fee market change
            let tx = alloy_consensus::TxEip1559::decode(&mut &bytes[1..])
                .map_err(|e| format!("Failed to decode EIP-1559 transaction: {}", e))?;
            return Ok(TypedTransaction::Eip1559(tx));
        }
        0x03 => {
            // EIP-4844: Blob transactions
            let tx = alloy_consensus::TxEip4844::decode(&mut &bytes[1..])
                .map_err(|e| format!("Failed to decode EIP-4844 transaction: {}", e))?;
            return Ok(TypedTransaction::Eip4844(
                alloy_consensus::TxEip4844Variant::TxEip4844(tx),
            ));
        }
        0x04 => {
            // EIP-7702: Set EOA account code
            let tx = alloy_consensus::TxEip7702::decode(&mut &bytes[1..])
                .map_err(|e| format!("Failed to decode EIP-7702 transaction: {}", e))?;
            return Ok(TypedTransaction::Eip7702(tx));
        }
        _ => {
            return Err(format!("Unknown transaction type: 0x{:02x}", bytes[0]).into());
        }
    }
}

fn convert_to_visual_sign_payload(
    transaction: TypedTransaction,
    decode_transfers: bool,
    title: Option<String>,
) -> SignablePayload {
    let mut fields = vec![
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: "Ethereum".to_string(),
                label: "Network".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: "Ethereum".to_string(),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{:?}", transaction.to()),
                label: "To".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{:?}", transaction.to()),
            },
        },
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
    ];

    // Add gas price field (handling different transaction types)
    let gas_price_text = match &transaction {
        TypedTransaction::Legacy(tx) => {
            format!("{} gwei", format_gwei(U256::from(tx.gas_price)))
        }
        TypedTransaction::Eip2930(tx) => {
            format!("{} gwei", format_gwei(U256::from(tx.gas_price)))
        }
        TypedTransaction::Eip1559(tx) => {
            format!("{} gwei", format_gwei(U256::from(tx.max_fee_per_gas)))
        }
        TypedTransaction::Eip4844(tx) => match tx {
            alloy_consensus::TxEip4844Variant::TxEip4844(inner_tx) => {
                format!("{} gwei", format_gwei(U256::from(inner_tx.max_fee_per_gas)))
            }
            alloy_consensus::TxEip4844Variant::TxEip4844WithSidecar(sidecar_tx) => {
                format!(
                    "{} gwei",
                    format_gwei(U256::from(sidecar_tx.tx.max_fee_per_gas))
                )
            }
        },
        TypedTransaction::Eip7702(tx) => {
            format!("{} gwei", format_gwei(U256::from(tx.max_fee_per_gas)))
        }
    };

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: gas_price_text.clone(),
            label: "Gas Price".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: gas_price_text,
        },
    });

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

    if decode_transfers {
        // Add ERC-20 token transfer parsing here
        if let Some(decoded_transfer) = decode_erc20_transfer(input) {
            fields.push(SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "ERC-20 Transfer: {} to {}",
                        decoded_transfer.amount, decoded_transfer.recipient
                    ),
                    label: "Token Transfer".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!(
                        "Amount: {}\nRecipient: {}",
                        decoded_transfer.amount, decoded_transfer.recipient
                    ),
                },
            });
        }
    }

    let title = title.unwrap_or_else(|| "Ethereum Transaction".to_string());
    SignablePayload::new(0, title, None, fields, "EthereumTx".to_string())
}

// Helper struct for ERC-20 transfers
#[derive(Debug)]
struct Erc20Transfer {
    recipient: String,
    amount: String,
}

fn decode_erc20_transfer(input: &[u8]) -> Option<Erc20Transfer> {
    // ERC-20 transfer function signature: transfer(address,uint256)
    const TRANSFER_SELECTOR: &[u8] = &[0xa9, 0x05, 0x9c, 0xbb];

    if input.len() >= 68 && input[0..4] == *TRANSFER_SELECTOR {
        let recipient = format!("0x{}", hex::encode(&input[16..36]));
        let amount_bytes = &input[36..68];
        let amount = U256::from_be_bytes(amount_bytes.try_into().unwrap_or([0u8; 32]));

        Some(Erc20Transfer {
            recipient,
            amount: amount.to_string(),
        })
    } else {
        None
    }
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
    use alloy_consensus::{TxLegacy, TypedTransaction};
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

        // Check that all expected fields are present
        assert!(payload.fields.iter().any(|f| f.label() == "Network"));
        assert!(payload.fields.iter().any(|f| f.label() == "To"));
        assert!(payload.fields.iter().any(|f| f.label() == "Value"));
        assert!(payload.fields.iter().any(|f| f.label() == "Gas Limit"));
        assert!(payload.fields.iter().any(|f| f.label() == "Gas Price"));
        assert!(payload.fields.iter().any(|f| f.label() == "Nonce"));

        // Check specific field values
        let to_field = payload.fields.iter().find(|f| f.label() == "To").unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = to_field {
            assert!(
                text_v2
                    .text
                    .contains("0x000000000000000000000000000000000000dead")
            );
        }

        let value_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Value")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = value_field {
            assert!(text_v2.text.contains("1"));
            assert!(text_v2.text.contains("ETH"));
        }

        let nonce_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Nonce")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = nonce_field {
            assert_eq!(text_v2.text, "42");
        }

        // Check title and type
        assert_eq!(payload.title, "Ethereum Transaction");
        assert_eq!(payload.payload_type, "EthereumTx");
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
    fn test_transaction_with_erc20_transfer() {
        // Create ERC-20 transfer call data
        let mut input_data = vec![0xa9, 0x05, 0x9c, 0xbb]; // transfer function selector
        input_data.extend_from_slice(&[0u8; 12]); // padding
        input_data
            .extend_from_slice(&hex::decode("1234567890123456789012345678901234567890").unwrap()); // recipient address

        // Convert amount to 32-byte big-endian representation
        let amount = U256::from(1000000000000000000u64);
        let amount_bytes = amount.to_be_bytes::<32>();
        input_data.extend_from_slice(&amount_bytes); // amount (1 token with 18 decimals)

        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 1,
            gas_price: 1_000_000_000u128,
            gas_limit: 50000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::from(input_data),
        });

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        // Check that token transfer field is present
        assert!(payload.fields.iter().any(|f| f.label() == "Token Transfer"));
        let transfer_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Token Transfer")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = transfer_field {
            assert!(text_v2.text.contains("1000000000000000000"));
            assert!(
                text_v2
                    .text
                    .contains("0x1234567890123456789012345678901234567890")
            );
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
    fn test_decode_erc20_transfer() {
        // Valid ERC-20 transfer data
        let mut input_data = vec![0xa9, 0x05, 0x9c, 0xbb]; // transfer function selector
        input_data.extend_from_slice(&[0u8; 12]); // padding
        input_data
            .extend_from_slice(&hex::decode("1234567890123456789012345678901234567890").unwrap()); // recipient

        // Convert amount to 32-byte big-endian representation
        let amount = U256::from(1000);
        let amount_bytes = amount.to_be_bytes::<32>();
        input_data.extend_from_slice(&amount_bytes); // amount

        let result = decode_erc20_transfer(&input_data);
        assert!(result.is_some());
        let transfer = result.unwrap();
        assert_eq!(
            transfer.recipient,
            "0x1234567890123456789012345678901234567890"
        );
        assert_eq!(transfer.amount, "1000");

        // Invalid data (too short)
        let short_data = vec![0xa9, 0x05, 0x9c, 0xbb, 0x12];
        assert!(decode_erc20_transfer(&short_data).is_none());

        // Invalid function selector
        let invalid_selector = vec![0x00, 0x00, 0x00, 0x00];
        assert!(decode_erc20_transfer(&invalid_selector).is_none());
    }

    #[test]
    fn test_transaction_wrapper_from_string() {
        // Test with invalid hex data
        let result = EthereumTransactionWrapper::from_string("invalid_hex_data");
        assert!(result.is_err());

        // Test with empty string
        let result = EthereumTransactionWrapper::from_string("");
        assert!(result.is_err());

        // Test with malformed hex (odd length)
        let result = EthereumTransactionWrapper::from_string("0x123");
        assert!(result.is_err());

        // Test with valid hex prefix but invalid RLP data
        let result = EthereumTransactionWrapper::from_string("0x1234567890abcdef");
        assert!(result.is_err());

        // Test with valid base64 but invalid RLP data
        let result = EthereumTransactionWrapper::from_string("aGVsbG8gd29ybGQ=");
        assert!(result.is_err());

        // Test with unknown transaction type
        let unknown_type_tx = "05f86401808504a817c800825208940000000000000000000000000000000000000000880de0b6b3a764000080c0";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", unknown_type_tx));
        assert!(result.is_err());
        if let Err(TransactionParseError::DecodeError(msg)) = result {
            assert!(msg.contains("Unknown transaction type"));
        } else {
            panic!("Expected decode error for unknown transaction type");
        }

        // Test with corrupted typed transaction (invalid RLP after type byte)
        let corrupted_typed_tx = "02ff";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", corrupted_typed_tx));
        assert!(result.is_err());

        // Test with valid transaction type but insufficient data
        let insufficient_data = "02";
        let result = EthereumTransactionWrapper::from_string(&format!("0x{}", insufficient_data));
        assert!(result.is_err());

        // Test with whitespace in input (should fail due to invalid format)
        let result = EthereumTransactionWrapper::from_string(" 0x1234 ");
        assert!(result.is_err());

        // For successful parsing tests, we'll create transactions directly rather than parsing RLP
        // since creating valid RLP data is complex and error-prone
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 20_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let wrapper = EthereumTransactionWrapper::new(tx);
        assert_eq!(wrapper.transaction_type(), "Ethereum");

        // Note: Real RLP parsing tests are omitted due to complexity of creating valid RLP data
        // The decode_transaction function is tested separately with actual network data
        println!("Note: Full RLP parsing tests require valid network transaction data");
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
}
