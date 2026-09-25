use std::fmt;

/// Everything that can go wrong reading enclave state or attesting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationError {
    /// The manifest file couldn't be read, was too large, or didn't decode.
    Manifest(String),
    /// The manifest decoded but couldn't be re-encoded for the boot proof.
    Encode(String),
    /// The ephemeral key couldn't be loaded.
    EphemeralKey(String),
    /// NSM returned something other than an attestation document.
    Nsm(String),
    /// The attestation doc didn't decode or verify against the AWS root.
    Certificate(String),
    /// The blocking NSM task panicked or exceeded its deadline.
    Task(String),
}

impl fmt::Display for AttestationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(e) => write!(f, "manifest: {e}"),
            Self::Encode(e) => write!(f, "manifest encode: {e}"),
            Self::EphemeralKey(e) => write!(f, "ephemeral key: {e}"),
            Self::Nsm(e) => write!(f, "nsm: {e}"),
            Self::Certificate(e) => write!(f, "attestation certificate: {e}"),
            Self::Task(e) => write!(f, "attestation task: {e}"),
        }
    }
}

impl std::error::Error for AttestationError {}
