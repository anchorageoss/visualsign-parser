//! What a Turnkey Verifiable Cloud (TVC) pivot needs to attest itself.
//!
//! Nothing here is visualsign-specific: it's QOS (QuorumOS) enclave state,
//! AWS Nitro NSM attestation, and Turnkey's boot-proof encoding. Built against
//! QOS 0.12.1 (what TVC boots):
//! - [`manifest`]: decode `/qos.manifest` in any QOS schema (v0/v1/v2) and
//!   encode it for the boot proof (borsh for v0/v1, storage JSON for v2).
//! - [`cert`]: verify an attestation doc against the AWS Nitro root at its own
//!   NSM timestamp and read its chain's earliest `notAfter`.
//! - [`cache`]: a single-flight attestation cache keyed on (ephemeral key,
//!   manifest hash), refreshed on change or near expiry, with a health signal.
//! - [`paths`]: where those files live in-enclave vs. local dev.

mod error;

pub mod cache;
pub mod cert;
pub mod manifest;
pub mod paths;

pub use error::AttestationError;
