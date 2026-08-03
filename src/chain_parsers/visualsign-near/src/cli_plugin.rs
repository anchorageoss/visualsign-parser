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

/// The CLI always runs the strict `RequireAllowlistedSigner` posture,
/// matching `visualsign-ethereum`'s CLI plugin (see its `cli_trust_policy`).
/// The wrapped allowlist is inert here: unlike Ethereum's single-allowlist
/// ABI path, NEAR already dispatches identity checks for a present signature
/// per origin chain (see `authorized_token_metadata_signers`), so only the
/// posture this variant selects -- reject any entry with no signature at
/// all -- is consulted.
fn cli_trust_policy() -> visualsign::signing::MetadataTrustPolicy {
    visualsign::signing::MetadataTrustPolicy::RequireAllowlistedSigner(
        visualsign::signing::SignerAllowlist::new(),
    )
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
    use parser_cli_core::ChainPlugin;

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
}
