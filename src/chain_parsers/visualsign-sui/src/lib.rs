// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! Convert Sui transactions into `VisualSign` payloads and visualize protocol-specific commands.
#![warn(clippy::all, clippy::pedantic)]

mod core;
mod integrations;
/// Structured Sui decode emitted as the conversion's intermediate output.
/// Published so consumers can `borsh::from_slice` into the same schema the
/// parser produces.
pub mod intermediate;
mod presets;
mod types;
mod utils;

pub use core::{
    SuiTransactionWrapper, SuiVisualSignConverter, VisualizeResult,
    transaction_string_to_visual_sign, transaction_to_visual_sign,
};
/// The `sui-sdk-types` crate whose `Transaction` the public API takes.
pub use sui_sdk_types;

#[allow(unused_imports)]
pub(crate) use utils::truncate_address;

#[cfg(test)]
#[path = "../tests/oracle/mod.rs"]
mod differential_oracle;
