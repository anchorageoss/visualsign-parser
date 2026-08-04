//! `NearVisualSignConverter`: NEAR transaction -> VisualSign payload.

use near_primitives::action::Action;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_address_field, create_text_field};
use visualsign::vsptrait::{
    ConversionResult, VisualSignConverter, VisualSignConverterFromString, VisualSignOptions,
};
use visualsign::{SignablePayload, SignablePayloadField};

use crate::actions::render_action;
use crate::networks::{NearNetwork, extract_network_from_metadata};
use crate::tx::NearTransaction;

/// Payload version emitted for NEAR transactions.
const PAYLOAD_VERSION: i64 = 0;
/// Payload type tag emitted for NEAR transactions, matching the other chain
/// crates' convention (`"SolanaTx"`, `"TronTx"`).
const PAYLOAD_TYPE: &str = "NearTx";

/// Converts a [`NearTransaction`] into a VisualSign [`SignablePayload`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NearVisualSignConverter {
    network: NearNetwork,
}

impl NearVisualSignConverter {
    /// Construct a converter for mainnet (the default for wallet display).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a converter for a specific [`NearNetwork`] (e.g. testnet).
    #[must_use]
    pub fn with_network(network: NearNetwork) -> Self {
        Self { network }
    }
}

impl VisualSignConverter<NearTransaction> for NearVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: NearTransaction,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let tx = transaction.inner();

        if tx.actions().is_empty() {
            return Err(VisualSignError::ValidationError(
                "NEAR transaction has no actions".to_string(),
            ));
        }

        let network = match extract_network_from_metadata(options.metadata.as_ref())? {
            Some(network) => network,
            None => self.network,
        };
        if let Some(mismatch) = network_mismatch(tx.signer_id().as_str(), network) {
            return Err(VisualSignError::ValidationError(mismatch));
        }

        let mut fields: Vec<SignablePayloadField> = Vec::new();
        fields.push(create_text_field("Network", network.display_name())?.signable_payload_field);
        fields.push(
            create_address_field("From", tx.signer_id().as_str(), None, None, None, None)?
                .signable_payload_field,
        );
        fields.push(
            create_address_field("To", tx.receiver_id().as_str(), None, None, None, None)?
                .signable_payload_field,
        );
        let total_actions = tx.actions().len();
        for action in tx.actions() {
            fields.extend(render_action(action, total_actions)?);
        }

        Ok(ConversionResult::new(SignablePayload::new(
            PAYLOAD_VERSION,
            title_for(tx.actions()),
            None,
            fields,
            PAYLOAD_TYPE.to_string(),
        )))
    }
}

impl VisualSignConverterFromString<NearTransaction> for NearVisualSignConverter {
    fn to_visual_sign_payload_from_string(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let transaction = NearTransaction::from_string_with_options(
            transaction_data,
            options.developer_config.as_ref(),
        )
        .map_err(VisualSignError::ParseError)?;
        self.to_validated_visual_sign_payload(transaction, options)
    }
}

/// Title for the payload: a single action names itself, otherwise a generic label.
fn title_for(actions: &[Action]) -> String {
    match actions {
        [single] => crate::actions::action_label(single).to_string(),
        _ => "NEAR Transaction".to_string(),
    }
}

