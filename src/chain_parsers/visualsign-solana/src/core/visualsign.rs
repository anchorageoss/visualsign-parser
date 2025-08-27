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

    #[test]
    fn test_jupiter_swap_transaction() {
        // Jupiter swap transaction from the user's request
        let jupiter_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAsTTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ6653iXpBtBVMUA2+7hURKVHSEiGP6Bzz+71DafYBHQDv0Yk27V9AGBuUCokgwtdJtHGjOn65hFbpKYxFjpOxf9DslqNk9ntU1o905D8G/f/M/gGJfV/szOEdGlj8ByB4ydCgh9JdZoBmFC/1V+60NB9JdEtwXur6E410yCBDwODn7a9i8ySuhrG7m4UOmmngOd7rrj0EIP/mIOo3poMglc7k/piKlm7+u7deeb1LQ3/H1gPv54+BUArFsw2O5lY54pz/YD6rtbZ/BQGLaOTytSS3SHI51lpsQDqNm8IHuyTAFQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwTp4S+8hOgmyTLM6eJkDM4VWQwcYnOwklcIujuFILC8BpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqYb8H//NLjVx31IUdFMPpkUf0008tghSu5vUckZpELeujJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FmycNZ/qYxRzwITBRNYliuvNXQr7VnJ2URenA0MhcfNkbQ/+if11/ZKdMCbHylYed5LCas238ndUUsyGqezjOXo/NFB6YMsrxCtkXSVyg8nG1spPNRwJ+pzcAftQOs5oL2MaEXlNY7kQGEFwqYqsAepz7QXX/3fSFmPGjLpqakIxwYJAAUCQA0DAA8GAAIADAgNAQEIAgACDAIAAACghgEAAAAAAA0BAgERChsNAAIDChIKEQoLBA4BBQIDEgwGCwANDRALBwoj5RfLl3rjrSoBAAAAJmQAAaCGAQAAAAAAkz4BAAAAAAAyAAANAwIAAAEJ";

        let solana_tx_result = SolanaTransactionWrapper::from_string(&jupiter_transaction);
        assert!(solana_tx_result.is_ok());

        let solana_tx = solana_tx_result.unwrap();

        // Convert to VisualSign payload using the converter
        let payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
            solana_tx,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: Some("Solana Transaction".to_string()),
            },
        );

        if let Err(ref e) = payload_result {
            println!("Error converting to payload: {:?}", e);
        }
        assert!(payload_result.is_ok());

        let payload = payload_result.unwrap();

        // Verify basic payload properties
        assert_eq!(payload.title, "Solana Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "SolanaTx");
        assert!(!payload.fields.is_empty());

        // Convert to JSON and verify structure
        let json_result = payload.to_json();
        assert!(json_result.is_ok());

        let json_value: serde_json::Value = serde_json::from_str(&json_result.unwrap()).unwrap();

        // Verify expected JSON structure using serde_json::json! macro for comparison
        let expected_structure = serde_json::json!({
            "Title": "Solana Transaction",
            "Version": "0",
            "PayloadType": "SolanaTx"
        });

        assert_eq!(json_value["Title"], expected_structure["Title"]);
        assert_eq!(json_value["Version"], expected_structure["Version"]);
        assert_eq!(json_value["PayloadType"], expected_structure["PayloadType"]);

        // Verify that fields array exists and is not empty
        assert!(json_value["Fields"].is_array());
        let fields = json_value["Fields"].as_array().unwrap();
        assert!(!fields.is_empty());

        // Look for Jupiter-related content in the fields
        let _fields_json = serde_json::to_string(&fields).unwrap();

        // Check for presence of Jupiter program ID or swap-related content
        let has_jupiter_content = fields.iter().any(|field| {
            let field_str = serde_json::to_string(field).unwrap_or_default();
            field_str.contains("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
                || field_str.contains("Jupiter")
                || field_str.contains("swap")
                || field_str.contains("Swap")
        });

        // Verify we found Jupiter content
        assert!(has_jupiter_content, "Should contain Jupiter swap content");

        // Note: This test verifies the transaction can be parsed without errors
        // The exact Jupiter swap detection depends on the instruction data parsing
        println!(
            "✅ Jupiter transaction parsed successfully with {} fields",
            fields.len()
        );
        println!("✅ Contains Jupiter content: {}", has_jupiter_content);
    }
}
