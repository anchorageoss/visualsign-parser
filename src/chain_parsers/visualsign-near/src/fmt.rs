//! yoctoNEAR, Tgas, and field-text formatting helpers.

/// Strips everything except printable ASCII and spaces, so a chain-supplied
/// string (`memo`, `msg`, `method_name`, an NFT/MT token id) cannot smuggle a
/// newline into a text field's fallback text. The core crate's charset
/// validator permits `\n` as the wallet's documented multi-line separator
/// (`SignablePayload::validate_charset`), so an unfiltered attacker-controlled
/// string can render as extra apparent confirmed fields on the signing screen.
///
/// Every untrusted string reaching a field on either NEAR path -- the borsh
/// transaction path and the intents path -- goes through here. Values typed as
/// `AccountId` are the one exemption: an id carrying these bytes fails its own
/// validation during decode, so filtering it would be dead code.
///
/// `TokenId` is not such a value, despite its account-id-shaped prefix. Its
/// `FromStr` parses only the contract half as an `AccountId` and takes the
/// remainder verbatim into a plain `String`, so an asset id needs filtering
/// like any other caller-supplied text.
///
/// Filtering rather than rejecting is deliberate. A legitimate memo carrying
/// an accented character or an emoji loses those characters instead of failing
/// the whole parse, which keeps a non-ASCII memo from denying the signer their
/// transaction.
///
/// A literal backslash is stripped too, on availability grounds rather than
/// spoofing: it serializes as `\\`, so a backslash before `u`/`t`/`r`/`b`/`f`
/// or `/` puts a `FORBIDDEN_JSON_ESCAPES` substring in the serialized payload
/// and `SignablePayload::validate_charset` rejects the whole transaction.
///
/// Double quotes are kept. They serialize as `\"`, which the core validator
/// deliberately permits so field text can carry real embedded JSON -- and
/// `ft_transfer_call`'s `msg` is exactly such a field, so deleting its quotes
/// would strip structure a signer needs to read literally.
pub(crate) fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '\\'))
        .collect()
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
    fn charset_safe_strips_the_wallet_line_separator() {
        assert_eq!(
            charset_safe("innocent\nTo: alice.near"),
            "innocentTo: alice.near"
        );
    }

    /// `\t`, `\r`, `\b`, `\f` serialize to `FORBIDDEN_JSON_ESCAPES` substrings,
    /// which make `SignablePayload::validate_charset` refuse the whole
    /// transaction. Stripping them here keeps one attacker-supplied byte from
    /// withholding the payload entirely.
    #[test]
    fn charset_safe_strips_the_other_control_escapes() {
        assert_eq!(charset_safe("a\tb\rc\u{8}d\u{c}e"), "abcde");
    }

    #[test]
    fn charset_safe_strips_a_literal_backslash() {
        assert_eq!(charset_safe(r"a\u0041b"), "au0041b");
    }

    #[test]
    fn charset_safe_strips_non_ascii() {
        // A bidi override can reorder a rendered line without changing its
        // bytes; an emoji and an accent are simply outside the ASCII range the
        // core validator accepts.
        assert_eq!(
            charset_safe("caf\u{e9} \u{202e}dlrow \u{1f600}"),
            "caf dlrow "
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

    #[test]
    fn charset_safe_empties_an_all_non_ascii_string() {
        assert_eq!(charset_safe("\u{e9}\u{e9}\u{e9}"), "");
    }
}
