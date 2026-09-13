//! yoctoNEAR, Tgas, and field-text formatting helpers.

/// Render chain-supplied text safely into a field, marking what cannot be
/// rendered rather than dropping it.
///
/// Every untrusted string reaching a field on either NEAR path -- the borsh
/// transaction path and the intents path -- goes through here. Values typed as
/// `AccountId` are the one exemption: an id carrying these bytes fails its own
/// validation during decode, so filtering it would be dead code.
///
/// `TokenId` is not such a value, despite its account-id-shaped prefix. Its
/// `FromStr` parses only the contract half as an `AccountId` and takes the
/// remainder verbatim into a plain `String`, so an asset id needs filtering like
/// any other caller-supplied text.
///
/// See [`visualsign::charset::elide_unsupported`] for why marking beats both
/// dropping and rejecting, and for why double quotes are kept -- which is where
/// NEAR's allowed set differs from the Solana argument renderer's.
pub(crate) fn charset_safe(text: &str) -> String {
    visualsign::charset::elide_unsupported(text)
}

/// Render `units / 10^decimals` as a decimal string with trailing-zero trim.
///
/// No rounding: the value is exact. A zero fractional part yields just the
/// integer portion (e.g. `1`); otherwise the fraction is zero-padded to
/// `decimals` digits and stripped of trailing zeros (e.g. `1.5`).
fn format_fixed(units: u128, decimals: u32) -> String {
    let scale = 10u128.pow(decimals);
    let whole = units / scale;
    let frac = units % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{frac:0width$}", width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{whole}.{trimmed}")
}

/// Format yoctoNEAR (10^-24 NEAR) as a decimal NEAR string with trailing-zero trim.
#[must_use]
pub fn format_near(yocto: u128) -> String {
    format_fixed(yocto, 24)
}

/// Format gas as Tgas (10^12 gas units) with trailing-zero trim.
#[must_use]
pub fn format_tgas(gas: u64) -> String {
    format_fixed(u128::from(gas), 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_near_zero() {
        assert_eq!(format_near(0), "0");
    }

    #[test]
    fn format_near_whole() {
        assert_eq!(format_near(1_000_000_000_000_000_000_000_000), "1");
    }

    #[test]
    fn format_near_fractional() {
        assert_eq!(format_near(1_500_000_000_000_000_000_000_000), "1.5");
    }

    #[test]
    fn format_near_tiny() {
        assert_eq!(format_near(1), "0.000000000000000000000001");
    }

    #[test]
    fn format_tgas_one() {
        assert_eq!(format_tgas(1_000_000_000_000), "1");
    }

    #[test]
    fn format_tgas_hundred() {
        assert_eq!(format_tgas(100_000_000_000_000), "100");
    }

    #[test]
    fn charset_safe_marks_the_wallet_line_separator() {
        assert_eq!(
            charset_safe("innocent\nTo: alice.near"),
            "innocent?To: alice.near"
        );
    }

    /// `\t`, `\r`, `\b`, `\f` serialize to `FORBIDDEN_JSON_ESCAPES` substrings,
    /// which make `SignablePayload::validate_charset` refuse the whole
    /// transaction. Marking them here keeps one attacker-supplied byte from
    /// withholding the payload entirely.
    #[test]
    fn charset_safe_marks_the_other_control_escapes() {
        assert_eq!(charset_safe("a\tb\rc\u{8}d\u{c}e"), "a?b?c?d?e");
    }

    #[test]
    fn charset_safe_marks_a_literal_backslash() {
        assert_eq!(charset_safe(r"a\u0041b"), "a?u0041b");
    }

    #[test]
    fn charset_safe_marks_non_ascii() {
        // A bidi override can reorder a rendered line without changing its
        // bytes; an emoji and an accent are simply outside the ASCII range the
        // core validator accepts.
        assert_eq!(
            charset_safe("caf\u{e9} \u{202e}dlrow \u{1f600}"),
            "caf? ?dlrow ?"
        );
    }

    #[test]
    fn charset_safe_keeps_printable_ascii_and_spaces() {
        assert_eq!(
            charset_safe("Send 1.5 wNEAR to alice.near (id #7)"),
            "Send 1.5 wNEAR to alice.near (id #7)"
        );
    }

    /// Double quotes survive: they serialize as `\"`, which the core validator
    /// permits so a field can carry real embedded JSON.
    #[test]
    fn charset_safe_keeps_double_quotes() {
        assert_eq!(charset_safe(r#"{"amount":"1"}"#), r#"{"amount":"1"}"#);
    }

    /// An all-non-ASCII string renders as markers rather than as nothing. The
    /// signer learns the sender attached text they cannot read, which an empty
    /// result would hide.
    #[test]
    fn charset_safe_marks_an_all_non_ascii_string() {
        assert_eq!(charset_safe("\u{e9}\u{e9}\u{e9}"), "???");
    }

    /// Deleting unrenderable characters let distinct values collapse onto one
    /// rendering. A signer comparing two fields has to be able to tell them
    /// apart.
    #[test]
    fn charset_safe_keeps_distinct_values_distinct() {
        assert_ne!(
            charset_safe("innocent\nTo: alice.near"),
            charset_safe("innocentTo: alice.near"),
            "a value carrying a line separator must not render as one without it"
        );
    }
}
