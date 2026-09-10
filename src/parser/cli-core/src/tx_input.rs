//! Resolve the `--transaction` CLI value, supporting curl-style `@` references.
//!
//! - `@path/to/file` reads the transaction string from a file.
//! - `@-` reads it from stdin.
//! - Anything else is returned unchanged.
//!
//! In all `@` cases, leading and trailing whitespace comes off, so a file
//! ending in a newline behaves like the same value passed inline.
//!
//! Internal ASCII whitespace (space, tab, line feed, form feed, carriage
//! return) is stripped as well for hex / base64 bodies, which cannot
//! legitimately contain it — this lets users paste line-wrapped hex from
//! block explorers or terminal emulators without manual cleanup. A JSON
//! envelope is exempt: it is itself a transaction format, and its string
//! values can legitimately contain spaces, so stripping them would decode
//! different bytes than the same input passed via `-t`.
//!
//! The 10 MB size limit is applied to the raw read so a whitespace-padded
//! file can't bypass it.

use std::io::Read;

/// Maximum allowed size for transaction input read via `@file` or `@-` (10 MB).
const MAX_TRANSACTION_INPUT_SIZE: u64 = 10 * 1024 * 1024;

/// Resolve a `--transaction` argument, expanding curl-style `@` references.
pub fn resolve_transaction_input(input: &str) -> Result<String, String> {
    let Some(rest) = input.strip_prefix('@') else {
        return Ok(input.to_string());
    };

    let raw = match rest {
        "" => {
            return Err(
                "'@' must be followed by a path, or use '@-' to read from stdin".to_string(),
            );
        }
        "-" => read_bounded(std::io::stdin().lock(), "<stdin>")?,
        path => {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("Failed to open transaction file '{path}': {e}"))?;
            read_bounded(file, path)?
        }
    };

    Ok(resolve_buffer(&raw))
}

/// Apply the whitespace rule for a buffer read via `@file` / `@-`.
///
/// A JSON envelope is itself a transaction format, and its string values can
/// legitimately contain spaces, so stripping is confined to the encodings that
/// cannot carry whitespace at all. Leading/trailing whitespace comes off
/// either way, so a file ending in a newline behaves the same for both.
///
/// A leading `{` is the whole test. Every JSON transaction format this CLI
/// accepts is an object, and no hex or base64 body can begin with that byte,
/// so the two cases cannot be confused.
///
/// One leading UTF-8 BOM is dropped before that test runs. A BOM is not
/// `White_Space`, so `trim_start` leaves it in place and BOM-prefixed JSON --
/// routine from Windows editors and spreadsheet exports -- would fail the `{`
/// test and be routed to the stripping branch, silently deleting spaces
/// *inside* its string values: exactly the corruption this rule exists to
/// prevent. It is not ASCII whitespace either, so on the other branch it
/// survives into a hex or base64 body and fails to decode. Dropping it fixes
/// both, and dropping it (rather than only re-routing) is what makes the file
/// usable at all, since `serde_json` rejects a leading BOM with "expected
/// value at line 1 column 1".
///
/// Only at offset 0: a U+FEFF anywhere else is ZWNBSP content, not a byte
/// order mark, and inside a JSON string value it is the caller's data.
fn resolve_buffer(raw: &str) -> String {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if raw.trim_start().starts_with('{') {
        raw.trim().to_string()
    } else {
        strip_ascii_whitespace(raw)
    }
}

