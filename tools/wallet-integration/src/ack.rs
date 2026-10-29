//! The deploy-time acknowledgement: the operator lists, exactly, the
//! `breaking-wallet-integration` PRs a deploy ships.

use std::collections::BTreeSet;

/// Parse `625,640` (also `#625, #640`; blank or `none` for the empty set).
/// A repeated number is an error: the list is meant to be typed deliberately.
pub fn parse_ack(list: &str) -> Result<BTreeSet<u64>, String> {
    let list = list.trim();
    if list.is_empty() || list.eq_ignore_ascii_case("none") {
        return Ok(BTreeSet::new());
    }
    let mut set = BTreeSet::new();
    for part in list.split(',') {
        let p = part.trim();
        let digits = p.strip_prefix('#').unwrap_or(p);
        let n: u64 = digits
            .parse()
            .map_err(|_| format!("not a PR number: {p:?} (expected e.g. 625,640)"))?;
        if !set.insert(n) {
            return Err(format!("PR #{n} is listed twice"));
        }
    }
    Ok(set)
}

/// Why an ack doesn't match the PRs a deploy ships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckMismatch {
    /// Shipped `breaking-wallet-integration` PRs the operator didn't list.
    pub missing: Vec<u64>,
    /// Listed PRs that aren't in this deploy.
    pub extra: Vec<u64>,
}

impl std::fmt::Display for AckMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fmt_list = |v: &[u64]| {
            v.iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("not acknowledged: {}", fmt_list(&self.missing)));
        }
        if !self.extra.is_empty() {
            parts.push(format!(
                "acknowledged but not in this deploy: {}",
                fmt_list(&self.extra)
            ));
        }
        write!(f, "{}", parts.join("; "))
    }
}

/// The ack must equal `required` exactly: nothing missing, nothing extra.
pub fn check_ack(required: &BTreeSet<u64>, acked: &BTreeSet<u64>) -> Result<(), AckMismatch> {
    let missing: Vec<u64> = required.difference(acked).copied().collect();
    let extra: Vec<u64> = acked.difference(required).copied().collect();
    if missing.is_empty() && extra.is_empty() {
        Ok(())
    } else {
        Err(AckMismatch { missing, extra })
    }
}

/// `625,640`, the form `--ack-breaking-wallet-integration` takes.
pub fn format_ack(set: &BTreeSet<u64>) -> String {
    set.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn set(v: &[u64]) -> BTreeSet<u64> {
        v.iter().copied().collect()
    }

    #[test]
    fn parse_forms() {
        assert_eq!(parse_ack("625,640").unwrap(), set(&[625, 640]));
        assert_eq!(parse_ack(" #640 , 625 ").unwrap(), set(&[625, 640]));
        assert_eq!(parse_ack("").unwrap(), set(&[]));
        assert_eq!(parse_ack("None").unwrap(), set(&[]));
    }

    #[test]
    fn parse_rejects_junk_and_duplicates() {
        assert!(parse_ack("625,").is_err());
        assert!(parse_ack("pr625").is_err());
        assert!(parse_ack("625 640").is_err());
        assert!(parse_ack("-1").is_err());
        let err = parse_ack("625,#625").unwrap_err();
        assert!(err.contains("twice"), "{err}");
    }

    #[test]
    fn exact_match_passes() {
        assert!(check_ack(&set(&[625, 640]), &set(&[640, 625])).is_ok());
        assert!(check_ack(&set(&[]), &set(&[])).is_ok());
    }

    #[test]
    fn missing_and_extra_are_both_errors() {
        let err = check_ack(&set(&[625, 640]), &set(&[625])).unwrap_err();
        assert_eq!(err.missing, vec![640]);
        assert!(err.extra.is_empty());

        let err = check_ack(&set(&[625]), &set(&[625, 700])).unwrap_err();
        assert_eq!(err.extra, vec![700]);

        let err = check_ack(&set(&[625]), &set(&[])).unwrap_err();
        assert_eq!(err.to_string(), "not acknowledged: #625");

        let err = check_ack(&set(&[]), &set(&[1, 2])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "acknowledged but not in this deploy: #1, #2"
        );
    }

    #[test]
    fn format_round_trips() {
        let s = set(&[640, 625]);
        assert_eq!(format_ack(&s), "625,640");
        assert_eq!(parse_ack(&format_ack(&s)).unwrap(), s);
    }
}
