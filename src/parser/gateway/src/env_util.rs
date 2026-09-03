//! Shared env-var reading helper.
//!
//! `std::env::var(..).ok()` collapses "unset" and "set but not valid UTF-8"
//! into the same `None`, which would silently fall through to the unset-var
//! default for a malformed value instead of reporting invalid configuration.
//! `auth.rs`, `attestation.rs`, and `x402_config.rs` each need this
//! distinction with a different error type, so it lives here once instead of
//! three hand-mirrored copies.

/// Reads an env var, distinguishing "unset" (`Ok(None)`) from "set but not
/// valid UTF-8" (`Err`, built from `key` via `not_unicode`).
pub(crate) fn checked_env_var<E>(
    key: &'static str,
    not_unicode: impl FnOnce(&'static str) -> E,
) -> Result<Option<String>, E> {
    match std::env::var(key) {
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(not_unicode(key)),
    }
}
