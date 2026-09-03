//! Parser HTTP gateway -- library entrypoint so integration tests can
//! construct the same router the binary serves.

pub mod attestation;
pub mod auth;
mod env_util;
pub mod handlers;
pub mod state;
pub mod x402_config;
