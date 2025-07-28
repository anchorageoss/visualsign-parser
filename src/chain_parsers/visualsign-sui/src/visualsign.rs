use crate::commands::{CoinObject, TransferInfo, detect_transfer_from_transaction};
use crate::field_helpers::{
    create_address_field, create_amount_field, create_raw_data_field, create_simple_text_field,
    create_text_field,
};
use crate::module_resolver::SuiModuleResolver;

use base64::Engine;

use move_bytecode_utils::module_cache::SyncModuleCache;

use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::gas_coin::MIST_PER_SUI;
use sui_types::transaction::{SenderSignedData, TransactionData};

use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

/// Wrapper around Sui's transaction type that implements the Transaction trait
#[derive(Debug, Clone)]
pub struct SuiTransactionWrapper {
    transaction: TransactionData,
}

impl Transaction for SuiTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        let format = SupportedEncodings::detect(data);

        let transaction = decode_transaction(data, format)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;

        Ok(Self { transaction })
    }

    fn transaction_type(&self) -> String {
        "Sui".to_string()
    }
}

impl SuiTransactionWrapper {
    /// Create a new SuiTransactionWrapper
    pub fn new(transaction: TransactionData) -> Self {
        Self { transaction }
    }

    /// Get a reference to the inner transaction
    pub fn inner(&self) -> &TransactionData {
        &self.transaction
    }
}

/// Converter that knows how to format Sui transactions for VisualSign
pub struct SuiVisualSignConverter;

impl VisualSignConverter<SuiTransactionWrapper> for SuiVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: SuiTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let transaction = transaction_wrapper.inner();

        convert_to_visual_sign_payload(
            transaction,
            options.decode_transfers,
            options.transaction_name,
        )
    }
}

impl VisualSignConverterFromString<SuiTransactionWrapper> for SuiVisualSignConverter {}

/// Decode a transaction from string format
pub(crate) fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<TransactionData, Box<dyn std::error::Error>> {
    if raw_transaction.is_empty() {
        return Err("Transaction is empty".into());
    }

    let bytes = match encodings {
        SupportedEncodings::Base64 => {
            base64::engine::general_purpose::STANDARD.decode(raw_transaction)?
        }
        SupportedEncodings::Hex => hex::decode(raw_transaction)?,
    };

    if let Ok(sender_signed_data) = bcs::from_bytes::<SenderSignedData>(&bytes) {
        return Ok(sender_signed_data.transaction_data().clone());
    }

    if let Ok(transaction_data) = bcs::from_bytes::<TransactionData>(&bytes) {
        return Ok(transaction_data);
    }

    Err("Unable to decode transaction data as either SenderSignedData or TransactionData".into())
}

/// Convert Sui transaction to visual sign payload
fn convert_to_visual_sign_payload(
    transaction: &TransactionData,
    decode_transfers: bool,
    title: Option<String>,
) -> Result<SignablePayload, VisualSignError> {
    let block_data = SuiTransactionBlockData::try_from_with_module_cache(
        transaction.clone(),
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .map_err(|e| VisualSignError::ParseError(TransactionParseError::DecodeError(e.to_string())))?;

    let mut fields = vec![create_simple_text_field("Network", "Sui Network")];

    if decode_transfers {
        add_transfer_preview_layouts(&mut fields, &block_data);
    }

    add_transaction_details_preview_layout(&mut fields, transaction, &block_data);

    let title = title.unwrap_or_else(|| determine_transaction_type_string(&block_data));
    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "Sui".to_string(),
    ))
}

/// Add transfer information using preview layout
fn add_transfer_preview_layouts(
    fields: &mut Vec<SignablePayloadField>,
    transaction: &SuiTransactionBlockData,
) {
    let detected_transfers = detect_transfer_from_transaction(transaction);

    let transfer_list: Vec<&TransferInfo> = detected_transfers
        .iter()
        // TODO: think about error handling
        .filter_map(|x| x.as_ref().ok())
        .collect();

    for (index, transfer) in transfer_list.iter().enumerate() {
        fields.push(create_transfer_preview_layout(transfer, index + 1));
    }
}

