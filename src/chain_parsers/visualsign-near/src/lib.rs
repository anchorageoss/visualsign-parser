//! NEAR chain parser for VisualSign.
//!
//! Decodes the payloads a NEAR user is asked to sign and renders them as
//! human-readable [`visualsign::SignablePayload`] fields.

use thiserror::Error;

/// Errors produced while parsing NEAR payloads.
#[derive(Debug, Error)]
pub enum NearParserError {
    /// The input bytes did not borsh-decode as a NEAR transaction.
    #[error("Failed to borsh-decode NEAR transaction: {0}")]
    BorshDecode(String),
    /// The transaction decoded but left unconsumed input.
    #[error("Unexpected trailing bytes after transaction ({0} bytes)")]
    TrailingData(usize),
    /// The transaction contains an action variant this parser does not render.
    #[error("Unsupported action variant: {0}")]
    UnsupportedAction(&'static str),
}
