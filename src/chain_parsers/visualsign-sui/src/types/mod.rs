//! Lightweight Sui transaction types.
//!
//! Transactions are decoded with `sui-sdk-types` (pure Rust BCS types, no
//! cryptography backends) and converted into the view in [`block`], which keeps
//! the `sui_json_rpc_types` shapes the presets match on.

// The view keeps the accessors and constructors of the types it replaces; some
// are only used by tests.
#![allow(dead_code, unused_imports)]

mod base;
mod block;
mod display;
mod json;

pub use base::{AddressParseError, ObjectDigest, ObjectID, SequenceNumber, SuiAddress};
pub use block::{
    ConversionError, SuiArgument, SuiCallArg, SuiCommand, SuiFundsWithdrawalArg, SuiGasData,
    SuiObjectArg, SuiProgrammableMoveCall, SuiProgrammableTransactionBlock, SuiPureValue,
    SuiReservation, SuiTransactionBlockData, SuiTransactionBlockKind, SuiWithdrawFrom,
    SuiWithdrawalTypeArg,
};
pub use display::{type_tag_to_canonical_string, type_tag_to_string};
pub use json::SuiJsonValue;
pub use sui_sdk_types::TypeTag;

/// MIST per SUI.
pub const MIST_PER_SUI: u64 = 1_000_000_000;
