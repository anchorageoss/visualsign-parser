//! Parser HTTP gateway — library entrypoint so integration tests can
//! construct the same router the binary serves.
// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

pub mod attestation;
pub mod auth;
pub mod handlers;
pub mod signing;
pub mod state;
pub mod x402_config;

// Re-export so existing `parser_gateway::turnkey::*` paths keep working.
// Actual types live in `host_primitives::turnkey` so other binaries
// (notably `parser_http_server`) can use the same Turnkey wire envelope.
pub use host_primitives::turnkey;
