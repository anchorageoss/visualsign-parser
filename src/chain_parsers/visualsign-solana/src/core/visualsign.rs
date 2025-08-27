use crate::core::instructions;
use base64::{self, Engine};
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

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
            &transaction,
            options.decode_transfers,
            options.transaction_name,
        );

        payload
    }
}

impl VisualSignConverterFromString<SolanaTransactionWrapper> for SolanaVisualSignConverter {}

/// Public API function for ease of use
pub fn transaction_to_visual_sign(
    transaction: SolanaTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SolanaVisualSignConverter
        .to_visual_sign_payload(SolanaTransactionWrapper::new(transaction), options)
}

/// Public API function for string-based transactions
pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SolanaVisualSignConverter.to_visual_sign_payload_from_string(transaction_data, options)
}

/// Convert Solana transaction to visual sign payload
fn convert_to_visual_sign_payload(
    transaction: &SolanaTransaction,
    decode_transfers: bool,
    title: Option<String>,
) -> Result<SignablePayload, VisualSignError> {
    let message = &transaction.message;
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
            text_v2: visualsign::SignablePayloadFieldTextV2 {
                text: "Solana".to_string(),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: account_keys.join(", "),
                label: "Account Keys".to_string(),
            },
            text_v2: visualsign::SignablePayloadFieldTextV2 {
                text: account_keys.join(", "),
            },
        },
    ];

    if decode_transfers {
        let transfer_fields = instructions::decode_transfers(transaction)?;
        fields.extend(
            transfer_fields
                .iter()
                .map(|e| e.signable_payload_field.clone()),
        );
    }

    // Process instructions with visualizers
    fields.extend(
        instructions::decode_instructions(transaction)?
            .iter()
            .map(|e| e.signable_payload_field.clone()),
    );

    Ok(SignablePayload::new(
        0,
        title.unwrap_or_else(|| "Solana Transaction".to_string()),
        None,
        fields,
        "SolanaTx".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::payload_from_b64;
    use crate::utils::create_transaction_with_empty_signatures;

    #[test]
    fn test_solana_transaction_to_vsp() {
        // This was generated using the Solana CLI using solana transfer --sign-only which only prints message, that needs to be wrapped into a transaction
        // Same as the test fixture used for integration as a baseline
        let solana_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let solana_transfer_transaction =
            create_transaction_with_empty_signatures(solana_transfer_message);
        let payload = payload_from_b64(&solana_transfer_transaction);
        assert_eq!(payload.title, "Solana Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "SolanaTx");

        assert!(!payload.fields.is_empty());

        let network_field = payload.fields.iter().find(|f| f.label() == "Network");
        assert!(network_field.is_some());
        assert_eq!(
            network_field.unwrap().fallback_text(),
            &"Solana".to_string()
        );

        let json_result = payload.to_json();
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_solana_transaction_trait() {
        let solana_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let solana_transfer_transaction =
            create_transaction_with_empty_signatures(solana_transfer_message);
        let result = SolanaTransactionWrapper::from_string(&solana_transfer_transaction);
        assert!(result.is_ok());

        let solana_tx = result.unwrap();
        assert_eq!(solana_tx.transaction_type(), "Solana");

        let invalid_result = SolanaTransactionWrapper::from_string("invalid_data");
        assert!(invalid_result.is_err());
    }
}
