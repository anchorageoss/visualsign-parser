//! NEAR chain parser for VisualSign.
//!
//! Decodes the payloads a NEAR user is asked to sign and renders them as
//! human-readable [`visualsign::SignablePayload`] fields.

pub mod actions;
pub mod convert;
pub mod fmt;
pub mod networks;
pub mod tx;

pub use convert::NearVisualSignConverter;
pub use tx::NearTransaction;
