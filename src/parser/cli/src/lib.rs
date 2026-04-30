// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! `VisualSign` Parser
#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// Chain-related functionality and types.
pub mod chains;
/// Command-line interface functionality and types.
pub mod cli;
/// Ethereum-specific CLI handling: ABI mappings, network metadata.
#[cfg(feature = "ethereum")]
pub mod ethereum;
/// Common mapping parser for ABI and IDL file mappings.
pub mod mapping_parser;
/// Local web UI for browsing decoded transactions from a directory.
#[cfg(feature = "serve")]
pub mod serve;
/// Solana-specific CLI handling: IDL mappings, Solana metadata.
#[cfg(feature = "solana")]
pub mod solana;
/// Shared test helpers (temp file creation, etc.).
#[cfg(test)]
pub(crate) mod test_utils;
/// Resolution of the `--transaction` argument, including curl-style `@` references.
pub mod tx_input;

/// Trait for integrating a chain into the CLI.
///
/// Implement this in a chain module, then register it in [`build_plugins`].
pub trait ChainPlugin {
    /// The chain this plugin handles.
    fn chain(&self) -> Chain;

    /// Register the chain's converter in the registry.
    fn register(&self, registry: &mut TransactionConverterRegistry);

    /// Build chain-specific metadata. `network` is the shared `--network` flag;
    /// any chain-specific args (e.g. ABI or IDL mappings) are owned by the plugin.
    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String>;
}

/// Per-chain CLI args needed to construct plugins. Populated from whichever
/// subcommand is running (`decode`, `serve`, …).
#[derive(Debug, Clone, Default)]
pub(crate) struct PluginArgs {
    #[cfg(feature = "ethereum")]
    pub ethereum: ethereum::EthereumArgs,
    #[cfg(feature = "solana")]
    pub solana: solana::SolanaArgs,
}

/// Constructs all enabled chain plugins, each pre-loaded with its CLI args.
///
/// **To add a new chain:** create its module, implement [`ChainPlugin`],
/// then add one entry here (behind its feature flag) and one
/// `#[command(flatten)]` field to whichever subcommand args struct uses it.
#[must_use]
#[allow(clippy::vec_init_then_push)] // cfg-gated pushes cannot be expressed as vec![...]
pub(crate) fn build_plugins(args: &PluginArgs) -> Vec<Box<dyn ChainPlugin>> {
    let mut plugins: Vec<Box<dyn ChainPlugin>> = vec![];

    #[cfg(feature = "ethereum")]
    plugins.push(Box::new(ethereum::EthereumPlugin::new(
        args.ethereum.clone(),
    )));

    #[cfg(feature = "solana")]
    plugins.push(Box::new(solana::SolanaPlugin::new(args.solana.clone())));

    plugins
}
