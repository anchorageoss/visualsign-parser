//! `NearVisualSignConverter`: NEAR transaction -> VisualSign payload.

use near_primitives::action::Action;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_address_field, create_text_field};
use visualsign::vsptrait::{
    ConversionResult, VisualSignConverter, VisualSignConverterFromString, VisualSignOptions,
};
use visualsign::{SignablePayload, SignablePayloadField};

use crate::actions::render_action;
use crate::networks::NearNetwork;
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
        _options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let tx = &transaction.inner;

        let mut fields: Vec<SignablePayloadField> = Vec::new();
        fields.push(
            create_text_field("Network", self.network.display_name())?.signable_payload_field,
        );
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
mod tests {
    use super::*;
    use near_primitives::action::{CreateAccountAction, TransferAction};
    use near_primitives::types::Balance;

    fn transfer() -> Action {
        Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1),
        })
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
}
