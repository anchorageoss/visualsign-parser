use clap::Args as ClapArgs;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// CLI arguments specific to Tron.
///
/// Currently no Tron-specific args are needed; the global `--network` flag is
/// accepted but not used (the Tron parser has no network metadata plumbing
/// today, mirroring the Solana plugin's behaviour).
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct TronArgs {}

/// [`crate::ChainPlugin`] implementation for Tron.
pub struct TronPlugin {
    // `TronArgs` is currently empty, but we still hold it so adding a Tron-specific flag
    // later doesn't require a struct shape change — matches the EthereumPlugin/SolanaPlugin
    // convention of carrying the parsed args.
    #[allow(dead_code)]
    args: TronArgs,
}

impl TronPlugin {
    /// Creates a new `TronPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: TronArgs) -> Self {
        Self { args }
    }
}

impl crate::ChainPlugin for TronPlugin {
    fn chain(&self) -> Chain {
        Chain::Tron
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<visualsign_tron::TronTransactionWrapper, _>(
            Chain::Tron,
            visualsign_tron::TronVisualSignConverter,
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        Ok(None)
    }
}
