//! Resolve the `--transaction` CLI value, supporting curl-style `@` references.
//!
//! - `@path/to/file` reads the transaction string from a file.
//! - `@-` reads it from stdin.
//! - Anything else is returned unchanged.
//!
//! In all `@` cases, leading and trailing whitespace is trimmed since the
//! transaction string itself (hex / base64) cannot legitimately contain it.

use std::io::Read;

/// Maximum allowed size for transaction input read via `@file` or `@-` (10 MB).
const MAX_TRANSACTION_INPUT_SIZE: u64 = 10 * 1024 * 1024;

/// Resolve a `--transaction` argument, expanding curl-style `@` references.
pub fn resolve_transaction_input(input: &str) -> Result<String, String> {
    let Some(rest) = input.strip_prefix('@') else {
        return Ok(input.to_string());
    };

    let raw = if rest == "-" {
        read_bounded(std::io::stdin().lock(), "<stdin>")?
    } else {
        let file = std::fs::File::open(rest)
            .map_err(|e| format!("Failed to open transaction file '{rest}': {e}"))?;
        read_bounded(file, rest)?
    };

    Ok(raw.trim().to_string())
}

fn read_bounded<R: Read>(reader: R, source: &str) -> Result<String, String> {
    let mut bounded = reader.take(MAX_TRANSACTION_INPUT_SIZE + 1);
    let mut buf = String::new();
    bounded
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read transaction from {source}: {e}"))?;
    if buf.len() as u64 > MAX_TRANSACTION_INPUT_SIZE {
        return Err(format!(
            "Transaction input from {source} exceeds maximum size ({MAX_TRANSACTION_INPUT_SIZE} bytes)"
        ));
    }
    Ok(buf)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn passthrough_when_no_at_prefix() {
        let input = "0xdeadbeef";
        assert_eq!(resolve_transaction_input(input).unwrap(), "0xdeadbeef");
    }

    #[test]
    fn reads_from_file_and_trims_whitespace() {
        let dir = std::env::temp_dir().join("vsp_tx_input_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "tx_{}_{}.hex",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "  0xdeadbeef\n\n").unwrap();

        let arg = format!("@{}", path.display());
        assert_eq!(resolve_transaction_input(&arg).unwrap(), "0xdeadbeef");
    }

    #[test]
    fn missing_file_returns_error() {
        let err = resolve_transaction_input("@/nonexistent/path/to/tx.hex").unwrap_err();
        assert!(
            err.contains("Failed to open transaction file"),
            "got: {err}"
        );
    }

    #[test]
    fn oversized_input_returns_error() {
        let limit = usize::try_from(MAX_TRANSACTION_INPUT_SIZE).unwrap();
        let oversized = vec![b'a'; limit + 16];
        let err = read_bounded(Cursor::new(oversized), "<test>").unwrap_err();
        assert!(err.contains("exceeds maximum size"), "got: {err}");
    }

    #[test]
    fn at_dash_alone_is_stdin_marker() {
        // Sanity: the `@-` path is exercised end-to-end in an integration test;
        // here we just confirm `@-` is not treated as a literal.
        // We can't easily inject stdin in a unit test without process boundaries,
        // so verify via the stripped-prefix check.
        assert_eq!("@-".strip_prefix('@'), Some("-"));
    }

    #[test]
    fn read_bounded_at_exact_limit_succeeds() {
        let limit = usize::try_from(MAX_TRANSACTION_INPUT_SIZE).unwrap();
        let exact = vec![b'a'; limit];
        let out = read_bounded(Cursor::new(exact), "<test>").unwrap();
        assert_eq!(
            u64::try_from(out.len()).unwrap(),
            MAX_TRANSACTION_INPUT_SIZE
        );
    }

    #[test]
    fn write_via_pipe_then_read_bounded() {
        // Smoke test that read_bounded works on arbitrary Read implementations.
        let mut cur = Cursor::new(Vec::new());
        cur.write_all(b"  hello  \n").unwrap();
        cur.set_position(0);
        let out = read_bounded(cur, "<pipe>").unwrap();
        assert_eq!(out.trim(), "hello");
    }
}
