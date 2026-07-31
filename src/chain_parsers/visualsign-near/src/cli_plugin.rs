use clap::Args as ClapArgs;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// CLI arguments specific to NEAR.
///
/// Currently no NEAR-specific args are needed; the global `--network` flag is
/// accepted but not used (the NEAR parser has no network metadata plumbing
/// today, mirroring the Solana/Tron plugins' behaviour).
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

impl parser_cli_core::ChainPlugin for NearPlugin {
    fn chain(&self) -> Chain {
        Chain::Near
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::NearTransaction, _>(
            Chain::Near,
            crate::NearVisualSignConverter::new(),
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        Ok(None)
    }
}
