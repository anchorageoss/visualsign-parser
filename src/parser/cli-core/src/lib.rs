#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

//! Reusable CLI scaffolding for the `VisualSign` Parser. External workspaces
//! depend on this crate to compose their own `parser_cli` binary with a custom
//! set of chain plugins.
