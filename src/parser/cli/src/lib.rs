// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! `VisualSign` Parser
#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

/// Command-line interface functionality and types.
pub mod cli;
/// Ethereum-specific CLI handling: ABI mappings, network metadata.
#[cfg(feature = "ethereum")]
pub mod ethereum;
/// Solana-specific CLI handling: IDL mappings, Solana metadata.
#[cfg(feature = "solana")]
pub mod solana;
/// Tron-specific CLI handling.
#[cfg(feature = "tron")]
pub mod tron;

/// Constructs all enabled chain plugins, each pre-loaded with its CLI args.
///
/// **To add a new chain:** create its module, implement [`parser_cli_core::ChainPlugin`],
/// then add one entry here (behind its feature flag) and one
/// `#[command(flatten)]` field to `cli::Args`.
#[must_use]
#[allow(clippy::vec_init_then_push)] // cfg-gated pushes cannot be expressed as vec![...]
pub(crate) fn build_plugins(args: &cli::Args) -> Vec<Box<dyn parser_cli_core::ChainPlugin>> {
    let mut plugins: Vec<Box<dyn parser_cli_core::ChainPlugin>> = vec![];

    #[cfg(feature = "ethereum")]
    plugins.push(Box::new(ethereum::EthereumPlugin::new(
        args.ethereum.clone(),
    )));

    #[cfg(feature = "solana")]
    plugins.push(Box::new(solana::SolanaPlugin::new(args.solana.clone())));

    #[cfg(feature = "tron")]
    plugins.push(Box::new(tron::TronPlugin::new(args.tron.clone())));

    plugins
}
