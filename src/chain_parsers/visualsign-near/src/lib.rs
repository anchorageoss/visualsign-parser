//! NEAR chain parser for VisualSign.
//!
//! Decodes the payloads a NEAR user is asked to sign and renders them as
//! human-readable [`visualsign::SignablePayload`] fields.

pub mod actions;
#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;
pub mod convert;
pub mod fmt;
pub mod networks;
pub mod presets;
pub mod tx;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{NearArgs, NearPlugin};
pub use convert::NearVisualSignConverter;
pub use tx::NearTransaction;
