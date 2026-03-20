//! `VisualSign` Parser
#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::VisualSignOptions;

/// Chain-related functionality and types.
pub mod chains;
/// Command-line interface functionality and types.
pub mod cli;
/// Ethereum-specific CLI handling: ABI registry, network metadata.
#[cfg(feature = "ethereum")]
pub mod ethereum;
/// Common mapping parser for ABI and IDL file mappings.
pub mod mapping_parser;
/// Solana-specific CLI handling: IDL mappings, Solana metadata.
#[cfg(feature = "solana")]
pub mod solana;
#[cfg(test)]
pub(crate) mod test_utils;

/// Trait for integrating a chain into the CLI.
///
/// Implement this in a chain module, then register it in [`build_plugins`].
/// Each method has a sensible default so only relevant behaviour needs overriding.
pub trait ChainPlugin {
    /// The chain this plugin handles.
    fn chain(&self) -> Chain;

    /// Register the chain's converter in the registry.
    fn register(&self, registry: &mut TransactionConverterRegistry);

    /// Build chain-specific metadata. `network` is the shared `--network` flag;
    /// any chain-specific args (e.g. ABI or IDL mappings) are owned by the plugin.
    fn create_metadata(&self, network: Option<String>) -> Option<ChainMetadata>;

    /// Post-process [`VisualSignOptions`] after metadata is set.
    /// Default: pass through unchanged.
    fn apply_options(&self, options: VisualSignOptions) -> VisualSignOptions {
        options
    }
}

/// Constructs all enabled chain plugins, each pre-loaded with its CLI args.
///
/// **To add a new chain:** create its module, implement [`ChainPlugin`],
/// then add one entry here (behind its feature flag) and one
/// `#[command(flatten)]` field to `cli::Args`.
#[must_use]
#[allow(clippy::vec_init_then_push)] // cfg-gated pushes cannot be expressed as vec![...]
pub(crate) fn build_plugins(args: &cli::Args) -> Vec<Box<dyn ChainPlugin>> {
    let mut plugins: Vec<Box<dyn ChainPlugin>> = vec![];

    #[cfg(feature = "ethereum")]
    plugins.push(Box::new(ethereum::EthereumPlugin::new(
        args.ethereum.clone(),
    )));

    #[cfg(feature = "solana")]
    plugins.push(Box::new(solana::SolanaPlugin::new(args.solana.clone())));

    plugins
}
