//! `NearVisualSignConverter`: NEAR input -> VisualSign payload.

use std::sync::Arc;

use near_primitives::action::Action;
use near_primitives::transaction::Transaction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_address_field, create_text_field};
use visualsign::registry::LayeredRegistry;
use visualsign::signing::MetadataTrustPolicy;
use visualsign::vsptrait::{
    ConversionResult, VisualSignConverter, VisualSignConverterFromString, VisualSignOptions,
};
use visualsign::{SignablePayload, SignablePayloadField};

use crate::actions::render_action;
use crate::networks::{NearNetwork, extract_network_from_metadata};
use crate::presets::intents::{
    NearTokenRegistry, authorized_token_metadata_signers,
    try_extract_token_metadata_from_chain_metadata,
};
use crate::tx::NearTransaction;

/// Build the token registry for this request: an empty global layer, plus
/// whatever `options.metadata` supplies (verified per
/// [`crate::presets::intents::TokenMetadataSignerAllowlists`]) as the
/// request-scoped layer. The compiled-in seed table lives separately in
/// `tokens::SEEDS`, consulted by `tokens::resolve` only after this registry's
/// own lookup misses.
///
/// `trust_policy` gates only whether an entry with no signature at all is
/// accepted; a present signature is always checked against the relevant
/// origin-chain allowlist (see `authorized_token_metadata_signers`)
/// regardless of posture -- NEAR, unlike Ethereum's single-allowlist ABI
/// path, already dispatches identity checks per origin chain, so the
/// allowlist `MetadataTrustPolicy::RequireAllowlistedSigner` carries is not
/// itself consulted here.
///
/// Returns the registry plus a diagnostic field for every entry the extraction
/// refused. Callers render those once per payload, not once per action.
fn token_registry_for(
    options: &VisualSignOptions,
    trust_policy: &MetadataTrustPolicy,
) -> (
    LayeredRegistry<NearTokenRegistry>,
    Vec<SignablePayloadField>,
) {
    let extraction = try_extract_token_metadata_from_chain_metadata(
        options.metadata.as_ref(),
        authorized_token_metadata_signers(),
        trust_policy,
    );
    let registry = match extraction.registry {
        Some(request) => {
            LayeredRegistry::with_request(Arc::new(NearTokenRegistry::default()), request)
        }
        None => LayeredRegistry::new(Arc::new(NearTokenRegistry::default())),
    };
    (
        registry,
        crate::presets::intents::rejected_metadata_diagnostics(&extraction.rejected),
    )
}

/// Payload version emitted for NEAR payloads.
const PAYLOAD_VERSION: i64 = 0;
/// Payload type tag emitted for NEAR payloads, matching the other chain
/// crates' convention (`"SolanaTx"`, `"TronTx"`).
const PAYLOAD_TYPE: &str = "NearTx";

/// Converts a [`NearTransaction`] into a VisualSign [`SignablePayload`].
#[derive(Debug, Clone)]
pub struct NearVisualSignConverter {
    network: NearNetwork,
    trust_policy: MetadataTrustPolicy,
}

impl NearVisualSignConverter {
    /// Construct a converter for mainnet with the permissive
    /// [`MetadataTrustPolicy::AcceptUnsigned`] posture -- the library/embedding
    /// default. Deployments that want an auditable, non-default posture should
    /// use [`Self::with_trust_policy`] instead.
    #[must_use]
    pub fn new() -> Self {
        Self {
            network: NearNetwork::default(),
            trust_policy: MetadataTrustPolicy::AcceptUnsigned,
        }
    }

    /// Construct a converter for a specific [`NearNetwork`] (e.g. testnet),
    /// with the permissive default trust posture.
    #[must_use]
    pub fn with_network(network: NearNetwork) -> Self {
        Self {
            network,
            ..Self::new()
        }
    }

    /// Construct a converter for mainnet with an explicit caller-metadata
    /// trust posture. This is the constructor a deployment should use to pin
    /// [`MetadataTrustPolicy::RequireAllowlistedSigner`] at construction time,
    /// fixed for the process rather than implied by what each request happens
    /// to contain.
    #[must_use]
    pub fn with_trust_policy(trust_policy: MetadataTrustPolicy) -> Self {
        Self {
            trust_policy,
            ..Self::new()
        }
    }
}

impl Default for NearVisualSignConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualSignConverter<NearTransaction> for NearVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: NearTransaction,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        match transaction {
            NearTransaction::OnChain(tx) => self.render_on_chain(&tx, &options),
            NearTransaction::Intent(json) => {
                render_intent_envelope(&json, &options, &self.trust_policy)
            }
        }
    }
}

