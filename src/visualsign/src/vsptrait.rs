use std::fmt::Debug;
use std::sync::Arc;
use std::any::Any;

use crate::SignablePayload;

pub use crate::errors::{TransactionParseError, VisualSignError};
pub use generated::parser::ChainMetadata;

#[derive(Clone)]
pub struct VisualSignOptions {
    pub decode_transfers: bool,
    pub transaction_name: Option<String>,
    pub metadata: Option<ChainMetadata>,
    pub abi_registry: Option<Arc<dyn Any + Send + Sync>>,
    // Add more options as needed - we can extend this struct later
}

impl Default for VisualSignOptions {
    fn default() -> Self {
        Self {
            decode_transfers: false,
            transaction_name: None,
            metadata: None,
            abi_registry: None,
        }
    }
}

impl Debug for VisualSignOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisualSignOptions")
            .field("decode_transfers", &self.decode_transfers)
            .field("transaction_name", &self.transaction_name)
            .field("metadata", &self.metadata)
            .field("abi_registry", &"<registry>")
            .finish()
    }
}

pub trait VisualSignConverter<T: Transaction> {
    fn to_visual_sign_payload(
        &self,
        transaction: T,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError>;

    /// Convert to VisualSign payload with automatic charset validation
    /// This method should be used instead of to_visual_sign_payload to ensure charset safety
    fn to_validated_visual_sign_payload(
        &self,
        transaction: T,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let payload = self.to_visual_sign_payload(transaction, options)?;
        payload.validate_charset()?;
        Ok(payload)
    }
}

/// Trait for blockchain transactions that can be converted to VisualSign
///
/// # Examples
///
/// ```
/// use visualsign::vsptrait::{Transaction, TransactionParseError};
///
/// #[derive(Debug, Clone)]
/// struct MyTransaction { /* ... */ }
///
/// impl Transaction for MyTransaction {
///     fn from_string(data: &str) -> Result<Self, TransactionParseError> {
///         // Parse transaction from string
///         # Ok(MyTransaction {})
///     }
///
///     fn transaction_type(&self) -> String {
///         "MyBlockchain".to_string()
///     }
/// }
/// ```
pub trait Transaction: Debug + Clone {
    /// Parse a transaction from a string representation (hex, base64, etc.)
    fn from_string(data: &str) -> Result<Self, TransactionParseError>
    where
        Self: Sized;

    /// Get the transaction type name (e.g., "Solana", "Ethereum", "Bitcoin")
    fn transaction_type(&self) -> String;
}

/// Convenience trait for converting from string directly
pub trait VisualSignConverterFromString<T: Transaction>: VisualSignConverter<T> {
    /// Convert a transaction string to a VisualSign payload with charset validation
    fn to_visual_sign_payload_from_string(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let transaction = T::from_string(transaction_data).map_err(VisualSignError::ParseError)?;
        self.to_validated_visual_sign_payload(transaction, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    // Mock transaction implementation
    #[derive(Debug, Clone)]
    struct MockTransaction {
        data: String,
        tx_type: &'static str,
    }

    impl Transaction for MockTransaction {
        fn from_string(data: &str) -> Result<Self, TransactionParseError> {
            if data.starts_with("invalid") {
                return Err(TransactionParseError::InvalidFormat(
                    "Invalid format".to_string(),
                ));
            }

            if data.starts_with("unsupported") {
                return Err(TransactionParseError::UnsupportedVersion(
                    "Unsupported version".to_string(),
                ));
            }

            if data.starts_with("decode_error") {
                return Err(TransactionParseError::DecodeError(
                    "Decode error".to_string(),
                ));
            }

            Ok(MockTransaction {
                data: data.to_string(),
                tx_type: if data.contains("ethereum") {
                    "Ethereum"
                } else {
                    "Solana"
                },
            })
        }

        fn transaction_type(&self) -> String {
            self.tx_type.to_string()
        }
    }

    // Mock converter implementation
    struct MockConverter;

    impl VisualSignConverter<MockTransaction> for MockConverter {
        fn to_visual_sign_payload(
            &self,
            transaction: MockTransaction,
            options: VisualSignOptions,
        ) -> Result<SignablePayload, VisualSignError> {
            if transaction.data.contains("error") {
                return Err(VisualSignError::ConversionError(
                    "Conversion failed".to_string(),
                ));
            }

            if transaction.data.contains("missing") {
                return Err(VisualSignError::MissingData("Missing data".to_string()));
            }

            if transaction.data.contains("decode_fail") {
                return Err(VisualSignError::DecodeError("Decode error".to_string()));
            }

            // Create a simple payload for testing
            let mut title = "Transaction".to_string();
            if let Some(name) = options.transaction_name {
                title = name;
            }

            // Include transaction details based on options
            let mut fields = vec![SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: transaction.transaction_type().to_string(),
                    label: "Network".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: transaction.transaction_type().to_string(),
                },
            }];

            if options.decode_transfers {
                fields.push(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Transfer details".to_string(),
                        label: "Transfer".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Transfer details".to_string(),
                    },
                });
            }

