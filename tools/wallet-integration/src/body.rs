//! Parsing a PR description's `## Wallet integration` section.
//!
//! Format (one item per line; other prose in the section is ignored):
//!
//! ```text
//! ## Wallet integration
//!
//! - requires: <integrator> >= <minimum version>
//! - requires: <another integrator> >= <minimum version>
//! - deploy-order: <who ships first, and when the parser may deploy>
//! ```
//!
//! At least one `requires` and one `deploy-order` line are needed. The section
//! runs until the next heading of the same or a higher level. Lines inside HTML
//! comments and fenced code blocks are skipped, so a commented-out template
//! never satisfies the check.

pub const SECTION_TITLE: &str = "Wallet integration";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub integrator: String,
    pub min_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub requires: Vec<Requirement>,
    pub deploy_order: Vec<String>,
}

impl std::fmt::Display for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for Requirement {
            integrator,
            min_version,
        } in &self.requires
        {
            writeln!(f, "  requires: {integrator} >= {min_version}")?;
        }
        for order in &self.deploy_order {
            writeln!(f, "  deploy-order: {order}")?;
        }
        Ok(())
    }
}

/// A short reminder of the format, appended to every parse error.
pub const FORMAT_HELP: &str = "expected, in the PR description:\n\
\n\
## Wallet integration\n\
\n\
- requires: <integrator> >= <minimum version>\n\
- deploy-order: <who ships first, and when the parser may deploy>";

/// Parse the section, or every problem found with it.
pub fn parse_section(body: &str) -> Result<Section, Vec<String>> {
    let lines = visible_lines(body);
    let Some((start, level)) = lines.iter().enumerate().find_map(|(i, l)| {
        heading(l)
            .filter(|(_, t)| is_title(t))
            .map(|(lv, _)| (i, lv))
    }) else {
        return Err(vec![format!(
            "no `## {SECTION_TITLE}` section in the PR description"
        )]);
    };

    let mut section = Section::default();
    let mut problems = Vec::new();
    for line in &lines[start + 1..] {
        if heading(line).is_some_and(|(lv, _)| lv <= level) {
            break;
        }
        let Some(item) = list_item(line) else {
            continue;
        };
        if let Some(rest) = strip_key(item, "requires:") {
            match parse_requirement(rest) {
                Ok(r) => section.requires.push(r),
                Err(e) => problems.push(e),
            }
        } else if let Some(rest) = strip_key(item, "deploy-order:") {
            let rest = rest.trim();
            if rest.is_empty() || has_placeholder(rest) {
                problems.push(format!("`deploy-order:` needs real text, got {rest:?}"));
            } else {
                section.deploy_order.push(rest.to_owned());
            }
        }
    }
    if section.requires.is_empty() {
        problems.push("no `- requires: <integrator> >= <version>` line".to_owned());
    }
    if section.deploy_order.is_empty() {
        problems.push("no `- deploy-order: ...` line".to_owned());
    }
    if problems.is_empty() {
        Ok(section)
    } else {
        Err(problems)
    }
}

fn parse_requirement(rest: &str) -> Result<Requirement, String> {
    let Some((integrator, version)) = rest.rsplit_once(">=") else {
        return Err(format!(
            "`requires:` line must read `<integrator> >= <version>`, got {:?}",
            rest.trim()
        ));
    };
    let (integrator, min_version) = (integrator.trim(), version.trim());
    if integrator.is_empty() || min_version.is_empty() {
        return Err(format!(
            "`requires:` line needs both an integrator and a version, got {:?}",
            rest.trim()
        ));
    }
    if has_placeholder(integrator) || has_placeholder(min_version) {
        return Err(format!(
            "`requires:` line still has a <placeholder>: {:?}",
            rest.trim()
        ));
    }
    Ok(Requirement {
        integrator: integrator.to_owned(),
        min_version: min_version.to_owned(),
    })
}

/// Lines outside HTML comments and fenced code, trimmed, `\r` stripped.
fn visible_lines(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut in_comment, mut in_fence) = (false, false);
    for raw in body.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if !in_comment && (line.starts_with("```") || line.starts_with("~~~")) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Drop comment spans, which may open and close on the same line.
        let mut kept = String::new();
        let mut rest = line;
        loop {
            if in_comment {
                match rest.find("-->") {
                    Some(i) => {
                        rest = &rest[i + 3..];
                        in_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("<!--") {
                    Some(i) => {
                        kept.push_str(&rest[..i]);
                        rest = &rest[i + 4..];
                        in_comment = true;
                    }
                    None => {
                        kept.push_str(rest);
                        break;
                    }
                }
            }
        }
        out.push(kept.trim().to_owned());
    }
    out
}

