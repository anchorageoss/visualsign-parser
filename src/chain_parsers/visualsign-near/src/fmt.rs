//! yoctoNEAR, Tgas, base58 formatting helpers.

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
