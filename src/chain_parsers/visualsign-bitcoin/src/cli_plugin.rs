use bitcoin::Network;
use clap::Args as ClapArgs;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// CLI arguments specific to Bitcoin.
///
/// A PSBT does not say which network it targets, and the parser has no Bitcoin
/// chain-metadata message yet, so the address encoding is selected here.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct BitcoinArgs {
    /// Network used to encode output addresses: bitcoin, testnet, testnet4, signet or regtest.
    #[arg(long, default_value = "bitcoin")]
    pub bitcoin_network: String,
}

/// [`parser_cli_core::ChainPlugin`] implementation for Bitcoin.
pub struct BitcoinPlugin {
    args: BitcoinArgs,
}

impl BitcoinPlugin {
    /// Creates a new `BitcoinPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: BitcoinArgs) -> Self {
        Self { args }
    }
}

impl parser_cli_core::ChainPlugin for BitcoinPlugin {
    fn chain(&self) -> Chain {
        Chain::Bitcoin
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        // `register` cannot fail, so a bad flag value is reported by `create_metadata`
        // and mainnet is used here.
        let network = crate::parse_network(&self.args.bitcoin_network).unwrap_or(Network::Bitcoin);
        registry.register::<crate::BitcoinTransactionWrapper, _>(
            Chain::Bitcoin,
            crate::BitcoinVisualSignConverter::new(network),
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        // `register` cannot fail, so validate the flag here, where an error is reported.
        crate::parse_network(&self.args.bitcoin_network)?;
        Ok(None)
    }
}