/// Create a preview layout for a transfer
fn create_transfer_preview_layout(transfer: &TransferInfo, index: usize) -> SignablePayloadField {
    let title_text = match &transfer.coin_object {
        CoinObject::Sui => format!(
            "Transfer {}: {} MIST ({} SUI)",
            index,
            transfer.amount,
            transfer.amount / MIST_PER_SUI,
        ),
        CoinObject::Unknown(_) => format!("Transfer {}: {} tokens", index, transfer.amount),
    };

    let subtitle_text = format!(
        "From {} to {}",
        truncate_address(&transfer.sender.to_string()),
        truncate_address(&transfer.recipient.to_string())
    );

    // Condensed view - just the transfer summary
    let condensed = SignablePayloadFieldListLayout {
        fields: vec![create_text_field(
            "Summary",
            &format!(
                "Transfer {} {} from {} to {}",
                transfer.amount,
                transfer.coin_object.get_label(),
                truncate_address(&transfer.sender.to_string()),
                truncate_address(&transfer.recipient.to_string())
            ),
        )],
    };

    // Expanded view - detailed breakdown
    let expanded = SignablePayloadFieldListLayout {
        fields: create_transfer_expanded_fields(transfer),
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title_text.clone(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: subtitle_text,
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: title_text.clone(),
            label: format!("Transfer {}", index),
        },
        preview_layout,
    }
}

/// Create expanded fields for a transfer
fn create_transfer_expanded_fields(transfer: &TransferInfo) -> Vec<AnnotatedPayloadField> {
    vec![
        // TODO: resolve object id
        create_text_field("Asset Object ID", &transfer.coin_object.to_string()),
        create_address_field("From", &transfer.sender.to_string(), None, None, None, None),
        create_address_field(
            "To",
            &transfer.recipient.to_string(),
            None,
            None,
            None,
            None,
        ),
        create_amount_field(
            "Amount",
            &transfer.amount.to_string(),
            &transfer.coin_object.get_label(),
        ),
    ]
}

/// Add transaction details using preview layout
fn add_transaction_details_preview_layout(
    fields: &mut Vec<SignablePayloadField>,
    tx_data: &TransactionData,
    block_data: &SuiTransactionBlockData,
) {
    let title_text = "Transaction Details";
    let subtitle_text = format!("Gas: {} MIST", block_data.gas_data().budget);

    let condensed = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Type", &determine_transaction_type_string(block_data)),
            create_text_field(
                "Gas Budget",
                &format!("{} MIST", block_data.gas_data().budget),
            ),
        ],
    };

    let mut expanded = SignablePayloadFieldListLayout {
        fields: create_transaction_expanded_fields(block_data),
    };

    if let Ok(encoded) = bcs::to_bytes::<TransactionData>(tx_data) {
        expanded.fields.push(create_raw_data_field(&encoded));
    }

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title_text.to_string(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: subtitle_text,
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    fields.push(SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: "Transaction Details".to_string(),
            label: "Transaction".to_string(),
        },
        preview_layout,
    });
}

/// Create expanded fields for transaction details
fn create_transaction_expanded_fields(
    tx_data: &SuiTransactionBlockData,
) -> Vec<AnnotatedPayloadField> {
    let mut fields = vec![
        create_text_field(
            "Transaction Type",
            &determine_transaction_type_string(tx_data),
        ),
        create_address_field(
            "Gas Owner",
            &tx_data.gas_data().owner.to_string(),
            None,
            None,
            None,
            None,
        ),
        create_amount_field("Gas Budget", &tx_data.gas_data().budget.to_string(), "MIST"),
        create_amount_field("Gas Price", &tx_data.gas_data().price.to_string(), "MIST"),
    ];

    if let SuiTransactionBlockKind::ProgrammableTransaction(pt) = &tx_data.transaction() {
        fields.push(create_text_field(
            "Commands",
            &pt.commands.len().to_string(),
        ));
    }

    fields
}