            Ok(SignablePayload::new(
                0,
                title,
                None,
                fields,
                "Test".to_string(),
            ))
        }
    }

    impl VisualSignConverterFromString<MockTransaction> for MockConverter {
        fn to_visual_sign_payload_from_string(
            &self,
            transaction_data: &str,
            options: VisualSignOptions,
        ) -> Result<SignablePayload, VisualSignError> {
            let transaction = MockTransaction::from_string(transaction_data)?;
            self.to_visual_sign_payload(transaction, options)
        }
    }

    #[test]
    fn test_transaction_from_string_success() {
        let result = MockTransaction::from_string("valid_transaction");
        assert!(result.is_ok());
        let tx = result.unwrap();
        assert_eq!(tx.data, "valid_transaction");
        assert_eq!(tx.transaction_type(), "Solana");

        let result = MockTransaction::from_string("valid_ethereum_tx");
        assert!(result.is_ok());
        let tx = result.unwrap();
        assert_eq!(tx.transaction_type(), "Ethereum");
    }

    #[test]
    fn test_transaction_from_string_errors() {
        // Test invalid format error
        let result = MockTransaction::from_string("invalid_tx");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransactionParseError::InvalidFormat(_) => (),
            _ => panic!("Expected InvalidFormat error"),
        }

        // Test unsupported version error
        let result = MockTransaction::from_string("unsupported_tx");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransactionParseError::UnsupportedVersion(_) => (),
            _ => panic!("Expected UnsupportedVersion error"),
        }

        // Test decode error
        let result = MockTransaction::from_string("decode_error_tx");
        assert!(result.is_err());
        match result.unwrap_err() {
            TransactionParseError::DecodeError(_) => (),
            _ => panic!("Expected DecodeError error"),
        }
    }

    #[test]
    fn test_visual_sign_converter_success() {
        let converter = MockConverter;
        let transaction = MockTransaction {
            data: "test_tx".to_string(),
            tx_type: "Solana",
        };

        // Test with default options
        let result =
            converter.to_visual_sign_payload(transaction.clone(), VisualSignOptions::default());
        assert!(result.is_ok());
        let payload = result.unwrap();
        assert_eq!(payload.title, "Transaction");
        assert_eq!(payload.fields.len(), 1); // Only network field

        // Test with custom options
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: Some("Custom Transaction".to_string()),
            metadata: None,
        };

        let result = converter.to_visual_sign_payload(transaction, options);
        assert!(result.is_ok());
        let payload = result.unwrap();
        assert_eq!(payload.title, "Custom Transaction");
        assert_eq!(payload.fields.len(), 2); // Network and transfer fields
    }

    #[test]
    fn test_visual_sign_converter_errors() {
        let converter = MockConverter;

        // Test conversion error
        let transaction = MockTransaction {
            data: "error_tx".to_string(),
            tx_type: "Solana",
        };
        let result = converter.to_visual_sign_payload(transaction, VisualSignOptions::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            VisualSignError::ConversionError(_) => (),
            _ => panic!("Expected ConversionError"),
        }

        // Test missing data error
        let transaction = MockTransaction {
            data: "missing_data".to_string(),
            tx_type: "Solana",
        };
        let result = converter.to_visual_sign_payload(transaction, VisualSignOptions::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            VisualSignError::MissingData(_) => (),
            _ => panic!("Expected MissingData error"),
        }

        // Test decode error
        let transaction = MockTransaction {
            data: "decode_fail".to_string(),
            tx_type: "Solana",
        };
        let result = converter.to_visual_sign_payload(transaction, VisualSignOptions::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            VisualSignError::DecodeError(_) => (),
            _ => panic!("Expected DecodeError error"),
        }
    }

    #[test]
    fn test_visual_sign_converter_from_string() {
        let converter = MockConverter;

        // Test successful conversion
        let result = converter
            .to_visual_sign_payload_from_string("valid_transaction", VisualSignOptions::default());
        assert!(result.is_ok());

        // Test parse error
        let result = converter
            .to_visual_sign_payload_from_string("invalid_tx", VisualSignOptions::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            VisualSignError::ParseError(_) => (),
            err => panic!("Expected ParseError, got: {err:?}"),
        }

        // Test conversion error after successful parse
        let result = converter
            .to_visual_sign_payload_from_string("valid_error_tx", VisualSignOptions::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            VisualSignError::ConversionError(_) => (),
            err => panic!("Expected ConversionError, got: {err:?}"),
        }
    }

    #[test]
    fn test_options_default() {
        let options = VisualSignOptions::default();
        assert!(!options.decode_transfers);
        assert!(options.transaction_name.is_none());
    }
}
