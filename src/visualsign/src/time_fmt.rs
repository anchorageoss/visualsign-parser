//! Shared timestamp formatting helpers for chain parsers.
//!
//! Absolute formatting via chrono; relative ("N minutes ago") is hand-rolled
//! integer math so the wording stays consistent across chains and the helper
//! has zero extra dependencies.

use chrono::DateTime;

/// Format epoch-milliseconds as `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Returns `"invalid timestamp"` for values outside chrono's representable
/// range — callers should still print the raw epoch alongside so signers can
/// see the original bytes even when the date is unrepresentable.
pub fn format_timestamp_ms(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "invalid timestamp".to_string())
}

/// Format epoch-ms relative to `now_ms`, e.g. `"about 2 hours ago"` or
/// `"in about 23 hours"`. Coarse one-unit precision, integer math, no f64.
///
/// Returns `None` when `ms` is outside chrono's representable range — callers
/// should omit the relative tag rather than emit a misleading one.
pub fn format_relative_ms(ms: i64, now_ms: i64) -> Option<String> {
    // Validate the timestamp via chrono so we behave the same way as
    // format_timestamp_ms.
    DateTime::from_timestamp_millis(ms)?;

    let diff_ms = (ms as i128) - (now_ms as i128);
    let abs_ms = diff_ms.unsigned_abs();
    let future = diff_ms > 0;

    const SEC: u128 = 1_000;
    const MIN: u128 = 60 * SEC;
    const HOUR: u128 = 60 * MIN;
    const DAY: u128 = 24 * HOUR;
    const MONTH: u128 = 30 * DAY;

    if abs_ms < SEC {
        return Some("just now".to_string());
    }

    let (n, unit, approx) = if abs_ms < MIN {
        (abs_ms / SEC, "second", false)
    } else if abs_ms < HOUR {
        (abs_ms / MIN, "minute", false)
    } else if abs_ms < DAY {
        (abs_ms / HOUR, "hour", true)
    } else if abs_ms < MONTH {
        (abs_ms / DAY, "day", true)
    } else {
        (abs_ms / MONTH, "month", true)
    };

    let plural = if n == 1 { "" } else { "s" };
    let approx_word = if approx { "about " } else { "" };
    let rendered = if future {
        format!("in {approx_word}{n} {unit}{plural}")
    } else {
        format!("{approx_word}{n} {unit}{plural} ago")
    };
    Some(rendered)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000; // 2023-11-14 22:13:20 UTC

    #[test]
    fn format_timestamp_ms_renders_known_epoch() {
        assert_eq!(format_timestamp_ms(NOW), "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn format_timestamp_ms_returns_marker_for_unrepresentable() {
        assert_eq!(format_timestamp_ms(i64::MAX), "invalid timestamp");
    }

    #[test]
    fn relative_ms_just_now_both_directions() {
        assert_eq!(format_relative_ms(NOW, NOW).as_deref(), Some("just now"));
        assert_eq!(
            format_relative_ms(NOW + 500, NOW).as_deref(),
            Some("just now"),
        );
        assert_eq!(
            format_relative_ms(NOW - 500, NOW).as_deref(),
            Some("just now"),
        );
    }

    #[test]
    fn relative_ms_seconds() {
        assert_eq!(
            format_relative_ms(NOW + 1_000, NOW).as_deref(),
            Some("in 1 second"),
        );
        assert_eq!(
            format_relative_ms(NOW - 45_000, NOW).as_deref(),
            Some("45 seconds ago"),
        );
    }

    #[test]
    fn relative_ms_minutes() {
        assert_eq!(
            format_relative_ms(NOW + 60_000, NOW).as_deref(),
            Some("in 1 minute"),
        );
        assert_eq!(
            format_relative_ms(NOW - 120_000, NOW).as_deref(),
            Some("2 minutes ago"),
        );
    }

    #[test]
    fn relative_ms_hours() {
        assert_eq!(
            format_relative_ms(NOW + 3_600_000, NOW).as_deref(),
            Some("in about 1 hour"),
        );
        assert_eq!(
            format_relative_ms(NOW - 23 * 3_600_000, NOW).as_deref(),
            Some("about 23 hours ago"),
        );
    }

    #[test]
    fn relative_ms_days() {
        let day = 86_400_000_i64;
        assert_eq!(
            format_relative_ms(NOW + day, NOW).as_deref(),
            Some("in about 1 day"),
        );
        assert_eq!(
            format_relative_ms(NOW - 5 * day, NOW).as_deref(),
            Some("about 5 days ago"),
        );
    }

    #[test]
    fn relative_ms_months() {
        let month = 30 * 86_400_000_i64;
        assert_eq!(
            format_relative_ms(NOW + 2 * month, NOW).as_deref(),
            Some("in about 2 months"),
        );
        assert_eq!(
            format_relative_ms(NOW - 6 * month, NOW).as_deref(),
            Some("about 6 months ago"),
        );
    }

    #[test]
    fn relative_ms_returns_none_for_unrepresentable() {
        assert!(format_relative_ms(i64::MAX, NOW).is_none());
        assert!(format_relative_ms(i64::MIN, NOW).is_none());
    }
}