/// Detects a signer account whose top-level suffix contradicts the resolved
/// network (`.testnet` under Mainnet, or `.near` under Testnet). Implicit
/// 64-hex accounts carry no suffix and are not guarded here.
fn network_mismatch(signer_id: &str, network: NearNetwork) -> Option<String> {
    let mismatched = match network {
        NearNetwork::Mainnet => signer_id.ends_with(".testnet"),
        NearNetwork::Testnet => signer_id.ends_with(".near"),
    };
    mismatched.then(|| {
        format!("signer account '{signer_id}' does not match resolved network {network:?}")
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};
    use near_crypto::{KeyType, PublicKey, Signature};
    use near_primitives::action::{CreateAccountAction, TransferAction};
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::{SignedTransaction, TransactionV0};
    use near_primitives::types::Balance;
    use visualsign::vsptrait::DeveloperConfig;

    fn transfer() -> Action {
        Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1),
        })
    }

    fn near_tx(actions: Vec<Action>) -> NearTransaction {
        near_tx_as("alice.near", "bob.near", actions)
    }

    fn near_tx_as(signer_id: &str, receiver_id: &str, actions: Vec<Action>) -> NearTransaction {
        NearTransaction::new(near_primitives::transaction::Transaction::V0(
            TransactionV0 {
                signer_id: signer_id.parse().expect("valid account id"),
                public_key: PublicKey::empty(KeyType::ED25519),
                nonce: 0,
                receiver_id: receiver_id.parse().expect("valid account id"),
                block_hash: CryptoHash::default(),
                actions,
            },
        ))
    }

    #[test]
    fn title_single_action_uses_action_name() {
        assert_eq!(title_for(&[transfer()]), "Transfer");
    }

    #[test]
    fn title_multi_action_is_generic() {
        let actions = [Action::CreateAccount(CreateAccountAction {}), transfer()];
        assert_eq!(title_for(&actions), "NEAR Transaction");
    }

    #[test]
    fn title_no_action_is_generic() {
        assert_eq!(title_for(&[]), "NEAR Transaction");
    }

    #[test]
    fn with_network_sets_the_configured_network() {
        assert_eq!(
            NearVisualSignConverter::with_network(NearNetwork::Testnet).network,
            NearNetwork::Testnet
        );
        assert_eq!(NearVisualSignConverter::new().network, NearNetwork::Mainnet);
    }

    #[test]
    fn rejects_transaction_with_no_actions() {
        let converter = NearVisualSignConverter::new();
        let result =
            converter.to_visual_sign_payload(near_tx(vec![]), VisualSignOptions::default());
        assert!(matches!(result, Err(VisualSignError::ValidationError(_))));
    }

    fn field_text(field: &SignablePayloadField) -> &str {
        let SignablePayloadField::TextV2 { text_v2, .. } = field else {
            panic!("expected a TextV2 field, got {field:?}");
        };
        &text_v2.text
    }

    #[test]
    fn renders_constructed_network_when_metadata_absent() {
        let converter = NearVisualSignConverter::with_network(NearNetwork::Testnet);
        let tx = near_tx_as("alice.testnet", "bob.testnet", vec![transfer()]);
        let result = converter
            .to_visual_sign_payload(tx, VisualSignOptions::default())
            .expect("conversion succeeds");
        assert_eq!(field_text(&result.payload.fields[0]), "NEAR Testnet");
    }

    #[test]
    fn metadata_network_overrides_constructed_network() {
        let converter = NearVisualSignConverter::new();
        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_TESTNET".to_string()),
                })),
            }),
            ..Default::default()
        };
        let tx = near_tx_as("alice.testnet", "bob.testnet", vec![transfer()]);
        let result = converter
            .to_visual_sign_payload(tx, options)
            .expect("conversion succeeds");
        assert_eq!(field_text(&result.payload.fields[0]), "NEAR Testnet");
    }

    #[test]
    fn rejects_unrecognized_network_id_instead_of_falling_back_to_mainnet() {
        let converter = NearVisualSignConverter::new();
        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("testnet".to_string()),
                })),
            }),
            ..Default::default()
        };
        let result = converter.to_visual_sign_payload(near_tx(vec![transfer()]), options);
        assert!(matches!(result, Err(VisualSignError::ValidationError(_))));
    }

    #[test]
    fn rejects_testnet_signer_under_resolved_mainnet_network() {
        let converter = NearVisualSignConverter::new();
        let tx = near_tx_as("alice.testnet", "bob.testnet", vec![transfer()]);
        let result = converter.to_visual_sign_payload(tx, VisualSignOptions::default());
        assert!(matches!(result, Err(VisualSignError::ValidationError(_))));
    }

    fn signed_transfer_hex() -> String {
        let unsigned = near_tx(vec![transfer()]).inner().clone();
        let signed = SignedTransaction::new(Signature::empty(KeyType::ED25519), unsigned);
        hex::encode(borsh::to_vec(&signed).expect("borsh encode"))
    }

    #[test]
    fn converter_rejects_signed_transaction_by_default() {
        let converter = NearVisualSignConverter::new();
        let result = converter.to_visual_sign_payload_from_string(
            &signed_transfer_hex(),
            VisualSignOptions::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn converter_accepts_signed_transaction_when_developer_config_allows_it() {
        let converter = NearVisualSignConverter::new();
        let options = VisualSignOptions {
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
            ..Default::default()
        };
        let result = converter.to_visual_sign_payload_from_string(&signed_transfer_hex(), options);
        assert!(result.is_ok());
    }
}
