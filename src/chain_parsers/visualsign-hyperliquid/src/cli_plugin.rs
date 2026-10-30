use clap::Args as ClapArgs;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::ChainMetadata;

/// CLI arguments specific to Hyperliquid.
///
/// None yet: the network comes from the signed payload itself (`source` for L1
/// actions, `hyperliquidChain` for user-signed ones), not from a flag.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct HyperliquidArgs {}

/// [`parser_cli_core::ChainPlugin`] implementation for Hyperliquid.
pub struct HyperliquidPlugin {
    #[allow(dead_code)]
    args: HyperliquidArgs,
}

impl HyperliquidPlugin {
    /// Creates a new `HyperliquidPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: HyperliquidArgs) -> Self {
        Self { args }
    }
}

impl parser_cli_core::ChainPlugin for HyperliquidPlugin {
    fn chain(&self) -> Chain {
        Chain::Hyperliquid
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::HyperliquidTransaction, _>(
            Chain::Hyperliquid,
            crate::HyperliquidVisualSignConverter,
        );
    }

    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        match network {
            None => Ok(None),
            Some(n) => Err(format!(
                "--network {n} is not used for hyperliquid: the network is read from the signed payload"
            )),
        }
    }
}
