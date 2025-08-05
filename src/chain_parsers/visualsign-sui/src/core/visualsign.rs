use crate::core::commands::add_tx_commands;
use crate::core::helper::SuiModuleResolver;
use crate::core::transaction::{
    add_tx_details, add_tx_network, decode_transaction, determine_transaction_type_string,
};

use move_bytecode_utils::module_cache::SyncModuleCache;

use sui_json_rpc_types::SuiTransactionBlockData;
use sui_types::transaction::TransactionData;

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
    /// Create a new SuiTransactionWrapper
    #[allow(dead_code)]
    pub fn new(transaction: TransactionData) -> Self {
        Self { transaction }
    }

    /// Get a reference to the inner transaction
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

/// Converter that knows how to format Sui transactions for VisualSign
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
    _decode_transfers: bool,
    title: Option<String>,
) -> Result<SignablePayload, VisualSignError> {
    let block_data: SuiTransactionBlockData = SuiTransactionBlockData::try_from_with_module_cache(
        transaction.clone(),
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .map_err(|e| VisualSignError::ParseError(TransactionParseError::DecodeError(e.to_string())))?;

    let mut fields: Vec<SignablePayloadField> = vec![];

    fields.push(add_tx_network().signable_payload_field);
    fields.push(add_tx_details(transaction, &block_data));

    // TODO: revisit this later
    // if decode_transfers {}

    fields.extend(add_tx_commands(&block_data));

    let title = title.unwrap_or_else(|| determine_transaction_type_string(&block_data));
    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "Sui".to_string(),
    ))
}

/// Public API function for ease of use
#[allow(dead_code)]
pub fn transaction_to_visual_sign(
    transaction: TransactionData,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SuiVisualSignConverter.to_visual_sign_payload(SuiTransactionWrapper::new(transaction), options)
}

