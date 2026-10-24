use clap::Args as ClapArgs;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// CLI arguments specific to Cosmos.
///
/// None today; the global `--network` flag is accepted but unused (a sign doc
/// carries its own chain ID, so there is no chain metadata to plumb).
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct CosmosArgs {}

/// [`parser_cli_core::ChainPlugin`] implementation for Cosmos.
pub struct CosmosPlugin {
    // Held so adding a Cosmos-specific flag later does not change the struct
    // shape, matching the other plugins.
    #[allow(dead_code)]
    args: CosmosArgs,
}

impl CosmosPlugin {
    /// Creates a new `CosmosPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: CosmosArgs) -> Self {
        Self { args }
    }
}

impl parser_cli_core::ChainPlugin for CosmosPlugin {
    fn chain(&self) -> Chain {
        Chain::Cosmos
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::CosmosTransaction, _>(
            Chain::Cosmos,
            crate::CosmosVisualSignConverter,
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        Ok(None)
    }
}
