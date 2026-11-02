use clap::Args as ClapArgs;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// CLI arguments specific to Zcash.
///
/// The PCZT's coin type is displayed as a declared, unsigned network.
/// This plugin does not provide a caller network pin.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct ZcashArgs {}

/// [`parser_cli_core::ChainPlugin`] implementation for Zcash.
pub struct ZcashPlugin {
    #[allow(dead_code)]
    args: ZcashArgs,
}

impl ZcashPlugin {
    /// Creates a new `ZcashPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: ZcashArgs) -> Self {
        Self { args }
    }
}

impl parser_cli_core::ChainPlugin for ZcashPlugin {
    fn chain(&self) -> Chain {
        Chain::Zcash
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::ZcashTransactionWrapper, _>(
            Chain::Zcash,
            crate::ZcashVisualSignConverter,
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        Ok(None)
    }
}