/// Public API function for string-based transactions
#[allow(dead_code)]
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
    fn test_sui_transaction_to_vsp() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let options = VisualSignOptions::default();

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload: SignablePayload = result.unwrap();
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
    fn test_token_transfer_1() {
        // https://suivision.xyz/txblock/4D74Jw1sA6ftnLU5JwTVmkrshtSJ5srBeaBXoHwwqXun
        let test_data = "AQAAAAAAAwEAiH3AfwMd9LgjR4Cpv4q9ohzJH5IGeEULdceikU993ywe1bUjAAAAACBk6AzdkhBsxlD09qOl5EZAO3xcqW6YGk3I/huiKDl/JwAIsAMAAAAAAAAAIIfCtnxql1/lDJTgzlHRhoM4PhhvgsnOzBYXB2t5uPgHAgIBAAABAQEAAQECAAABAgCqoKWfAWNCech3JFGHAe31KyrhICC2Xnk32BB6CBv3iQEvqmE5BRF5+VxSGYJp3pmHy08B5Ha1j1QhOjzCugXiaB7VtSMAAAAAIL6nYe4HoYtMDfV/DHDI9cQFEojqzSSrgcY1CFS4X53NqqClnwFjQnnIdyRRhwHt9Ssq4SAgtl55N9gQeggb94kmAgAAAAAAAIg9NAAAAAAAAAFhALw7iSOLS7LpZVsR0DZ4g3N/CCfB7O3YBtJ9fmxMOhBW9r+8Qzg5enH6KpIaq8PR/+sID/qeo+rvDpxB3jXdlgtUydWB+lIRciOIfNf/w8FzDBGL/PRFz4UbH7gWBqeEZA==";

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        let transaction_preview = payload
            .fields
            .iter()
            .find(|f| f.label() == "Transaction Details");
        assert!(
            transaction_preview.is_some(),
            "Should have Transaction Details layout"
        );
    }

    #[test]
    fn test_transfer_command() {
        // https://suivision.xyz/txblock/CE46w3GYgWnZU8HF4P149m6ANGebD22xuNqA64v7JykJ
        let test_data = "AQAAAAAABQEAm9cmP35lHGKppWJLgoYU7aexd43oTT2ci4QzxDXFNv92CAsjAAAAACANp0teIzSyzZ4Pj5dL3YaYBdeVmiWScWL/9RCV4mUINwEAARQFJheK7qwbpqmQudEhsSyQ6AjVawfLpN4XRBhe12FH6TIiAAAAACDXzuT2xanZ36QNQSYtDhZn31zfzIlhRk5H6pTsqGdRDAEAXpykdGz3KJdaAVjyAMZQxufRYJfqzNXfOu8jVCAjEjIzfYIhAAAAACA5hk9rACYb1i5fqrUBJIgXhdUFOqOaouNWmQINCW4/WQAIAPLhNQAAAAAAIEutPmqkZpN81fwdos/haXZAQJoZsX8SvKilyMRxrv/pAwMBAAACAQEAAQIAAgEAAAEBAwABAQIBAAEEAA4x8k3bZAV+p192pmk9h7U2nGDwuTmW8EY6c95JyFHCAaCnde0j6aiVXUd/1gCf3q5Uuj1mPVIuuEpJn1teueghdggLIwAAAAAgNhuP2zGpc0qF3gRzxQC5B0lpAZR7xyssXC3gKbH8uxwOMfJN22QFfqdfdqZpPYe1Npxg8Lk5lvBGOnPeSchRwugDAAAAAAAAoIVIAAAAAAAAAWEAFrlPuI8JOSzIoIBc0xwfWia7T5uPf1PS+aSSphoTTq0lRpNuTOg8eOggpBxpLsQDrbAx3jDoWg1R8hZKR62LBex1R808U6AgiY8V7LxOVsChXFf8nSAEGaeSLQc7mJbx";

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        let transaction_preview = payload
            .fields
            .iter()
            .find(|f| f.label() == "Transfer Command");
        assert!(
            transaction_preview.is_some(),
            "Should have Transfer Command layout"
        );
    }

    #[test]
    fn test_stake_commands() {
        // Stake
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "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";

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        let transaction_preview = payload.fields.iter().find(|f| f.label() == "Stake Command");
        assert!(
            transaction_preview.is_some(),
            "Should have Stake Command layout"
        );
    }

    #[test]
    fn test_withdraw_commands() {
        // Withdraw
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "AQAAAAAAAgEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAUBAAAAAAAAAAEBADtuZRRZcXabYn2eLpOPGq3onyss/0Kyuv3BoB3PQPiIJHpFHQAAAAAgDlI1Bti2mpZBb/rDxYkyB+lyANUGRTtYgKbRoBow53cBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCnN1aV9zeXN0ZW0WcmVxdWVzdF93aXRoZHJhd19zdGFrZQACAQAAAQEAPmYGcGNxVi5pj8Tk1ufHEB6SYs6TFjQYj+JG7623BnUCN8ccpwVmcafDNOXvnEAo6kzltjdniobA56to42fHdUio9wcjAAAAACDQVC4fMhsmX6OlHpAhyPR8LaRzgu43Bj8xrhlRY6YKG/Yv6m2ncHpPhbrEkOrSiyh1ID3T4FARE+raMUofCsQPqPcHIwAAAAAg5qp+jjoniUXPNG4N0/9XDFSpoUt0isbEUMiXjNtGivA+ZgZwY3FWLmmPxOTW58cQHpJizpMWNBiP4kbvrbcGdSECAAAAAAAADAqcAAAAAAAAAWEAkj0EN51BkbIUE/6lMi967MHGsBMl2i8TtntUnFhlC2rK8AW2fGQxc8mg1gTbV+2eHs1CsZ9m67cU4CWzA+9PAg//ECUrmzUzzsg0xYRgwDQDy9lAF8e6bpAa8/5Yec6s";

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        let transaction_preview = payload
            .fields
            .iter()
            .find(|f| f.label() == "Withdraw Command");
        assert!(
            transaction_preview.is_some(),
            "Should have Withdraw Command layout"
        );
    }

    #[test]
    fn test_cetus_amm_swap_b2a_commands() {
        // https://suivision.xyz/txblock/7Je4yeXMvvEHFcRSTD4WYv3eSsaDk2zqvdoSxWXdUYGx
        let test_data = "AQAAAAAACQEAEXs/ewhS1RZrUZQ2xQEliCJn40SK4PvEV75r2SGFMXhjUsAjAAAAACBSKqlrLdPXYeuzckz31NAkeSO09qmNPv/pkWggJMTC2QAIuMbAAQAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFK94o+ni1sq8pdp5wea/9ImVZqQhMh/DtaYZZkAXpg1nkOqBoAAAAAAQABAQAIuMbAAQAAAAAACI0+GgMAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAQAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB7eETiiahBDlD7PKSNaeuc8p4n0iPvkDU/4b2OJ/+PP4BGNvaW4EQ09JTgAJAQIAAQMAAgEAAgAAAQQAAQUAAQYAAQcAAQgArltnUkfA5IdctLm9N6YO1bz4kng0TThA3StCbiinZoUBZI8YcdbCiGOtIFCZV/M9U6lZTgf3lg6t7feHRsBBqR1jUsAjAAAAACCmwR6aeqn8D632smpzU9fbDhP3vPOQhgc806IrzekPH65bZ1JHwOSHXLS5vTemDtW8+JJ4NE04QN0rQm4op2aFBQIAAAAAAAC8YDQAAAAAAAABYQAdbFpPHuOPe/TYRMttj4FSzAN1ErZdI75GooTkFmiIVkvCM+lnSS3pR/qQt6j7K3gsrtBExfgOL/dffWapvuMEyeP1ig9kZWEaY4lMw99QxRTo2PcUhKsb1gquOOAGXP8=";

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(test_data, options);
        assert!(result.is_ok());

        let payload = result.unwrap();
        let transaction_preview = payload
            .fields
            .iter()
            .find(|f| f.label() == "CetusAMM Swap Command");
        assert!(
            transaction_preview.is_some(),
            "Should have CetusAMM Swap Command layout"
        );
    }
}
