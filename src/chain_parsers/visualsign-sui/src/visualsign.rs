use crate::{TransactionEncoding, parse_sui_transaction};

use sui_json_rpc_types::{
    SuiTransactionBlock, SuiTransactionBlockData, SuiTransactionBlockDataAPI,
    SuiTransactionBlockKind,
};

use crate::commands::{TransferInfo, detect_transfer_from_transaction};
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

/// Wrapper for SuiTransactionBlock to implement the Transaction trait
#[derive(Debug, Clone)]
pub struct SuiTransactionWrapper {
    pub transaction_block: SuiTransactionBlock,
    pub raw_data: String,
}

impl Transaction for SuiTransactionWrapper {
    fn from_string(data: &str) -> anyhow::Result<Self, TransactionParseError> {
        let encoding = TransactionEncoding::Base64;
        match parse_sui_transaction(data.to_string(), encoding) {
            Ok(transaction_block) => Ok(SuiTransactionWrapper {
                transaction_block,
                raw_data: data.to_string(),
            }),
            Err(e) => Err(TransactionParseError::InvalidFormat(e.to_string())),
        }
    }

    fn transaction_type(&self) -> String {
        "Sui".to_string()
    }
}

/// Converter for Sui transactions to VSP format
pub struct SuiTransactionConverter;

impl VisualSignConverter<SuiTransactionWrapper> for SuiTransactionConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: SuiTransactionWrapper,
        _options: VisualSignOptions,
    ) -> anyhow::Result<SignablePayload, VisualSignError> {
        let tx_block = &transaction.transaction_block;

        // Try to detect if this is a transfer transaction
        let transfer_info = detect_transfer_from_transaction(tx_block);

        let mut fields = Vec::new();

        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: "Sui Network".to_string(),
                label: "Network".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: "Sui Network".to_string(),
            },
        });

        let tx_data: &SuiTransactionBlockData = &tx_block.data;

        fields.push(SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: tx_data.sender().to_string(),
                label: "From".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: tx_data.sender().to_string(),
                name: "".to_string(),
                memo: None,
                asset_label: "".to_string(),
                badge_text: None,
            },
        });

        // Add transfer-specific fields if this is a transfer
        for transfer in transfer_info {
            fields.extend(transfer_info_to_vsp(&transfer));
        }

        // Add gas owner
        fields.push(SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: tx_data.gas_data().owner.to_string(),
                label: "Gas Owner".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: tx_data.gas_data().owner.to_string(),
                name: "".to_string(),
                memo: None,
                asset_label: "".to_string(),
                badge_text: None,
            },
        });

        // Add gas budget
        fields.push(SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} MIST", tx_data.gas_data().budget),
                label: "Gas Budget".to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: tx_data.gas_data().budget.to_string(),
                abbreviation: Some("MIST".to_string()),
            },
        });

        // Add gas price
        fields.push(SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} MIST", tx_data.gas_data().price),
                label: "Gas Price".to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: tx_data.gas_data().price.to_string(),
                abbreviation: Some("MIST".to_string()),
            },
        });

        let tx_kind = match &tx_data.transaction() {
            SuiTransactionBlockKind::ProgrammableTransaction(pt) => {
                let command_count = pt.commands.len();
                format!("Programmable Transaction ({} commands)", command_count)
            }
            SuiTransactionBlockKind::ChangeEpoch(_) => "Change Epoch".to_string(),
            SuiTransactionBlockKind::Genesis(_) => "Genesis".to_string(),
            SuiTransactionBlockKind::ConsensusCommitPrologue(_) => {
                "Consensus Commit Prologue".to_string()
            }
            SuiTransactionBlockKind::AuthenticatorStateUpdate(_) => {
                "Authenticator State Update".to_string()
            }
            SuiTransactionBlockKind::RandomnessStateUpdate(_) => {
                "Randomness State Update".to_string()
            }
            SuiTransactionBlockKind::EndOfEpochTransaction(_) => {
                "End of Epoch Transaction".to_string()
            }
            SuiTransactionBlockKind::ConsensusCommitPrologueV2(_) => {
                "Consensus Commit Prologue V2".to_string()
            }
            SuiTransactionBlockKind::ConsensusCommitPrologueV3(_) => {
                "Consensus Commit Prologue V3".to_string()
            }
            SuiTransactionBlockKind::ConsensusCommitPrologueV4(_) => {
                "Consensus Commit Prologue V4".to_string()
            }
            SuiTransactionBlockKind::ProgrammableSystemTransaction(_) => {
                "Programmable System Transaction".to_string()
            }
        };

        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: tx_kind.clone(),
                label: "Transaction Type".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: tx_kind },
        });

        let title = match &tx_data.transaction() {
            SuiTransactionBlockKind::ProgrammableTransaction(_) => "Execute Transaction",
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
        .to_string();
        let subtitle = "".to_string();
        let payload_type = "Sui".to_string();

        Ok(SignablePayload::new(
            0,
            title,
            Some(subtitle),
            fields,
            payload_type,
        ))
    }
}