impl NearVisualSignConverter {
    fn render_on_chain(
        &self,
        tx: &Transaction,
        options: &VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        if tx.actions().is_empty() {
            return Err(VisualSignError::ValidationError(
                "NEAR transaction has no actions".to_string(),
            ));
        }

        let network = match extract_network_from_metadata(options.metadata.as_ref())? {
            Some(network) => network,
            None => self.network,
        };
        for (role, account_id) in [
            ("signer", tx.signer_id().as_str()),
            ("receiver", tx.receiver_id().as_str()),
        ] {
            if let Some(mismatch) = network_mismatch(role, account_id, network) {
                return Err(VisualSignError::ValidationError(mismatch));
            }
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
        // Built once for the whole transaction: the metadata is request-scoped,
        // so a rejection is a property of the request, not of each action that
        // consults the registry. Building it per action would repeat every
        // rejection diagnostic for a multi-action transaction.
        let (registry, rejection_diagnostics) = token_registry_for(options, &self.trust_policy);
        fields.extend(rejection_diagnostics);

        let total_actions = tx.actions().len();
        for action in tx.actions() {
            fields.extend(render_action(action, total_actions)?);
            fields.extend(decode_intents(
                tx.receiver_id().as_str(),
                action,
                options,
                &registry,
            )?);
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

/// Decode an `execute_intents` call to `intents.near` and render the signed
/// intent batch. Any other action or receiver yields no extra fields. A
/// decode failure surfaces as a conversion error rather than silently
/// dropping the intents.
fn decode_intents(
    receiver_id: &str,
    action: &Action,
    options: &VisualSignOptions,
    registry: &LayeredRegistry<NearTokenRegistry>,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    if receiver_id != "intents.near" {
        return Ok(vec![]);
    }
    let Action::FunctionCall(fc) = action else {
        return Ok(vec![]);
    };
    if fc.method_name != "execute_intents" {
        return Ok(vec![]);
    }
    crate::presets::intents::try_decode_execute_intents(&fc.args, registry, options)
        .map_err(|e| VisualSignError::ConversionError(e.to_string()))
}

/// Render the pre-signature intents envelope a user is about to sign: no
/// signature exists yet, so this is a confirmation view only.
fn render_intent_envelope(
    json: &str,
    options: &VisualSignOptions,
    trust_policy: &MetadataTrustPolicy,
) -> Result<ConversionResult, VisualSignError> {
    let (registry, rejection_diagnostics) = token_registry_for(options, trust_policy);
    let mut fields = rejection_diagnostics;
    let rendered =
        crate::presets::intents::try_render_single_intent(json.as_bytes(), &registry, options)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?;
    fields.extend(rendered.fields);
    Ok(ConversionResult::new(SignablePayload::new(
        PAYLOAD_VERSION,
        rendered.title,
        None,
        fields,
        PAYLOAD_TYPE.to_string(),
    )))
}

/// Title for the payload: a single action names itself, otherwise a generic
/// label. An action whose fields are only partially decoded carries the same
/// qualifier in the title as in its field, so the headline does not claim more
/// than the body.
fn title_for(actions: &[Action]) -> String {
    match actions {
        [single] if crate::actions::is_partially_decoded(single) => {
            crate::actions::partially_decoded_label(single)
        }
        [single] => crate::actions::action_label(single).to_string(),
        _ => "NEAR Transaction".to_string(),
    }
}

/// Detects an account whose top-level suffix contradicts the resolved network
/// (`.testnet` under Mainnet, or `.near` under Testnet). `role` names which
/// account failed, so the error distinguishes signer from receiver. Implicit
/// 64-hex accounts carry no suffix and are not guarded here.
fn network_mismatch(role: &str, account_id: &str, network: NearNetwork) -> Option<String> {
    let mismatched = match network {
        NearNetwork::Mainnet => account_id.ends_with(".testnet"),
        NearNetwork::Testnet => account_id.ends_with(".near"),
    };
    mismatched.then(|| {
        format!("{role} account '{account_id}' does not match resolved network {network:?}")
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};
    use near_crypto::{KeyType, PublicKey, Signature};
    use near_primitives::action::{CreateAccountAction, FunctionCallAction, TransferAction};
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::{SignedTransaction, TransactionV0};
    use near_primitives::types::{Balance, Gas};
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
        NearTransaction::OnChain(Transaction::V0(TransactionV0 {
            signer_id: signer_id.parse().expect("valid account id"),
            public_key: PublicKey::empty(KeyType::ED25519),
            nonce: 0,
            receiver_id: receiver_id.parse().expect("valid account id"),
            block_hash: CryptoHash::default(),
            actions,
        }))
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
                    token_mappings: Default::default(),
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
                    token_mappings: Default::default(),
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

    /// The receiver renders as `To` alongside `Network`, so a receiver whose
    /// suffix contradicts the resolved network is the same contradiction as a
    /// signer's.
    #[test]
    fn rejects_testnet_receiver_under_resolved_mainnet_network() {
        let converter = NearVisualSignConverter::new();
        let tx = near_tx_as("alice.near", "bob.testnet", vec![transfer()]);
        let err = converter
            .to_visual_sign_payload(tx, VisualSignOptions::default())
            .expect_err("mainnet network with a .testnet receiver is rejected");
        let VisualSignError::ValidationError(message) = err else {
            panic!("expected ValidationError, got {err:?}");
        };
        assert!(message.contains("receiver account"), "message: {message}");
    }

    /// The suffix check is a convention heuristic, so a 64-hex implicit
    /// account -- which carries no suffix on either network -- must render
    /// rather than be rejected under whichever network is resolved.
    #[test]
    fn implicit_hex_account_is_not_rejected_by_the_suffix_check() {
        let implicit = "98793cd91a3f870fb126f66285808c7e094afcfc4eda8a970f6648cdf0dbd6de";
        for network in [NearNetwork::Mainnet, NearNetwork::Testnet] {
            let converter = NearVisualSignConverter::with_network(network);
            let tx = near_tx_as(implicit, implicit, vec![transfer()]);
            let result = converter.to_visual_sign_payload(tx, VisualSignOptions::default());
            assert!(result.is_ok(), "network: {network:?}");
        }
    }

    fn signed_transfer_hex() -> String {
        let unsigned = match near_tx(vec![transfer()]) {
            NearTransaction::OnChain(tx) => tx,
            NearTransaction::Intent(_) => panic!("expected OnChain"),
        };
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

    #[test]
    fn execute_intents_call_renders_intents() {
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});

        let txv0 = TransactionV0 {
            signer_id: "alice.near".parse().unwrap(),
            public_key: "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"
                .parse()
                .unwrap(),
            nonce: 1,
            receiver_id: "intents.near".parse().unwrap(),
            block_hash: CryptoHash::default(),
            actions: vec![Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "execute_intents".to_string(),
                args: serde_json::to_vec(&args).unwrap(),
                gas: Gas::from_gas(30_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }))],
        };
        let near_tx = NearTransaction::OnChain(Transaction::V0(txv0));
        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload(near_tx, VisualSignOptions::default())
            .expect("convert");
        let json = payload.payload.to_json().expect("json");

        // Generic FunctionCall view + decoded intents both present.
        assert!(json.contains("execute_intents"), "method missing: {json}");
        assert!(json.contains("Signer"), "intents envelope missing: {json}");
        assert!(
            json.contains("wNEAR"),
            "resolved ft_withdraw amount missing: {json}"
        );
    }

    /// Proves the `unverified-token-metadata` diagnostic (see
    /// `presets::intents::render::token_amount_field`) reaches the actual
    /// serialized payload from real `options.metadata`, not just the render
    /// helper in isolation: `options.metadata` -> `token_registry_for` ->
    /// `decode_intents` -> the field the signer sees.
    #[test]
    fn unsigned_gap_fill_metadata_surfaces_a_diagnostic_end_to_end() {
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"gap-fill-token.near","receiver_id":"bob.near","amount":"1000000"}]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});

        let txv0 = TransactionV0 {
            signer_id: "alice.near".parse().unwrap(),
            public_key: "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"
                .parse()
                .unwrap(),
            nonce: 1,
            receiver_id: "intents.near".parse().unwrap(),
            block_hash: CryptoHash::default(),
            actions: vec![Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "execute_intents".to_string(),
                args: serde_json::to_vec(&args).unwrap(),
                gas: Gas::from_gas(30_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }))],
        };
        let near_tx = NearTransaction::OnChain(Transaction::V0(txv0));

        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        "nep141:gap-fill-token.near".to_string(),
                        generated::parser::TokenMetadataEntry {
                            value: r#"{"symbol":"GAPFILL","decimals":6}"#.to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..Default::default()
        };

        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload(near_tx, options)
            .expect("convert");
        let json = payload.payload.to_json().expect("json");

        assert!(
            json.contains("GAPFILL"),
            "unsigned gap-fill entry must still resolve the symbol: {json}"
        );
        assert!(
            json.contains("unverified-token-metadata"),
            "unsigned gap-fill entry must carry its provenance into the render: {json}"
        );
    }

    /// Metadata the parser refuses must be visible in the payload, not just in
    /// an operator log.
    ///
    /// The refusal here is the `require-signed` posture dropping an unsigned
    /// entry. Without the diagnostic, the signer sees an amount in raw base
    /// units against an `unresolved` asset id and has no way to tell that
    /// metadata was supplied at all.
    #[test]
    fn rejected_metadata_surfaces_a_diagnostic_end_to_end() {
        let asset_id = "nep141:rejected-token.near";
        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        asset_id.to_string(),
                        generated::parser::TokenMetadataEntry {
                            value: r#"{"symbol":"DROPPED","decimals":6}"#.to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..Default::default()
        };

        let envelope = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"rejected-token.near","receiver_id":"bob.near","amount":"1000000"}]}"#.to_string();
        let payload = NearVisualSignConverter::with_trust_policy(
            MetadataTrustPolicy::RequireAllowlistedSigner(
                visualsign::signing::SignerAllowlist::new(),
            ),
        )
        .to_visual_sign_payload(NearTransaction::Intent(envelope), options)
        .expect("convert");
        let fields = &payload.payload.fields;

        assert!(
            fields.iter().any(
                |f| crate::presets::intents::test_support::is_warning_diagnostic(
                    f,
                    "rejected-token-metadata"
                )
            ),
            "a refused entry must report itself in the payload: {fields:?}"
        );
        let json = payload.payload.to_json().expect("json");
        assert!(
            !json.contains("DROPPED"),
            "the refused entry must not resolve the symbol: {json}"
        );
        assert!(
            json.contains(asset_id),
            "the diagnostic must name the asset id it refused: {json}"
        );
    }

    /// The rejection is a property of the request's metadata, so it reports
    /// once even when several actions consult the registry.
    #[test]
    fn rejected_metadata_reports_once_for_a_multi_action_transaction() {
        let asset_id = "nep141:rejected-token.near";
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"rejected-token.near","receiver_id":"bob.near","amount":"1000000"}]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});
        let call = || {
            Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "execute_intents".to_string(),
                args: serde_json::to_vec(&args).unwrap(),
                gas: Gas::from_gas(30_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }))
        };
        let txv0 = TransactionV0 {
            signer_id: "alice.near".parse().unwrap(),
            public_key: PublicKey::empty(KeyType::ED25519),
            nonce: 1,
            receiver_id: "intents.near".parse().unwrap(),
            block_hash: CryptoHash::default(),
            actions: vec![call(), call()],
        };

        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        asset_id.to_string(),
                        generated::parser::TokenMetadataEntry {
                            value: "not valid json".to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..Default::default()
        };

        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload(NearTransaction::OnChain(Transaction::V0(txv0)), options)
            .expect("convert");
        let count = payload
            .payload
            .fields
            .iter()
            .filter(|f| {
                crate::presets::intents::test_support::is_warning_diagnostic(
                    f,
                    "rejected-token-metadata",
                )
            })
            .count();
        assert_eq!(
            count, 1,
            "two actions must not each repeat the request's one rejection"
        );
    }

    /// A caller-controlled asset id cannot smuggle a newline into the rejection
    /// diagnostic, which would render as extra apparent fields on the signing
    /// screen. Same class as the `memo`/`msg`/`method_name` filtering in
    /// `actions.rs`, reached through a different field.
    #[test]
    fn rejected_metadata_diagnostic_strips_newlines_from_the_asset_id() {
        let hostile = "nep141:x.near\nAmount: 1000000 NEAR";
        let options = VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        hostile.to_string(),
                        generated::parser::TokenMetadataEntry {
                            value: "not valid json".to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..Default::default()
        };

        let envelope = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000"}]}"#;
        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload(NearTransaction::Intent(envelope.to_string()), options)
            .expect("convert");
        let json = payload.payload.to_json().expect("json");

        assert!(
            json.contains("rejected-token-metadata"),
            "the refusal must still be reported: {json}"
        );
        assert!(
            !json.contains("x.near\\nAmount"),
            "the newline must be filtered out of the rendered text: {json}"
        );
    }

    #[test]
    fn intent_envelope_renders_without_signature_section() {
        let swap = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2100-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}]}"#;
        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload(
                NearTransaction::Intent(swap.to_string()),
                VisualSignOptions::default(),
            )
            .expect("convert");
        let json = payload.payload.to_json().expect("json");
        for expected in ["Signer", "alice.near", "wNEAR"] {
            assert!(json.contains(expected), "missing {expected}: {json}");
        }
        assert!(!json.contains("Standard"), "unexpected signature section");
    }
}