/// Remove every ASCII-whitespace byte from `input`. We intentionally use the
/// ASCII variant (not `split_whitespace`) so exotic Unicode-whitespace
/// characters such as NBSP (`\u{00A0}`) stay in the buffer and surface as
/// decode errors rather than being silently swallowed.
fn strip_ascii_whitespace(input: &str) -> String {
    input.split_ascii_whitespace().collect()
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
    use crate::test_utils::write_temp_json;
    use std::io::{Cursor, Write};

    #[test]
    fn passthrough_when_no_at_prefix() {
        let input = "0xdeadbeef";
        assert_eq!(resolve_transaction_input(input).unwrap(), "0xdeadbeef");
    }

    #[test]
    fn reads_from_file_and_trims_whitespace() {
        let path = write_temp_json("vsp_tx_input_tests", "tx.hex", "  0xdeadbeef\n\n");
        let arg = format!("@{}", path.display());
        assert_eq!(resolve_transaction_input(&arg).unwrap(), "0xdeadbeef");
    }

    #[test]
    fn reads_from_file_and_strips_internal_whitespace() {
        // Line-wrapped hex (with stray internal space and tab) as a user
        // might paste from a block explorer or wrapped terminal output.
        let wrapped = "0a8a010a020793\n2208e4e3a4d46f74\td763\r\n   ";
        let path = write_temp_json("vsp_tx_input_tests", "tx_wrapped.hex", wrapped);
        let arg = format!("@{}", path.display());
        assert_eq!(
            resolve_transaction_input(&arg).unwrap(),
            "0a8a010a0207932208e4e3a4d46f74d763",
        );
    }

    #[test]
    fn strip_ascii_whitespace_preserves_non_ascii() {
        // Non-ASCII whitespace (e.g. NBSP) must pass through unchanged so we
        // never silently corrupt input that happens to land in `@file` mode.
        // Using NBSP (which `split_whitespace` *would* strip) guards against
        // accidentally switching to Unicode-aware whitespace splitting.
        assert_eq!(
            strip_ascii_whitespace("a b\tc\nd \u{00A0}e"),
            "abcd\u{00A0}e"
        );
    }

    // A JSON envelope is a transaction format whose string values can
    // legitimately contain spaces, so the hex-oriented stripping above must
    // not reach it: `-t` and `@file` have to decode the same bytes.
    #[test]
    fn reads_json_from_file_without_touching_its_whitespace() {
        let json = r#"{"memo":"AAA BBB  CCC"}"#;
        let path = write_temp_json("vsp_tx_input_tests", "tx.json", json);
        let arg = format!("@{}", path.display());
        assert_eq!(resolve_transaction_input(&arg).unwrap(), json);
    }

    /// A BOM-prefixed JSON file is routine from Windows editors and
    /// spreadsheet exports. Before the BOM was dropped, `trim_start` left it
    /// in place (it is not `White_Space`), the `{` test failed, and the buffer
    /// went to the stripping branch -- deleting the spaces *inside* the memo
    /// while still producing valid JSON. Silent corruption of signed content,
    /// which is the failure this whole rule exists to prevent.
    #[test]
    fn bom_prefixed_json_keeps_its_internal_whitespace() {
        let json = r#"{"memo":"AAA BBB  CCC"}"#;
        let with_bom = format!("\u{feff}{json}");
        assert_eq!(resolve_buffer(&with_bom), json);
    }

    /// End to end through the file path, since the BOM arrives from a file in
    /// every real report of this.
    #[test]
    fn reads_bom_prefixed_json_from_file_without_touching_its_whitespace() {
        let json = r#"{"memo":"AAA BBB  CCC"}"#;
        let path = write_temp_json(
            "vsp_tx_input_tests",
            "tx_bom.json",
            &format!("\u{feff}{json}"),
        );
        let arg = format!("@{}", path.display());
        assert_eq!(resolve_transaction_input(&arg).unwrap(), json);
    }

    /// The BOM is dropped rather than merely re-routed: `serde_json` rejects a
    /// leading BOM ("expected value at line 1 column 1"), so keeping it would
    /// trade silent corruption for a confusing parse error instead of making
    /// the file work.
    #[test]
    fn bom_is_removed_not_just_routed() {
        assert!(!resolve_buffer("\u{feff}{\"a\":1}").starts_with('\u{feff}'));
    }

    /// A BOM on a hex body is not ASCII whitespace, so it used to survive into
    /// the decoder and fail there. Same one-BOM rule covers it.
    #[test]
    fn bom_prefixed_hex_still_strips_to_a_clean_body() {
        assert_eq!(
            resolve_buffer("\u{feff}0a8a01\n0a0207 93\n"),
            "0a8a010a020793"
        );
    }

    /// Only offset 0 is a byte order mark. A U+FEFF inside a JSON string value
    /// is the caller's data and must survive untouched, so the fix cannot be
    /// implemented as a global replace.
    #[test]
    fn a_bom_inside_a_json_value_is_preserved() {
        let json = "{\"memo\":\"A\u{feff}B\"}";
        assert_eq!(resolve_buffer(json), json);
    }

    /// Exactly one BOM comes off, so a second is still content and still
    /// routes the buffer honestly rather than being quietly absorbed.
    #[test]
    fn only_one_leading_bom_is_dropped() {
        let doubled = "\u{feff}\u{feff}{\"a\":1}";
        assert_eq!(resolve_buffer(doubled), "\u{feff}{\"a\":1}");
    }

    #[test]
    fn json_detection_tolerates_surrounding_whitespace() {
        let path = write_temp_json(
            "vsp_tx_input_tests",
            "tx_padded.json",
            "\n  {\"memo\":\"A B\"}\n",
        );
        let arg = format!("@{}", path.display());
        assert_eq!(
            resolve_transaction_input(&arg).unwrap(),
            "{\"memo\":\"A B\"}"
        );
    }

    #[test]
    fn json_from_stdin_keeps_its_whitespace() {
        let json = r#"{"memo":"AAA BBB  CCC"}"#;
        assert_eq!(resolve_buffer(json), json);
    }

    #[test]
    fn a_hex_body_is_still_stripped() {
        // The JSON carve-out must not weaken the line-wrapped-hex handling.
        assert_eq!(resolve_buffer("0a8a01\n0a0207 93\n"), "0a8a010a020793");
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
    fn empty_at_returns_clear_error() {
        let err = resolve_transaction_input("@").unwrap_err();
        assert!(err.contains("must be followed by a path"), "got: {err}");
    }

    #[test]
    fn oversized_input_returns_error() {
        let limit = usize::try_from(MAX_TRANSACTION_INPUT_SIZE).unwrap();
        let oversized = vec![b'a'; limit + 16];
        let err = read_bounded(Cursor::new(oversized), "<test>").unwrap_err();
        assert!(err.contains("exceeds maximum size"), "got: {err}");
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
        let mut cur = Cursor::new(Vec::new());
        cur.write_all(b"  hello  \n").unwrap();
        cur.set_position(0);
        let out = read_bounded(cur, "<pipe>").unwrap();
        assert_eq!(out.trim(), "hello");
    }
}