fn transfer_info_to_vsp(transfer_info: &TransferInfo) -> Vec<SignablePayloadField> {
    let mut fields: Vec<SignablePayloadField> = Vec::new();

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: transfer_info.token.to_string(),
            label: "Asset".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: transfer_info.token.to_string(),
        },
    });

    if let Some(recipient) = &transfer_info.recipient {
        fields.push(SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: recipient.to_string(),
                label: "To".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: recipient.to_string(),
                name: "".to_string(),
                memo: None,
                asset_label: "".to_string(),
                badge_text: None,
            },
        });
    }

    if let Some(amount) = &transfer_info.amount {
        let asset_label = transfer_info.token.get_label();
        fields.push(SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} {}", amount, asset_label),
                label: "Amount".to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: amount.to_string(),
                abbreviation: Some(asset_label),
            },
        });
    }

    fields
}

impl VisualSignConverterFromString<SuiTransactionWrapper> for SuiTransactionConverter {}

/// Convenience function to convert a base64-encoded Sui transaction to VSP format
pub fn sui_transaction_to_vsp(
    transaction_data: &str,
    options: VisualSignOptions,
) -> anyhow::Result<SignablePayload, VisualSignError> {
    let converter = SuiTransactionConverter;
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sui_transaction_to_vsp() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let options = VisualSignOptions::default();

        let result = sui_transaction_to_vsp(test_data, options);

        dbg!(&result);

        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload.title, "Execute Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "Sui");

        assert!(!payload.fields.is_empty());

        let network_field = payload.fields.iter().find(|f| f.label() == "Network");
        assert!(network_field.is_some());

        let from_field = payload.fields.iter().find(|f| f.label() == "From");
        assert!(from_field.is_some());

        let gas_budget_field = payload.fields.iter().find(|f| f.label() == "Gas Budget");
        assert!(gas_budget_field.is_some());

        let gas_price_field = payload.fields.iter().find(|f| f.label() == "Gas Price");
        assert!(gas_price_field.is_some());

        let tx_type_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Transaction Type");
        assert!(tx_type_field.is_some());

        let json_result = payload.to_json();
        assert!(json_result.is_ok());
        println!("VSP JSON: {}", json_result.unwrap());
    }

    #[test]
    fn test_sui_transaction_trait() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";

        let result = SuiTransactionWrapper::from_string(test_data);
        assert!(result.is_ok());

        let sui_tx = result.unwrap();
        assert_eq!(sui_tx.transaction_type(), "Sui");
        assert_eq!(sui_tx.raw_data, test_data);

        // Test with invalid data
        let invalid_result = SuiTransactionWrapper::from_string("invalid_data");
        assert!(invalid_result.is_err());
    }
}
