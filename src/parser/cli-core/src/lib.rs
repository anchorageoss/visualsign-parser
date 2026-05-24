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
