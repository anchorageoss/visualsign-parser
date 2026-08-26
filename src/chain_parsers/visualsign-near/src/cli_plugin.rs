use clap::Args as ClapArgs;
use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};
use visualsign::registry::{Chain, TransactionConverterRegistry};

use crate::networks::NearNetwork;

/// CLI arguments specific to NEAR.
///
/// Currently no NEAR-specific args are needed beyond the global `--network`
/// flag, which `create_metadata` below turns into `NearMetadata`.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct NearArgs {}

/// [`parser_cli_core::ChainPlugin`] implementation for NEAR.
pub struct NearPlugin {
    #[allow(dead_code)]
    args: NearArgs,
}

impl NearPlugin {
    /// Creates a new `NearPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: NearArgs) -> Self {
        Self { args }
    }
}

/// The CLI always runs the strict posture -- every token-metadata entry must
/// carry a signature -- matching `visualsign-ethereum`'s CLI plugin (see its
/// `cli_trust_policy`). Signer identity for a present signature is checked per
/// origin chain against `authorized_token_metadata_signers`, which is why this
/// posture carries no allowlist of its own.
fn cli_trust_policy() -> crate::presets::intents::NearTokenTrustPolicy {
    crate::presets::intents::NearTokenTrustPolicy::RequireSignedEntries
}

impl parser_cli_core::ChainPlugin for NearPlugin {
    fn chain(&self) -> Chain {
        Chain::Near
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::NearTransaction, _>(
            Chain::Near,
            crate::NearVisualSignConverter::with_trust_policy(cli_trust_policy()),
        );
    }

    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        let Some(network) = network else {
            return Ok(None);
        };
        if NearNetwork::from_network_id(&network).is_none() {
            return Err(format!(
                "Invalid network '{network}'. Supported: NEAR_MAINNET, NEAR_TESTNET"
            ));
        }
        Ok(Some(ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(network.to_uppercase()),
                token_mappings: Default::default(),
            })),
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use generated::parser::TokenMetadataEntry;
    use near_crypto::{KeyType, PublicKey};
    use near_primitives::action::{Action, FunctionCallAction};
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::{Transaction, TransactionV0};
    use near_primitives::types::{Balance, Gas};
    use parser_cli_core::ChainPlugin;
    use visualsign::vsptrait::VisualSignOptions;

    fn plugin() -> NearPlugin {
        NearPlugin::new(NearArgs {})
    }

    #[test]
    fn create_metadata_defaults_to_none_without_a_network_flag() {
        assert_eq!(plugin().create_metadata(None).unwrap(), None);
    }

    #[test]
    fn create_metadata_builds_near_metadata_for_a_valid_network() {
        let metadata = plugin()
            .create_metadata(Some("near_testnet".to_string()))
            .unwrap()
            .expect("Some(ChainMetadata)");
        assert_eq!(
            metadata,
            ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_TESTNET".to_string()),
                    token_mappings: Default::default(),
                })),
            }
        );
    }

    #[test]
    fn create_metadata_rejects_an_invalid_network() {
        assert!(plugin().create_metadata(Some("bogus".to_string())).is_err());
    }

    /// The posture must be pinned through `register`, not through the helper that
    /// feeds it.
    ///
    /// Asserting on `cli_trust_policy()` in isolation proves nothing about what the
    /// CLI actually runs: editing `register` back to
    /// `NearVisualSignConverter::new()` leaves such an assertion passing and
    /// silently reverts the CLI to accept-unsigned. So this goes through the
    /// plugin, converts a real transaction, and observes the decode.
    ///
    /// `test-token.near` is deliberately not in `tokens::SEEDS`, so the unsigned
    /// metadata entry is the only thing that could resolve its symbol. The
    /// accept-unsigned control converter proves the fixture is good, so the
    /// registered converter's refusal cannot be a false negative.
    #[test]
    fn test_register_installs_require_signed_posture() {
        let asset_id = "nep141:test-token.near";
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"test-token.near","receiver_id":"bob.near","amount":"1000000"}]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});

        let build_tx_hex = || {
            let tx = Transaction::V0(TransactionV0 {
                signer_id: "alice.near".parse().unwrap(),
                public_key: PublicKey::empty(KeyType::ED25519),
                nonce: 1,
                receiver_id: "intents.near".parse().unwrap(),
                block_hash: CryptoHash::default(),
                actions: vec![Action::FunctionCall(Box::new(FunctionCallAction {
                    method_name: "execute_intents".to_string(),
                    args: serde_json::to_vec(&args).unwrap(),
                    gas: Gas::from_gas(30_000_000_000_000),
                    deposit: Balance::from_yoctonear(0),
                }))],
            });
            hex::encode(borsh::to_vec(&tx).unwrap())
        };

        // Deliberately unsigned: this is the entry the CLI's posture must refuse.
        let build_options = || VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        asset_id.to_string(),
                        TokenMetadataEntry {
                            value: r#"{"symbol":"UNVERIFIED","decimals":6}"#.to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..VisualSignOptions::default()
        };

        // Control: a converter on the permissive posture resolves the same fixture.
        let mut permissive_registry = TransactionConverterRegistry::new();
        permissive_registry.register::<crate::NearTransaction, _>(
            Chain::Near,
            crate::NearVisualSignConverter::new(),
        );
        let permissive = permissive_registry
            .convert_transaction(&Chain::Near, &build_tx_hex(), build_options())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(
            permissive.contains("UNVERIFIED"),
            "accept-unsigned must resolve the unsigned metadata symbol, got: {permissive}"
        );

        // The converter the CLI plugin actually installs must refuse it.
        let mut registry = TransactionConverterRegistry::new();
        plugin().register(&mut registry);
        let rendered = registry
            .convert_transaction(&Chain::Near, &build_tx_hex(), build_options())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(
            !rendered.contains("UNVERIFIED"),
            "the converter `register` installs must not resolve an unsigned metadata symbol, got: {rendered}"
        );
        assert!(
            rendered.contains(&format!("unresolved {asset_id}")),
            "dropping the unsigned entry must leave the raw asset id unresolved, got: {rendered}"
        );
    }
}
