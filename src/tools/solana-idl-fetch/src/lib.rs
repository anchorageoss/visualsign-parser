//! Library split of the `solana-idl-fetch` binary, so every pure piece
//! (address derivation, account decoding, record building and validation,
//! the compare summary, the JSON formatting) is testable without network.
//!
//! `src/main.rs` is a thin wrapper over [`cli::main`].

pub mod cli;
pub mod compare;
pub mod json;
pub mod record;
pub mod rpc;
pub mod solana;
