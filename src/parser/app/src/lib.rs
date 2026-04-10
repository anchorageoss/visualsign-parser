// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! `VisualSign` Parser
#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::unwrap_used)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub mod cli;

pub mod host;

pub mod service;

pub mod errors;

pub mod chain_conversion;

pub mod registry;

/// Routes for the parser service
pub mod routes {
    /// Parse route
    pub mod parse;
}
