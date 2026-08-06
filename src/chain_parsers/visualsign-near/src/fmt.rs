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
/// `AccountId` do not need it: an id carrying these bytes fails its own
/// validation during decode.
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
}
