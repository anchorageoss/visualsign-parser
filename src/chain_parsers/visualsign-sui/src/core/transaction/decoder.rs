use base64::Engine;

use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::TransactionParseError;

use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::transaction::{SenderSignedData, TransactionData};

/// Decode a transaction from string format
pub fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<TransactionData, TransactionParseError> {
    if raw_transaction.is_empty() {
        return Err(TransactionParseError::DecodeError(
            "Transaction is empty".into(),
        ));
    }

    let bytes = match encodings {
        SupportedEncodings::Base64 => base64::engine::general_purpose::STANDARD
            .decode(raw_transaction)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
        SupportedEncodings::Hex => hex::decode(raw_transaction)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
    };

    if let Ok(sender_signed_data) = bcs::from_bytes::<SenderSignedData>(&bytes) {
        return Ok(sender_signed_data.transaction_data().clone());
    }

    if let Ok(transaction_data) = bcs::from_bytes::<TransactionData>(&bytes) {
        return Ok(transaction_data);
    }

    Err(TransactionParseError::DecodeError(
        "Unable to decode transaction data as either `SenderSignedData` or `TransactionData`"
            .into(),
    ))
}

/// Determine transaction title based on type
pub fn determine_transaction_type_string(tx_data: &SuiTransactionBlockData) -> &'static str {
    match &tx_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(_) => "Programmable Transaction",
        SuiTransactionBlockKind::ChangeEpoch(_) => "Change Epoch",
        SuiTransactionBlockKind::Genesis(_) => "Genesis",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_transaction_empty() {
        let result = decode_transaction("", SupportedEncodings::Base64);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Decode error: Transaction is empty"
        );
    }

    #[test]
    fn test_decode_transaction_native_transfer() {
        let result = decode_transaction(
            "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==",
            SupportedEncodings::Base64,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_decode_transaction_basic_info() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let result = decode_transaction(test_data, SupportedEncodings::Base64);

        assert!(result.is_ok());
    }
}
