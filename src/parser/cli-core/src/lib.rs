#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

//! Reusable CLI scaffolding for the `VisualSign` Parser. External workspaces
//! depend on this crate to compose their own `parser_cli` binary with a custom
//! set of chain plugins.

/// Chain enum parsing and conversion.
pub mod chains;
/// ABI/IDL mapping file parser shared across chain plugins.
pub mod mapping_parser;
/// Resolution of the `--transaction` argument, including curl-style `@` references.
pub mod tx_input;

/// Shared test helpers (temp file creation, etc.).
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
