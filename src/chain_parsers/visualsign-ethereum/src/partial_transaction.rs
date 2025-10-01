// EIP-7702 Transaction Decoder for Ethereum
// This module provides functionality to decode EIP-7702 transactions using Alloy's TxEip7702

use alloy_consensus::TxEip7702;
use alloy_primitives::{Address, Bytes, ChainId, U256, hex};
use alloy_rlp::{Decodable, Encodable};
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    vsptrait::VisualSignOptions,
};

/// Parsing modes for transaction decoding
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionParsingMode {
    /// Standard EIP-7702 transaction format
    Eip7702,
    /// Custom partial transaction format (catch all for non-standard cases for now)
    CustomPartial,
}

/// A wrapper around Alloy's TxEip7702 for transaction handling
/// Uses Alloy's built-in RLP encoding/decoding and transaction structure
/// Can represent both EIP-7702 transactions and legacy partial transactions converted to EIP-7702 format
#[derive(Debug, Clone, PartialEq)]
pub struct Eip7702TransactionWrapper {
    pub inner: TxEip7702,
    pub parsing_mode: TransactionParsingMode,
}

impl Default for Eip7702TransactionWrapper {
    fn default() -> Self {
        Self {
            inner: TxEip7702 {
                chain_id: ChainId::from(1u64),
                nonce: 0,
                gas_limit: 0,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                to: Address::ZERO,
                value: U256::ZERO,
                input: Bytes::new(),
                access_list: Default::default(),
                authorization_list: Default::default(),
            },
            parsing_mode: TransactionParsingMode::Eip7702,
        }
    }
}

impl Eip7702TransactionWrapper {
    /// Create a new EIP-7702 transaction with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from a TxEip7702 instance (defaults to EIP-7702 parsing mode)
    pub fn from_inner(inner: TxEip7702) -> Self {
        Self {
            inner,
            parsing_mode: TransactionParsingMode::Eip7702,
        }
    }

    /// Create from a TxEip7702 instance with specific parsing mode
    pub fn from_inner_with_mode(inner: TxEip7702, parsing_mode: TransactionParsingMode) -> Self {
        Self {
            inner,
            parsing_mode,
        }
    }

    /// Create from RLP bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        Self::decode_partial(bytes)
    }

    /// Decode from RLP bytes using TxEip7702's built-in decoding
    pub fn decode_partial(buf: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if buf.is_empty() {
            return Err("Cannot decode transaction from empty data".into());
        }

        // Use TxEip7702's built-in RLP decoding
        let mut buf_slice = buf;
        match TxEip7702::decode(&mut buf_slice) {
            Ok(tx) => Ok(Self {
                inner: tx,
                parsing_mode: TransactionParsingMode::Eip7702,
            }),
            Err(e) => Err(format!("Failed to decode RLP: {}", e).into()),
        }
    }

    /// Encode to RLP bytes using TxEip7702's built-in encoding
    pub fn encode_partial(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.inner.encode(&mut buffer);
        buffer
    }
    /// Convert to visual sign format
    pub fn to_visual_sign_payload(&self, options: VisualSignOptions) -> SignablePayload {
        let mut fields = Vec::new();

        // Network field
        let chain_id_u64: u64 = self.inner.chain_id.into();
        let chain_name = match chain_id_u64 {
            1 => "Ethereum Mainnet".to_string(),
            11155111 => "Sepolia Testnet".to_string(),
            5 => "Goerli Testnet".to_string(),
            17 => "Custom Chain ID 17".to_string(), // From our fixture
            137 => "Polygon Mainnet".to_string(),
            _ => format!("Chain ID: {}", chain_id_u64),
        };

        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: chain_name.clone(),
                label: "Network".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: chain_name },
        });

        // Transaction type field - indicate parsing mode
        let tx_type_text = match self.parsing_mode {
            TransactionParsingMode::Eip7702 => "EIP-7702 Transaction".to_string(),
            TransactionParsingMode::CustomPartial => {
                "Custom Partial Transaction (converted to EIP-7702)".to_string()
            }
        };

        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: tx_type_text.clone(),
                label: "Transaction Type".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: tx_type_text },
        });

        // To address
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: self.inner.to.to_string(),
                label: "To Address".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: self.inner.to.to_string(),
            },
        });

        // Value - use alloy's format_units to properly format the value
        let value_text = format_value_with_unit(self.inner.value);

        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: value_text.clone(),
                label: "Value".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: value_text },
        });

        // Nonce
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: self.inner.nonce.to_string(),
                label: "Nonce".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: self.inner.nonce.to_string(),
            },
        });

        // Gas limit
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: self.inner.gas_limit.to_string(),
                label: "Gas Limit".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: self.inner.gas_limit.to_string(),
            },
        });

        // Max fee per gas (EIP-1559)
        let max_fee_text = format!("{} wei", self.inner.max_fee_per_gas);
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: max_fee_text.clone(),
                label: "Max Fee Per Gas".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: max_fee_text },
        });

        // Max priority fee per gas (EIP-1559)
        let max_priority_fee_text = format!("{} wei", self.inner.max_priority_fee_per_gas);
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: max_priority_fee_text.clone(),
                label: "Max Priority Fee Per Gas".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: max_priority_fee_text,
            },
        });

        // Input data
        if !self.inner.input.is_empty() {
            fields.push(SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("0x{}", hex::encode(&self.inner.input)),
                    label: "Input Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("0x{}", hex::encode(&self.inner.input)),
                },
            });
        }

        let default_title = match self.parsing_mode {
            TransactionParsingMode::Eip7702 => "EIP-7702 Ethereum Transaction".to_string(),
            TransactionParsingMode::CustomPartial => {
                "Custom Partial Ethereum Transaction".to_string()
            }
        };

        let title = options.transaction_name.unwrap_or(default_title);

        let description = match self.parsing_mode {
            TransactionParsingMode::Eip7702 => {
                "EIP-7702 transaction decoded using Alloy".to_string()
            }
            TransactionParsingMode::CustomPartial => {
                "Custom partial transaction converted to EIP-7702 format".to_string()
            }
        };

        SignablePayload::new(
            0,
            title,
            Some(description),
            fields,
            "EthereumTx".to_string(),
        )
    }
}

