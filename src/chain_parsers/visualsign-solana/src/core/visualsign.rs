use crate::core::instructions;
use base64::{self, Engine};
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::{
    SignablePayload, SignablePayloadField,
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
    let mut fields: Vec<SignablePayloadField> = Vec::new();

    if decode_transfers {
        fields.extend(
            instructions::decode_transfers(transaction)?
                .iter()
                .map(|e| e.signable_payload_field.clone()),
        );
    }

    fields.extend(
        instructions::decode_instructions(transaction)?
            .iter()
            .map(|e| e.signable_payload_field.clone()),
    );
    // TODO: details - this is where "advanced stuff would go like account keys"
    Ok(SignablePayload::new(
        0,
        title.unwrap_or_else(|| "Solana Transaction".to_string()),
        None,
        fields,
        "Solana".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solana_transaction_to_vsp() {
        let test_data = "your_test_data_here";
    }
}
