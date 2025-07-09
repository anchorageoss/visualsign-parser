use base64::engine::Engine;
use ethereum_types::U256;
use ethers::types::Transaction as EthereumTransaction;
use ethers::utils::rlp;
use hex;
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

/// Wrapper around Ethereum's transaction type that implements the Transaction trait
#[derive(Debug, Clone)]
pub struct EthereumTransactionWrapper {
    transaction: EthereumTransaction,
}

impl Transaction for EthereumTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        // Detect if format is base64 or hex
        let format = visualsign::encodings::SupportedEncodings::detect(data);

        let transaction = decode_transaction(data, format)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;

        Ok(Self { transaction })
    }

    fn transaction_type(&self) -> String {
        "Ethereum".to_string()
    }
}

impl EthereumTransactionWrapper {
    pub fn new(transaction: EthereumTransaction) -> Self {
        Self { transaction }
    }

    pub fn inner(&self) -> &EthereumTransaction {
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

        // Convert the transaction to a VisualSign payload
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
) -> Result<EthereumTransaction, Box<dyn std::error::Error>> {
    let bytes = match encodings {
        SupportedEncodings::Base64 => {
            base64::engine::general_purpose::STANDARD.decode(raw_transaction)?
        }
        SupportedEncodings::Hex => hex::decode(raw_transaction)?,
    };

    // Decode RLP-encoded Ethereum transaction
    let transaction: EthereumTransaction = rlp::decode(&bytes)?;
    Ok(transaction)
}

fn convert_to_visual_sign_payload(
    transaction: EthereumTransaction,
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
                fallback_text: format!("{:?}", transaction.to),
                label: "To".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{:?}", transaction.to),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} ETH", ethers::utils::format_ether(transaction.value)),
                label: "Value".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{} ETH", ethers::utils::format_ether(transaction.value)),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{}", transaction.gas),
                label: "Gas Limit".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{}", transaction.gas),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "{} gwei",
                    ethers::utils::format_units(transaction.gas_price.unwrap_or_default(), "gwei")
                        .unwrap_or_default()
                ),
                label: "Gas Price".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!(
                    "{} gwei",
                    ethers::utils::format_units(transaction.gas_price.unwrap_or_default(), "gwei")
                        .unwrap_or_default()
                ),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{}", transaction.nonce),
                label: "Nonce".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{}", transaction.nonce),
            },
        },
    ];

    // Add contract call data if present
    if !transaction.input.is_empty() {
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("0x{}", hex::encode(&transaction.input)),
                label: "Input Data".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("0x{}", hex::encode(&transaction.input)),
            },
        });
    }

    if decode_transfers {
        // Add ERC-20 token transfer parsing here
        // This would require parsing the input data for ERC-20 function calls
        if let Some(decoded_transfer) = decode_erc20_transfer(&transaction.input) {
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
        let amount = U256::from_big_endian(amount_bytes);

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
    transaction: EthereumTransaction,
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
    use ethers::types::{H160, H256, U256};

    #[test]
    fn test_transaction_to_visual_sign_basic() {
        // Create a dummy Ethereum transaction
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(42),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(
                "0x000000000000000000000000000000000000dead"
                    .parse()
                    .unwrap(),
            ),
            value: U256::from(1000000000000000000u64), // 1 ETH
            gas_price: Some(U256::from(20000000000u64)), // 20 gwei
            gas: U256::from(21000),
            input: vec![].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

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

        let gas_limit_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Gas Limit")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = gas_limit_field {
            assert_eq!(text_v2.text, "21000");
        }

        let gas_price_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Gas Price")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = gas_price_field {
            assert!(text_v2.text.contains("20"));
            assert!(text_v2.text.contains("gwei"));
        }

        // Check title and type
        assert_eq!(payload.title, "Ethereum Transaction");
        assert_eq!(payload.payload_type, "EthereumTx");
    }

    #[test]
    fn test_transaction_with_input_data() {
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(1),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: Some(U256::from(1000000000u64)),
            gas: U256::from(50000),
            input: vec![0x12, 0x34, 0x56, 0x78].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

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
        let mut amount_bytes = [0u8; 32];
        amount.to_big_endian(&mut amount_bytes);
        input_data.extend_from_slice(&amount_bytes); // amount (1 token with 18 decimals)

        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(1),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: Some(U256::from(1000000000u64)),
            gas: U256::from(50000),
            input: input_data.into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

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
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(0),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: Some(U256::from(1000000000u64)),
            gas: U256::from(21000),
            input: vec![].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

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
        let mut amount_bytes = [0u8; 32];
        amount.to_big_endian(&mut amount_bytes);
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
        // Test with mock data - in practice you'd need valid RLP-encoded transaction
        let result = EthereumTransactionWrapper::from_string("invalid_data");
        assert!(result.is_err());
    }

    #[test]
    fn test_transaction_wrapper_type() {
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(0),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: Some(U256::from(1000000000u64)),
            gas: U256::from(21000),
            input: vec![].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

        let wrapper = EthereumTransactionWrapper::new(tx);
        assert_eq!(wrapper.transaction_type(), "Ethereum");
    }

    #[test]
    fn test_zero_value_transaction() {
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(0),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: Some(U256::from(1000000000u64)),
            gas: U256::from(21000),
            input: vec![].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

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
    fn test_transaction_without_gas_price() {
        let tx = EthereumTransaction {
            hash: H256::zero(),
            nonce: U256::from(0),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: H160::zero(),
            to: Some(H160::zero()),
            value: U256::zero(),
            gas_price: None,
            gas: U256::from(21000),
            input: vec![].into(),
            v: ethers::types::U64::from(0),
            r: U256::from(0),
            s: U256::from(0),
            transaction_type: None,
            access_list: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            chain_id: None,
            other: Default::default(),
        };

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        assert!(payload.fields.iter().any(|f| f.label() == "To"));
        assert!(payload.fields.iter().any(|f| f.label() == "Value"));
    }
}