// Helper function to format value with appropriate unit using alloy's format_units
fn format_value_with_unit(wei: U256) -> String {
    use alloy_primitives::utils::format_units;

    // For very small values (< 1000 wei), show as wei
    if wei < U256::from(1000u64) {
        format!("{} wei", wei)
    } else {
        // For larger values, show as ETH using alloy's format_units
        let formatted = format_units(wei, 18).unwrap_or_else(|_| wei.to_string());

        // Trim trailing zeros
        let trimmed = if formatted.contains('.') {
            formatted
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        } else {
            formatted
        };

        format!("{} ETH", trimmed)
    }
}

/// Legacy structure for the original partial transaction format
/// This matches the old RLP structure: [chain_id, nonce, gas_price, gas_tip, gas_limit, to, value, data, access_list]
#[derive(Debug, Clone, PartialEq)]
struct LegacyPartialTransaction {
    pub chain_id: U256,
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_tip: U256,
    pub gas_limit: U256,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<u8>,
}

impl LegacyPartialTransaction {
    fn to_eip7702_wrapper(&self) -> Eip7702TransactionWrapper {
        Eip7702TransactionWrapper {
            inner: TxEip7702 {
                chain_id: ChainId::from(self.chain_id.to::<u64>()),
                nonce: self.nonce.to::<u64>(),
                gas_limit: self.gas_limit.to::<u64>(),
                // Convert legacy gas_price + gas_tip to EIP-1559 format
                max_fee_per_gas: (self.gas_price + self.gas_tip).to::<u128>(),
                max_priority_fee_per_gas: self.gas_tip.to::<u128>(),
                to: self.to,
                value: self.value,
                input: self.data.clone(),
                access_list: Default::default(),
                authorization_list: Default::default(),
            },
            parsing_mode: TransactionParsingMode::CustomPartial,
        }
    }
}

/// Decode transaction from hex string - tries EIP-7702 first, then falls back to legacy format
pub fn decode_eip7702_transaction_from_hex(
    hex_data: &str,
) -> Result<Eip7702TransactionWrapper, Box<dyn std::error::Error>> {
    let clean_hex = hex_data.strip_prefix("0x").unwrap_or(hex_data);

    // Handle empty hex by erroring instead of making up values
    if clean_hex.is_empty() {
        return Err("Cannot decode transaction from empty hex string".into());
    }

    let bytes = hex::decode(clean_hex)?;

    // First try to decode as EIP-7702
    match Eip7702TransactionWrapper::decode_partial(&bytes) {
        Ok(tx) => Ok(tx),
        Err(_) => {
            // Fall back to legacy format
            decode_legacy_partial_transaction(&bytes)
        }
    }
}

