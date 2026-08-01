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
        let tx = &transaction.inner;

        if tx.actions().is_empty() {
            return Err(VisualSignError::ValidationError(
                "NEAR transaction has no actions".to_string(),
            ));
        }

        let network =
            extract_network_from_metadata(options.metadata.as_ref()).unwrap_or(self.network);

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

impl VisualSignConverterFromString<NearTransaction> for NearVisualSignConverter {}

/// Title for the payload: a single action names itself, otherwise a generic label.
fn title_for(actions: &[Action]) -> String {
    match actions {
        [single] => crate::actions::action_label(single).to_string(),
        _ => "NEAR Transaction".to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};
    use near_crypto::{KeyType, PublicKey};
    use near_primitives::action::{CreateAccountAction, TransferAction};
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::TransactionV0;
    use near_primitives::types::Balance;

    fn transfer() -> Action {
        Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1),
        })
    }

    fn near_tx(actions: Vec<Action>) -> NearTransaction {
        NearTransaction {
            inner: near_primitives::transaction::Transaction::V0(TransactionV0 {
                signer_id: "alice.near".parse().expect("valid account id"),
                public_key: PublicKey::empty(KeyType::ED25519),
                nonce: 0,
                receiver_id: "bob.near".parse().expect("valid account id"),
                block_hash: CryptoHash::default(),
                actions,
            }),
        }
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
        let result = converter
            .to_visual_sign_payload(near_tx(vec![transfer()]), VisualSignOptions::default())
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
        let result = converter
            .to_visual_sign_payload(near_tx(vec![transfer()]), options)
            .expect("conversion succeeds");
        assert_eq!(field_text(&result.payload.fields[0]), "NEAR Testnet");
    }
}
