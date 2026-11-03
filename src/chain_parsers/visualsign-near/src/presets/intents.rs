//! NEAR's entry to the NEAR Intents decoder.
//!
//! The decoder is [`visualsign_intents`], a crate with no chain dependency: the
//! same `DefusePayload` reaches NEAR through an `execute_intents` call, a
//! NEP-413 message or a raw message, and reaches Solana and Ethereum through
//! their own message envelopes.
//!
//! What is NEAR-specific is recognizing that a payload carries intents at all --
//! the `receiver_id`/`method_name` gate in [`crate::convert`] -- not the
//! decoding that follows it.
pub use visualsign_intents::*;
