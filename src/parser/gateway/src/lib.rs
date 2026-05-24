//! Parser HTTP gateway — library entrypoint so integration tests can
//! construct the same router the binary serves.
// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

pub mod attestation;
pub mod handlers;
pub mod state;
pub mod turnkey;
pub mod x402_config;
