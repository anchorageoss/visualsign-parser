//! Hyperliquid parser for VisualSign.
//!
//! Hyperliquid L1 actions (orders, cancels, transfers, deploys) are signed as an
//! EIP-712 `Agent` struct whose only content is `connectionId`, a keccak hash of the
//! msgpack-encoded action. A wallet that shows the typed data shows nothing an
//! approver can check. This crate takes the typed data together with the action it
//! claims to commit to, recomputes the hash from the action's exact bytes, refuses on
//! any mismatch, and renders the action.
//!
//! User-signed actions (`HyperliquidTransaction:*`, and `Hyperliquid:AcceptTerms`) are
//! plain EIP-712 with readable fields; they are rendered from the typed data after
//! pinning the domain and types.
//!
//! Everything not on an allowlist is refused: unknown action types, unknown
//! user-signed primary types, and unknown fields.

#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;
pub mod hash;
pub mod input;
pub mod l1;
mod render;
pub mod tx;
pub mod user_signed;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{HyperliquidArgs, HyperliquidPlugin};
pub use tx::{HyperliquidTransaction, HyperliquidVisualSignConverter};