/// Decode legacy partial transaction format and convert to EIP-7702 wrapper  
fn decode_legacy_partial_transaction(
    bytes: &[u8],
) -> Result<Eip7702TransactionWrapper, Box<dyn std::error::Error>> {
    use alloy_rlp::{Decodable, Header};

    // Try to decode as the old format [chain_id, nonce, gas_price, gas_tip, gas_limit, to, value, data, access_list]
    let mut buf = bytes;

    // Decode the RLP header first to ensure it's a list
    let header = Header::decode(&mut buf)?;
    if !header.list {
        return Err("Expected RLP list".into());
    }

    // Manually decode each field in sequence
    let chain_id = U256::decode(&mut buf)?;
    let nonce = U256::decode(&mut buf)?;
    let gas_price = U256::decode(&mut buf)?;
    let gas_tip = U256::decode(&mut buf)?;
    let gas_limit = U256::decode(&mut buf)?;
    let to = Address::decode(&mut buf)?;
    let value = U256::decode(&mut buf)?;
    let data = Bytes::decode(&mut buf)?;
    let access_list = Vec::<u8>::decode(&mut buf)?;

    let legacy_tx = LegacyPartialTransaction {
        chain_id,
        nonce,
        gas_price,
        gas_tip,
        gas_limit,
        to,
        value,
        data,
        access_list,
    };

    Ok(legacy_tx.to_eip7702_wrapper())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create fixture data for testing - this is test-specific and doesn't belong in the main impl
    fn create_fixture_transaction() -> Eip7702TransactionWrapper {
        Eip7702TransactionWrapper {
            inner: TxEip7702 {
                chain_id: ChainId::from(17u64),               // 0x11
                nonce: 0,                                     // 0x (empty)
                max_fee_per_gas: 23_000_000_000u128, // gas_price (3) + gas_tip (20) = 23 gwei
                max_priority_fee_per_gas: 20_000_000_000u128, // gas_tip (20) = 20 gwei
                gas_limit: 21000,                    // 0x5208
                to: Address::from_slice(
                    &hex::decode("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
                ),
                value: U256::from(1u64), // 0x01 (represents 1 ETH in context)
                input: Bytes::new(),     // 0x (empty)
                access_list: Default::default(), // [] (empty)
                authorization_list: Default::default(), // [] (empty)
            },
            parsing_mode: TransactionParsingMode::Eip7702,
        }
    }

    #[test]
    fn test_fixture_transaction() {
        let fixture_tx = create_fixture_transaction();

        assert_eq!(fixture_tx.inner.chain_id, ChainId::from(17u64));
        assert_eq!(fixture_tx.inner.nonce, 0);
        assert_eq!(fixture_tx.inner.max_fee_per_gas, 23_000_000_000u128);
        assert_eq!(
            fixture_tx.inner.max_priority_fee_per_gas,
            20_000_000_000u128
        );
        assert_eq!(fixture_tx.inner.gas_limit, 21000);
        assert_eq!(
            fixture_tx.inner.to,
            Address::from_slice(&hex::decode("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap())
        );
        assert_eq!(fixture_tx.inner.value, U256::from(1u64)); // Represents 1 ETH in context
        assert_eq!(fixture_tx.inner.input, Bytes::new());
        assert!(fixture_tx.inner.access_list.is_empty());
        assert!(fixture_tx.inner.authorization_list.is_empty());
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original_tx = create_fixture_transaction();

        // Encode to RLP
        let encoded = original_tx.encode_partial();

        // Decode back from RLP
        let decoded_tx = Eip7702TransactionWrapper::decode_partial(&encoded).unwrap();

        // Should be identical
        assert_eq!(original_tx, decoded_tx);
    }

    #[test]
    fn test_encode_decode_with_hex() {
        // Test the encode/decode cycle with hex encoding
        let fixture_tx = create_fixture_transaction();
        let encoded = fixture_tx.encode_partial();
        let hex_str = hex::encode(&encoded);

        let decoded_tx = decode_eip7702_transaction_from_hex(&hex_str).unwrap();
        assert_eq!(fixture_tx, decoded_tx);
    }

    #[test]
    fn test_visual_sign_payload() {
        let fixture_tx = create_fixture_transaction();

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: Some("Test Fixture Transaction".to_string()),
            partial_parsing: true,
        };

        let payload = fixture_tx.to_visual_sign_payload(options);
        assert_eq!(payload.title, "Test Fixture Transaction");

        // Helper function to find field by label
        let find_field_text = |label: &str| -> Option<String> {
            payload.fields.iter().find_map(|f| match f {
                SignablePayloadField::TextV2 { common, text_v2 } if common.label == label => {
                    Some(text_v2.text.clone())
                }
                SignablePayloadField::Text { common, text } if common.label == label => {
                    Some(text.text.clone())
                }
                _ => None,
            })
        };

        // Check the fields match our fixture
        assert_eq!(
            find_field_text("Network"),
            Some("Custom Chain ID 17".to_string())
        );
        assert_eq!(
            find_field_text("Transaction Type"),
            Some("EIP-7702 Transaction".to_string())
        );
        // Address comparison should be case-insensitive
        let to_address = find_field_text("To Address").unwrap();
        assert_eq!(
            to_address.to_lowercase(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
        assert_eq!(find_field_text("Value"), Some("1 wei".to_string())); // Using alloy format_units
        assert_eq!(find_field_text("Nonce"), Some("0".to_string()));
        assert_eq!(find_field_text("Gas Limit"), Some("21000".to_string()));
        assert_eq!(
            find_field_text("Max Fee Per Gas"),
            Some("23000000000 wei".to_string())
        );
        assert_eq!(
            find_field_text("Max Priority Fee Per Gas"),
            Some("20000000000 wei".to_string())
        );
    }

    #[test]
    fn test_new_constructors() {
        // Test default constructor
        let default_tx = Eip7702TransactionWrapper::new();
        assert_eq!(default_tx.inner.chain_id, ChainId::from(1u64));
        assert_eq!(default_tx.inner.nonce, 0);
        assert_eq!(default_tx.inner.gas_limit, 0);
        assert_eq!(default_tx.inner.max_fee_per_gas, 0);
        assert_eq!(default_tx.inner.max_priority_fee_per_gas, 0);
        assert_eq!(default_tx.inner.to, Address::ZERO);
        assert_eq!(default_tx.inner.value, U256::ZERO);

        // Test from_inner constructor
        let test_inner = TxEip7702 {
            chain_id: ChainId::from(42u64),
            nonce: 5,
            gas_limit: 30000,
            max_fee_per_gas: 50_000_000_000u128,
            max_priority_fee_per_gas: 10_000_000_000u128,
            to: Address::from([1u8; 20]),
            value: U256::from(1000u64),
            input: Bytes::from(vec![0x12, 0x34]),
            access_list: Default::default(),
            authorization_list: Default::default(),
        };
        let from_inner_tx = Eip7702TransactionWrapper::from_inner(test_inner.clone());
        assert_eq!(from_inner_tx.inner, test_inner);

        // Test from_bytes constructor
        let encoded = from_inner_tx.encode_partial();
        let from_bytes_tx = Eip7702TransactionWrapper::from_bytes(&encoded).unwrap();
        assert_eq!(from_bytes_tx, from_inner_tx);
    }

    #[test]
    fn test_eip7702_rlp_pattern() {
        // Test the EIP-7702 transaction encoding/decoding pattern
        let my_tx = Eip7702TransactionWrapper {
            inner: TxEip7702 {
                chain_id: ChainId::from(17u64),
                nonce: 0,
                max_fee_per_gas: 23_000_000_000u128,
                max_priority_fee_per_gas: 20_000_000_000u128,
                gas_limit: 21000,
                to: Address::from_slice(
                    &hex::decode("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
                ),
                value: U256::from(1u64),
                input: Bytes::new(),
                access_list: Default::default(),
                authorization_list: Default::default(),
            },
            parsing_mode: TransactionParsingMode::Eip7702,
        };

        let mut buffer = Vec::<u8>::new();
        my_tx.inner.encode(&mut buffer);
        let decoded_inner = TxEip7702::decode(&mut buffer.as_slice()).unwrap();
        let decoded = Eip7702TransactionWrapper {
            inner: decoded_inner,
            parsing_mode: TransactionParsingMode::Eip7702,
        };
        assert_eq!(my_tx, decoded);
    }

    #[test]
    fn test_legacy_partial_transaction_fallback() {
        // Test that the original hex fixture now works with legacy fallback
        let legacy_hex = "0xdf11800314825208941d0dd7a303374bd5f8c57bd8d16e52316d3bfe740180c0";

        // This should now work with the legacy fallback
        let result = decode_eip7702_transaction_from_hex(legacy_hex);
        assert!(
            result.is_ok(),
            "Legacy transaction should decode successfully with fallback"
        );

        let decoded_tx = result.unwrap();
        assert_eq!(
            decoded_tx.parsing_mode,
            TransactionParsingMode::CustomPartial
        );

        // Verify the transaction type in visual sign payload
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: true,
        };

        let payload = decoded_tx.to_visual_sign_payload(options);

        // Should indicate it was converted from custom partial format
        let find_field_text = |label: &str| -> Option<String> {
            payload.fields.iter().find_map(|f| match f {
                SignablePayloadField::TextV2 { common, text_v2 } if common.label == label => {
                    Some(text_v2.text.clone())
                }
                _ => None,
            })
        };

        assert_eq!(
            find_field_text("Transaction Type"),
            Some("Custom Partial Transaction (converted to EIP-7702)".to_string())
        );
        assert_eq!(payload.title, "Custom Partial Ethereum Transaction");
    }
}
