//! `VisualSign` Parser
#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

/// Chain-related functionality and types.
pub mod chains;
/// Command-line interface functionality and types.
pub mod cli;
/// Common mapping parser for ABI and IDL file mappings.
pub mod mapping_parser;