/// Determine transaction title based on type
fn determine_transaction_type_string(tx_data: &SuiTransactionBlockData) -> String {
    match &tx_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(_) => "Programmable Transaction",
        SuiTransactionBlockKind::ChangeEpoch(_) => "Change Epoch",
        SuiTransactionBlockKind::Genesis(_) => "Genesis Transaction",
        SuiTransactionBlockKind::ConsensusCommitPrologue(_) => "Consensus Commit",
        SuiTransactionBlockKind::AuthenticatorStateUpdate(_) => "Authenticator State Update",
        SuiTransactionBlockKind::RandomnessStateUpdate(_) => "Randomness State Update",
        SuiTransactionBlockKind::EndOfEpochTransaction(_) => "End of Epoch Transaction",
        SuiTransactionBlockKind::ConsensusCommitPrologueV2(_) => "Consensus Commit Prologue V2",
        SuiTransactionBlockKind::ConsensusCommitPrologueV3(_) => "Consensus Commit Prologue V3",
        SuiTransactionBlockKind::ConsensusCommitPrologueV4(_) => "Consensus Commit Prologue V4",
        SuiTransactionBlockKind::ProgrammableSystemTransaction(_) => {
            "Programmable System Transaction"
        }
    }
    .to_string()
}

/// Truncate address to show first 6 and last 4 characters
fn truncate_address(address: &str) -> String {
    if address.len() <= 10 {
        return address.to_string();
    }

    format!("{}...{}", &address[..6], &address[address.len() - 4..])
}

/// Public API function for ease of use
pub fn transaction_to_visual_sign(
    transaction: TransactionData,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SuiVisualSignConverter.to_visual_sign_payload(SuiTransactionWrapper::new(transaction), options)
}

/// Public API function for string-based transactions
pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SuiVisualSignConverter.to_visual_sign_payload_from_string(transaction_data, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_transaction() {
        let result = decode_transaction("", SupportedEncodings::Base64);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Transaction is empty");
    }

    #[test]
    fn test_parse_native_transfer() {
        let result = decode_transaction(
            "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==",
            SupportedEncodings::Base64,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_basic_transaction_info() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let result = decode_transaction(test_data, SupportedEncodings::Base64);

        assert!(result.is_ok());
    }

    #[test]
    fn test_sui_transaction_to_vsp() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let options = VisualSignOptions::default();

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload.title, "Programmable Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "Sui");

        assert!(!payload.fields.is_empty());

        let network_field = payload.fields.iter().find(|f| f.label() == "Network");
        assert!(network_field.is_some());

        let json_result = payload.to_json();
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_sui_transaction_trait() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";

        let result = SuiTransactionWrapper::from_string(test_data);
        assert!(result.is_ok());

        let sui_tx = result.unwrap();
        assert_eq!(sui_tx.transaction_type(), "Sui");

        let invalid_result = SuiTransactionWrapper::from_string("invalid_data");
        assert!(invalid_result.is_err());
    }

    #[test]
    fn test_truncate_address() {
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let truncated = truncate_address(address);
        assert_eq!(truncated, "0x1234...5678");

        let short_address = "0x12345";
        let truncated_short = truncate_address(short_address);
        assert_eq!(truncated_short, "0x12345");
    }

    #[test]
    fn test_preview_layout_structure() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();

        let preview_fields: Vec<_> = payload
            .fields
            .iter()
            .filter(|f| f.field_type() == "preview_layout")
            .collect();

        assert!(
            !preview_fields.is_empty(),
            "Should have preview layout fields"
        );

        let transaction_preview = payload.fields.iter().find(|f| f.label() == "Transaction");
        assert!(
            transaction_preview.is_some(),
            "Should have transaction preview layout"
        );
    }
}