/// `## Title` -> (2, "Title").
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    (rest.is_empty() || rest.starts_with(' ')).then(|| (level, rest.trim()))
}

fn is_title(text: &str) -> bool {
    text.trim_end_matches(':')
        .eq_ignore_ascii_case(SECTION_TITLE)
}

fn list_item(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(str::trim)
}

/// Case-insensitive `key` prefix, e.g. `Requires:`.
fn strip_key<'a>(item: &'a str, key: &str) -> Option<&'a str> {
    let head = item.get(..key.len())?;
    head.eq_ignore_ascii_case(key).then(|| &item[key.len()..])
}

/// `<something>` left over from the template.
fn has_placeholder(s: &str) -> bool {
    s.find('<')
        .is_some_and(|i| s[i + 1..].find('>').is_some_and(|j| j > 0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn req(integrator: &str, min_version: &str) -> Requirement {
        Requirement {
            integrator: integrator.to_owned(),
            min_version: min_version.to_owned(),
        }
    }

    #[test]
    fn parses_a_full_section() {
        let body = "## Summary\r\n\r\nstuff\r\n\r\n## Wallet integration\r\n\r\n\
Readers must ship first.\r\n\
- requires: Anchorage HSM v5 reader (PROF-1231) >= 2026.10.30\r\n\
* Requires: visualsign-turnkeyclient >= v0.217.0\r\n\
- deploy-order: readers first, then the parser\r\n\
### Notes\r\nnested heading stays in the section\r\n\
- requires: nested >= 1\r\n\
## Testing\r\n- requires: not-in-section >= 9\r\n";
        let s = parse_section(body).unwrap();
        assert_eq!(
            s.requires,
            vec![
                req("Anchorage HSM v5 reader (PROF-1231)", "2026.10.30"),
                req("visualsign-turnkeyclient", "v0.217.0"),
                req("nested", "1"),
            ]
        );
        assert_eq!(s.deploy_order, vec!["readers first, then the parser"]);
        assert!(s.to_string().contains("requires: nested >= 1"));
    }

    #[test]
    fn missing_section_is_an_error() {
        let err = parse_section("## Summary\n- requires: a >= 1\n").unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(err[0].contains("no `## Wallet integration`"), "{err:?}");
    }

    #[test]
    fn empty_and_malformed_sections_list_every_problem() {
        let err = parse_section("## Wallet integration\n").unwrap_err();
        assert_eq!(err.len(), 2, "{err:?}");

        let err = parse_section(
            "## Wallet integration\n- requires: just-a-name\n- requires: x >=\n- deploy-order:\n",
        )
        .unwrap_err();
        // two bad requires, an empty deploy-order, and then nothing valid of either
        assert_eq!(err.len(), 5, "{err:?}");
    }

    #[test]
    fn template_placeholders_are_rejected() {
        let err = parse_section(
            "## Wallet integration\n- requires: <integrator> >= <version>\n\
             - deploy-order: <who ships first>\n",
        )
        .unwrap_err();
        assert!(err.iter().any(|e| e.contains("placeholder")), "{err:?}");
        assert!(err.iter().any(|e| e.contains("deploy-order")), "{err:?}");
    }

    #[test]
    fn commented_and_fenced_lines_are_ignored() {
        let body = "## Wallet integration\n\
<!--\n- requires: hidden >= 1\n- deploy-order: hidden\n-->\n\
```\n- requires: fenced >= 1\n```\n\
- requires: real >= 2 <!-- inline note -->\n\
<!-- - deploy-order: hidden --> - deploy-order: visible\n";
        let s = parse_section(body).unwrap();
        assert_eq!(s.requires, vec![req("real", "2")]);
        assert_eq!(s.deploy_order, vec!["visible"]);

        // A commented-out template alone does not satisfy the check.
        let body = "<!--\n## Wallet integration\n- requires: a >= 1\n- deploy-order: x\n-->\n";
        assert!(parse_section(body).is_err());
    }

    #[test]
    fn heading_variants() {
        assert!(
            parse_section("# wallet integration:\n- requires: a >= 1\n- deploy-order: x").is_ok()
        );
        assert!(
            parse_section("##Wallet integration\n- requires: a >= 1\n- deploy-order: x").is_err()
        );
    }
}
