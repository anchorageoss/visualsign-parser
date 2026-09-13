//! Rendering chain-supplied text safely into a signable payload field.
//!
//! An untrusted string reaching a field can smuggle a newline into its fallback
//! text, and [`crate::SignablePayload::validate_charset`] permits `\n` as the
//! wallet's documented multi-line separator -- so the string can render as extra
//! apparent confirmed fields on the signing screen.
//!
//! There are two policies here, and they differ in ways a signer can see. They
//! live together so that difference is stated in one place rather than
//! rediscovered from four implementations with the same name.

/// Marker substituted for a character that cannot be rendered.
pub const ELIDED: char = '?';

/// Render `text` as printable ASCII and spaces, substituting [`ELIDED`] for
/// every other character. Double quotes are kept.
///
/// Substituting rather than deleting keeps distinct inputs distinct: deletion
/// renders `a\nb` and `ab` identically, so two chain-supplied values a signer
/// must be able to tell apart can reach the screen as one string. It also makes
/// the loss visible instead of presenting what is left as the whole value.
///
/// Substituting rather than rejecting is deliberate too. A legitimate memo
/// carrying an accented character or an emoji renders with markers instead of
/// failing the whole parse, which keeps a non-ASCII memo from denying the signer
/// their transaction.
///
/// A literal backslash is marked on availability grounds rather than spoofing:
/// it serializes as `\\`, so a backslash before `u`/`t`/`r`/`b`/`f` or `/` puts a
/// `FORBIDDEN_JSON_ESCAPES` substring in the serialized payload and the charset
/// validator rejects the whole transaction.
///
/// Double quotes are kept because they serialize as `\"`, which the validator
/// deliberately permits so field text can carry real embedded JSON -- NEAR's
/// `ft_transfer_call` `msg` is exactly such a field, and marking its quotes
/// would obscure structure a signer needs to read literally.
#[must_use]
pub fn elide_unsupported(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == ' ' || (c.is_ascii_graphic() && c != '\\') {
                c
            } else {
                ELIDED
            }
        })
        .collect()
}

/// Render `text` as printable ASCII and spaces, dropping every other character
/// and double quotes as well.
///
/// Used where the renderer emits its own `{key:value}` bracketing and an
/// unescaped quote would be ambiguous, so a quote cannot be kept.
///
/// Dropping rather than marking is the difference from [`elide_unsupported`],
/// and it is lossy in a way a signer cannot see: `a\nb` and `ab` both render as
/// `ab`. Switching this to mark would change what a signer reads, so it is a
/// decision about the signing screen rather than a refactor.
#[must_use]
pub fn strip_unsupported(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn elide_marks_the_wallet_line_separator() {
        assert_eq!(elide_unsupported("a\nb"), "a?b");
    }

    #[test]
    fn elide_keeps_printable_ascii_spaces_and_quotes() {
        assert_eq!(elide_unsupported(r#"a "b" c"#), r#"a "b" c"#);
    }

    #[test]
    fn elide_marks_a_literal_backslash_and_non_ascii() {
        assert_eq!(elide_unsupported("a\\b"), "a?b");
        assert_eq!(elide_unsupported("aé"), "a?");
    }

    // The property the marker exists for.
    #[test]
    fn elide_keeps_distinct_values_distinct() {
        assert_ne!(elide_unsupported("a\nb"), elide_unsupported("ab"));
    }

    #[test]
    fn strip_drops_the_line_separator_and_quotes() {
        assert_eq!(strip_unsupported("a\nb"), "ab");
        assert_eq!(strip_unsupported(r#"a "b" c"#), "a b c");
    }

    // Stated as a test because it is the reason the two policies are not one.
    #[test]
    fn strip_collapses_values_that_elide_keeps_apart() {
        assert_eq!(strip_unsupported("a\nb"), strip_unsupported("ab"));
    }
}
