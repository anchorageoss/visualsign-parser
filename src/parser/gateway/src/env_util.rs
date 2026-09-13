//! Shared env-var and bounded-file-read helpers.
//!
//! `std::env::var(..).ok()` collapses "unset" and "set but not valid UTF-8"
//! into the same `None`, which would silently fall through to the unset-var
//! default for a malformed value instead of reporting invalid configuration.
//! `auth.rs`, `attestation.rs`, and `x402_config.rs` each need this
//! distinction with a different error type, so it lives here once instead of
//! three hand-mirrored copies.
//!
//! `auth.rs` and `attestation.rs` also each read a small config file (bearer
//! token / pinned pubkey) with a bounded reader so a mistaken path to a very
//! large file or character device can't exhaust memory or hang startup. That
//! idiom lives here too, parameterized by the caller's error constructors.

use std::io::Read;

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

/// Reads `path` with a bounded reader: at most `max_size + 1` bytes, so a
/// file at exactly `max_size` can be distinguished from one that overflows
/// it without ever reading an unbounded amount from disk. Errors from
/// opening/reading are built via `read_err`; exceeding `max_size` is built
/// via `too_large_err`.
pub(crate) fn read_bounded_file<E>(
    path: &str,
    max_size: u64,
    read_err: impl Fn(String, String) -> E,
    too_large_err: impl FnOnce(String, u64) -> E,
) -> Result<String, E> {
    let file = std::fs::File::open(path).map_err(|e| read_err(path.to_string(), e.to_string()))?;
    let mut bounded = file.take(max_size + 1);
    let mut contents = String::new();
    bounded
        .read_to_string(&mut contents)
        .map_err(|e| read_err(path.to_string(), e.to_string()))?;
    if contents.len() as u64 > max_size {
        return Err(too_large_err(path.to_string(), max_size));
    }
    Ok(contents)
}
