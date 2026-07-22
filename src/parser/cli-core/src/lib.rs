#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

//! Reusable CLI scaffolding for the `VisualSign` Parser. External workspaces
//! depend on this crate to compose their own `parser_cli` binary with a custom
//! set of chain plugins.

use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};

/// Chain enum parsing and conversion.
pub mod chains;
/// ABI/IDL mapping file parser shared across chain plugins.
pub mod mapping_parser;
/// Resolution of the `--transaction` argument, including curl-style `@` references.
pub mod tx_input;

/// Shared test helpers (temp file creation, etc.).
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

/// Output formatting and the parse-and-display entry point.
pub mod output;

pub use output::OutputFormat;

/// Trait for integrating a chain into the CLI.
///
/// Implement this in a chain crate (e.g. `visualsign-ethereum::cli_plugin::EthereumPlugin`),
/// then the binary composes a `Vec<Box<dyn ChainPlugin>>` and hands it to [`run`].
pub trait ChainPlugin {
    /// The chain this plugin handles.
    fn chain(&self) -> Chain;

    /// Register the chain's converter in the registry.
    fn register(&self, registry: &mut TransactionConverterRegistry);

    /// Build chain-specific metadata from the shared `--network` flag and any
    /// chain-specific args owned by the plugin.
    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String>;
}

use clap::Args as ClapArgs;
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};

/// CLI flags that every chain shares.
#[derive(ClapArgs, Debug)]
pub struct SharedArgs {
    /// Chain type (e.g., ethereum, solana, near).
    #[arg(short, long)]
    pub chain: String,

    /// Raw transaction string. Prefix with '@' to read from a file
    /// (e.g. '@/path/to/tx.hex'), or use '@-' to read from stdin.
    #[arg(short, long, value_name = "RAW_TX")]
    pub transaction: String,

    /// Output format.
    #[arg(short, long, default_value = "text")]
    pub output: OutputFormat,

    /// Show only condensed view (what hardware wallets display).
    #[arg(long)]
    pub condensed_only: bool,

    /// Request and display the chain-specific `intermediate_output` blob (the
    /// borsh-serialized structured decode used by downstream policy engines).
    /// Prints the raw bytes as hex so they can be captured as a test fixture.
    #[arg(long)]
    pub with_intermediate: bool,

    /// Network identifier (chain ID or canonical name).
    #[arg(
        long,
        short = 'n',
        value_name = "NETWORK",
        long_help = "Network identifier - chain ID or canonical name.\n\
                     Chain ID: 1, 137, 42161, etc.\n\
                     Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET, etc."
    )]
    pub network: Option<String>,
}

/// A resolved registry plus the visual-sign options for a chain, ready to decode
/// transactions. Built by [`prepare_runtime`] and shared by [`run`] (the `decode`
/// path) and any other consumer that decodes many transactions for one chain
/// (e.g. the binary's `serve` subcommand).
pub struct Runtime {
    /// Registry with every plugin's converter registered.
    pub registry: TransactionConverterRegistry,
    /// Visual-sign options carrying the selected chain's metadata.
    pub options: VisualSignOptions,
}

/// Resolve `chain_str` against `plugins`: register every plugin, select the one
/// whose `chain()` matches, and build its [`VisualSignOptions`] from `network`.
///
/// On an unknown or unsupported chain, returns an `Err(String)` with a
/// user-facing message — the caller prints it and exits non-zero.
pub fn prepare_runtime(
    chain_str: &str,
    network: Option<String>,
    plugins: &[Box<dyn ChainPlugin>],
    include_intermediate_output: bool,
) -> Result<Runtime, String> {
    let chain = chains::parse_chain(chain_str);
    let mut registry = TransactionConverterRegistry::new();
    for plugin in plugins {
        plugin.register(&mut registry);
    }
    let plugin = plugins.iter().find(|p| p.chain() == chain).ok_or_else(|| {
        let supported: Vec<String> = plugins
            .iter()
            .map(|p| p.chain().as_str().to_lowercase())
            .collect();
        let supported_str = if supported.is_empty() {
            "none".to_string()
        } else {
            supported.join(", ")
        };
        if chain == Chain::Unspecified {
            format!("unrecognized chain '{chain_str}'.\nSupported chains: {supported_str}")
        } else {
            format!(
                "chain '{chain_str}' is not supported by this CLI build.\nSupported chains: {supported_str}"
            )
        }
    })?;

    let chain_metadata = plugin.create_metadata(network)?;
    let options = VisualSignOptions {
        include_intermediate_output,
        decode_transfers: true,
        transaction_name: None,
        metadata: chain_metadata,
        developer_config: Some(DeveloperConfig {
            allow_signed_transactions: true,
        }),
    };

    Ok(Runtime { registry, options })
}

/// CLI entry point. Pass the shared args plus an ordered list of chain plugins.
/// The first plugin whose `chain()` matches `shared.chain` handles the transaction.
pub fn run(shared: &SharedArgs, plugins: &[Box<dyn ChainPlugin>]) -> Result<(), String> {
    let Runtime { registry, options } = prepare_runtime(
        &shared.chain,
        shared.network.clone(),
        plugins,
        shared.with_intermediate,
    )?;

    let raw_tx =
        tx_input::resolve_transaction_input(&shared.transaction).map_err(|e| e.to_string())?;

    output::parse_and_display(
        &shared.chain,
        &raw_tx,
        &registry,
        options,
        shared.output,
        shared.condensed_only,
    )
}
