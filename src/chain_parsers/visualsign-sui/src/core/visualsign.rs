use crate::core::commands::decode_commands;
use crate::core::helper::SuiModuleResolver;
use crate::core::transaction::{
    decode_transaction, determine_transaction_type_string, get_tx_details, get_tx_network,
};

use move_bytecode_utils::module_cache::SyncModuleCache;

use sui_json_rpc_types::SuiTransactionBlockData;
use sui_types::transaction::TransactionData;

use crate::core::commands;
use visualsign::{
    SignablePayload, SignablePayloadField,
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

impl SuiTransactionWrapper {
    /// Create a new `SuiTransactionWrapper`
    #[must_use]
    pub fn new(transaction: TransactionData) -> Self {
        Self { transaction }
    }

    /// Get a reference to the inner transaction
    #[must_use]
    pub fn inner(&self) -> &TransactionData {
        &self.transaction
    }
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

/// Converter that knows how to format Sui transactions for `VisualSign`
pub struct SuiVisualSignConverter;

impl VisualSignConverterFromString<SuiTransactionWrapper> for SuiVisualSignConverter {}

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

/// Convert Sui transaction to visual sign payload
fn convert_to_visual_sign_payload(
    transaction: &TransactionData,
    decode_transfers: bool,
    title: Option<String>,
) -> Result<SignablePayload, VisualSignError> {
    let block_data: SuiTransactionBlockData = SuiTransactionBlockData::try_from_with_module_cache(
        transaction.clone(),
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .map_err(|e| VisualSignError::ParseError(TransactionParseError::DecodeError(e.to_string())))?;

    let mut fields: Vec<SignablePayloadField> = vec![get_tx_network()?.signable_payload_field];

    if decode_transfers {
        fields.extend(
            commands::decode_transfers(&block_data)?
                .iter()
                .map(|e| e.signable_payload_field.clone()),
        );
    }

    fields.extend(
        decode_commands(&block_data)?
            .iter()
            .map(|e| e.signable_payload_field.clone()),
    );

    fields.push(get_tx_details(transaction, &block_data)?.signable_payload_field);

    let title = title.unwrap_or_else(|| determine_transaction_type_string(&block_data).to_string());
    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "Sui".to_string(),
    ))
}

/// Public API function for ease of use
///
/// # Errors
/// Returns a `VisualSignError` if the transaction cannot be converted into
/// `SuiTransactionBlockData`, if command/transfer decoding fails, or if any
/// field construction fails during payload building.
pub fn transaction_to_visual_sign(
    transaction: TransactionData,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SuiVisualSignConverter.to_visual_sign_payload(SuiTransactionWrapper::new(transaction), options)
}

/// Public API function for string-based transactions
///
/// # Errors
/// Returns a `VisualSignError` if the string cannot be decoded into a Sui
/// transaction (invalid encoding/content) or if payload construction fails
/// while decoding commands, transfers, or building fields.
pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SuiVisualSignConverter.to_visual_sign_payload_from_string(transaction_data, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::payload_from_b64;

    #[test]
    fn test_sui_transaction_to_vsp() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let payload: SignablePayload = payload_from_b64(test_data);
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
    fn test_transaction_details() {
        // https://suivision.xyz/txblock/4D74Jw1sA6ftnLU5JwTVmkrshtSJ5srBeaBXoHwwqXun
        let test_data = "AQAAAAAAAwEAiH3AfwMd9LgjR4Cpv4q9ohzJH5IGeEULdceikU993ywe1bUjAAAAACBk6AzdkhBsxlD09qOl5EZAO3xcqW6YGk3I/huiKDl/JwAIsAMAAAAAAAAAIIfCtnxql1/lDJTgzlHRhoM4PhhvgsnOzBYXB2t5uPgHAgIBAAABAQEAAQECAAABAgCqoKWfAWNCech3JFGHAe31KyrhICC2Xnk32BB6CBv3iQEvqmE5BRF5+VxSGYJp3pmHy08B5Ha1j1QhOjzCugXiaB7VtSMAAAAAIL6nYe4HoYtMDfV/DHDI9cQFEojqzSSrgcY1CFS4X53NqqClnwFjQnnIdyRRhwHt9Ssq4SAgtl55N9gQeggb94kmAgAAAAAAAIg9NAAAAAAAAAFhALw7iSOLS7LpZVsR0DZ4g3N/CCfB7O3YBtJ9fmxMOhBW9r+8Qzg5enH6KpIaq8PR/+sID/qeo+rvDpxB3jXdlgtUydWB+lIRciOIfNf/w8FzDBGL/PRFz4UbH7gWBqeEZA==";

        let payload = payload_from_b64(test_data);
        let transaction_preview = payload
            .fields
            .iter()
            .find(|f| f.label() == "Transaction Details");
        assert!(
            transaction_preview.is_some(),
            "Should have Transaction Details layout"
        );
    }
}
