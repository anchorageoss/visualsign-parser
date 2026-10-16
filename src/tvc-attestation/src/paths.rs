//! Where QOS leaves enclave state for the pivot.
//!
//! QOS 0.12.1 made `qos_core::{MANIFEST_FILE, EPHEMERAL_KEY_FILE}`
//! unconditionally absolute and removed the `vm` feature that used to switch
//! them to `./local-enclave/...` for local runs. These keep that split so local
//! dev and integration tests (which write `./local-enclave/qos.ephemeral.key`)
//! behave as before.

/// The QOS manifest envelope.
#[cfg(feature = "enclave")]
pub const MANIFEST_FILE: &str = qos_core::MANIFEST_FILE;
/// The QOS manifest envelope.
#[cfg(not(feature = "enclave"))]
pub const MANIFEST_FILE: &str = "./local-enclave/qos.manifest";

/// The ephemeral key QOS provisions for the pivot (rotated from the setup key
/// to the live key after provisioning).
#[cfg(feature = "enclave")]
pub const EPHEMERAL_KEY_FILE: &str = qos_core::EPHEMERAL_KEY_FILE;
/// The ephemeral key QOS provisions for the pivot (rotated from the setup key
/// to the live key after provisioning).
#[cfg(not(feature = "enclave"))]
pub const EPHEMERAL_KEY_FILE: &str = "./local-enclave/qos.ephemeral.key";
