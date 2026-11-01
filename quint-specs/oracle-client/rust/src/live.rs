//! The live client: everything compiled only under the `enabled` feature.
//!
//! Re-exported wholesale at the crate root, where [`crate::stubs`] takes its
//! place when the feature is off — so both configurations present one identical
//! public surface.

mod event;
mod registry;
mod transport;
mod value;

pub use event::{Event, EventBuilder, PathSeg};
pub use itf::Value;
pub use registry::{
    __record_outcome, __register_for, auto_register_test, current_test, register_test, TestGuard,
    TestHandle, TestOutcome,
};
pub use value::{record, tuple, ToLogged};

/// Whether instrumentation is live: compiled with the `enabled` feature AND
/// `QUINT_ORACLE_URL` set (read once, on first call).
pub fn enabled() -> bool {
    config().is_some()
}

/// Oracle daemon endpoint, from `QUINT_ORACLE_URL`.
pub(crate) struct Config {
    /// Base URL with any trailing `/` removed.
    pub base_url: String,
}

pub(crate) fn config() -> Option<&'static Config> {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<Option<Config>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var("QUINT_ORACLE_URL")
                .ok()
                .map(|url| url.trim().trim_end_matches('/').to_string())
                .filter(|url| !url.is_empty())
                .map(|base_url| Config { base_url })
        })
        .as_ref()
}
