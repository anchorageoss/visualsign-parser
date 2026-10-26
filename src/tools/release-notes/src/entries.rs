//! Turn one merged PR into an announcement entry: which section it belongs in, the line
//! to show, and who to credit.

use std::collections::BTreeSet;

use serde::Serialize;

/// Where an entry is listed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "chain", rename_all = "snake_case")]
pub enum Section {
    Chain(String),
    /// User-visible change not tied to one chain (core types, server, proto).
    Core,
    /// CI, build, tests, docs, CLA. Shown only to an internal audience.
    Tooling,
}

/// A PR touching more chains than this is a cross-cutting change, listed under Core.
const MAX_CHAINS_PER_ENTRY: usize = 3;

/// Conventional-commit types that never change what a signer sees, whatever they touch.
const TOOLING_TYPES: &[&str] = &["ci", "build", "test", "docs", "style"];
/// Types that are tooling unless they touch a chain crate (`chore(near)` can change
/// decoding).
const CHORE_TYPES: &[&str] = &["chore"];
/// Scopes that mark a PR as tooling whatever its type.
const TOOLING_SCOPES: &[&str] = &["ci", "cla", "deps", "make", "release"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conventional<'a> {
    pub kind: &'a str,
    pub scope: Option<&'a str>,
    pub breaking: bool,
    pub subject: &'a str,
}

/// `type(scope)!: subject`, or None when the title isn't conventional.
pub fn parse_conventional(title: &str) -> Option<Conventional<'_>> {
    let (head, subject) = title.split_once(": ")?;
    let (head, breaking) = match head.strip_suffix('!') {
        Some(h) => (h, true),
        None => (head, false),
    };
    let (kind, scope) = match head.split_once('(') {
        Some((k, rest)) => (k, Some(rest.strip_suffix(')')?)),
        None => (head, None),
    };
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
    };
    if !valid(kind) || scope.is_some_and(|s| !valid(s)) {
        return None;
    }
    Some(Conventional {
        kind,
        scope,
        breaking,
        subject: subject.trim(),
    })
}

/// The PR number from a squash-merge subject: `title (#123)`.
pub fn pr_number_from_subject(subject: &str) -> Option<u64> {
    let start = subject.rfind("(#")?;
    subject[start + 2..].strip_suffix(')')?.parse().ok()
}

/// Chains whose crate a file list touches.
pub fn chains_touched(files: &[String]) -> BTreeSet<String> {
    files
        .iter()
        .filter_map(|f| {
            if let Some(rest) = f.strip_prefix("src/chain_parsers/visualsign-") {
                rest.split('/').next().map(str::to_string)
            } else if f.starts_with("src/solana_test_utils/") {
                Some("solana".to_string())
            } else {
                None
            }
        })
        .collect()
}

pub fn sections_for(
    title: &str,
    files: &[String],
    known_chains: &BTreeSet<String>,
) -> Vec<Section> {
    let conventional = parse_conventional(title);
    let explicit_tooling = conventional.as_ref().is_some_and(|c| {
        TOOLING_TYPES.contains(&c.kind) || c.scope.is_some_and(|s| TOOLING_SCOPES.contains(&s))
    });
    if explicit_tooling {
        return vec![Section::Tooling];
    }
    let touched: BTreeSet<String> = chains_touched(files)
        .into_iter()
        .filter(|c| known_chains.contains(c))
        .collect();
    if !touched.is_empty() && touched.len() <= MAX_CHAINS_PER_ENTRY {
        return touched.into_iter().map(Section::Chain).collect();
    }
    // Past here `touched` is empty or too wide for one entry; a wide sweep is Core even
    // when its title names a chain.
    if let Some(c) = &conventional
        && touched.is_empty()
    {
        if let Some(scope) = c.scope.filter(|s| known_chains.contains(*s)) {
            return vec![Section::Chain(scope.to_string())];
        }
        if CHORE_TYPES.contains(&c.kind) {
            return vec![Section::Tooling];
        }
    }
    vec![Section::Core]
}

/// The line shown for a PR: its `## Release note` section if it has one, otherwise the
/// title without the conventional prefix.
pub fn display_line(title: &str, body: &str) -> String {
    if let Some(note) = section_text(body, &["release note", "release notes"])
        && let Some(first) = note.split("\n\n").next()
    {
        return single_line(first);
    }
    let subject = parse_conventional(title).map_or(title, |c| c.subject);
    capitalize(subject.trim())
}

/// Text under a markdown heading named one of `names` (case-insensitive), up to the next
/// heading of any level. Empty sections count as absent.
pub fn section_text(body: &str, names: &[&str]) -> Option<String> {
    let mut lines = body.lines();
    for line in lines.by_ref() {
        let Some(heading) = heading_text(line) else {
            continue;
        };
        let heading = heading.trim_end_matches(':').to_ascii_lowercase();
        if names.contains(&heading.as_str()) {
            break;
        }
    }
    let text: Vec<&str> = lines.take_while(|l| heading_text(l).is_none()).collect();
    let text = text.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn heading_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    trimmed[hashes..].strip_prefix(' ').map(str::trim)
}

/// A breaking change for integrators: `!` in the title, a `breaking` label, or a
/// `BREAKING CHANGE` trailer.
pub fn is_breaking(title: &str, labels: &[String], body: &str) -> bool {
    parse_conventional(title).is_some_and(|c| c.breaking)
        || labels.iter().any(|l| l.eq_ignore_ascii_case("breaking"))
        || body.contains("BREAKING CHANGE")
}

/// The integrator note to show for a PR, if it has one.
pub fn integrator_note(body: &str) -> Option<String> {
    section_text(
        body,
        &[
            "integrator note",
            "integrator notes",
            "integration note",
            "breaking change",
            "breaking changes",
        ],
    )
    .map(|t| single_line(&t))
}

/// Accounts that are not people. `gh` reports GitHub Apps as `app/<name>`.
pub fn is_bot(login_or_name: &str, email: &str) -> bool {
    let l = login_or_name.to_ascii_lowercase();
    let e = email.to_ascii_lowercase();
    l.ends_with("[bot]")
        || l.starts_with("app/")
        || e.contains("[bot]")
        || l == "copilot"
        || l.starts_with("claude")
        || l.starts_with("github-actions")
        || l.starts_with("dependabot")
        || e == "noreply@anthropic.com"
        || e == "noreply@github.com"
}

/// A `Co-authored-by:` trailer. The email is only used to resolve a GitHub login and is
/// never rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoAuthor {
    pub name: String,
    pub email: String,
}

impl CoAuthor {
    /// The login when the email is a GitHub noreply address.
    pub fn noreply_login(&self) -> Option<String> {
        github_login(&self.email)
    }
}

/// Human co-authors from `Co-authored-by:` trailers, bots removed, duplicates by email
/// removed.
pub fn co_author_trailers(message: &str) -> Vec<CoAuthor> {
    let mut out: Vec<CoAuthor> = Vec::new();
    for line in message.lines() {
        let Some(rest) = line.trim().strip_prefix("Co-authored-by:") else {
            continue;
        };
        let rest = rest.trim();
        let (name, email) = match rest.split_once('<') {
            Some((n, e)) => (n.trim(), e.trim_end_matches('>').trim()),
            None => (rest, ""),
        };
        let login_is_bot = github_login(email).is_some_and(|l| is_bot(&l, ""));
        if is_bot(name, email) || login_is_bot || name.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|c| c.email.eq_ignore_ascii_case(email) && !email.is_empty())
        {
            out.push(CoAuthor {
                name: name.to_string(),
                email: email.to_string(),
            });
        }
    }
    out
}

fn github_login(email: &str) -> Option<String> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    let login = local.split_once('+').map_or(local, |(_, l)| l);
    (!login.is_empty()).then(|| login.to_string())
}

fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn chains() -> BTreeSet<String> {
        ["bitcoin", "ethereum", "near", "solana", "sui", "tron"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn conventional_titles() {
        let c = parse_conventional("fix(sui)!: show other funds").unwrap();
        assert_eq!(
            (c.kind, c.scope, c.breaking, c.subject),
            ("fix", Some("sui"), true, "show other funds")
        );
        let c = parse_conventional("ci: pin workflows").unwrap();
        assert_eq!((c.kind, c.scope), ("ci", None));
        assert!(parse_conventional("Add a thing: with colon").is_none());
        assert!(parse_conventional("no colon here").is_none());
    }

    #[test]
    fn pr_number_only_from_trailing_suffix() {
        assert_eq!(
            pr_number_from_subject("feat(bitcoin): add PSBT parser crate (#577)"),
            Some(577)
        );
        assert_eq!(pr_number_from_subject("Merge branch 'x'"), None);
        assert_eq!(pr_number_from_subject("see (#12) for context"), None);
    }

    #[test]
    fn chain_files_win_over_title_type() {
        let s = sections_for(
            "chore(near): decode with local wire types",
            &files(&[
                "src/Cargo.lock",
                "src/chain_parsers/visualsign-near/src/lib.rs",
            ]),
            &chains(),
        );
        assert_eq!(s, vec![Section::Chain("near".into())]);
    }

    #[test]
    fn scope_names_the_chain_when_no_chain_files() {
        let s = sections_for(
            "fix(sui): something",
            &files(&["docs/chains/sui.mdx"]),
            &chains(),
        );
        assert_eq!(s, vec![Section::Chain("sui".into())]);
    }

    #[test]
    fn tooling_scope_and_types() {
        let cla = sections_for(
            "ci(cla): take the org",
            &files(&[".github/workflows/cla.yml"]),
            &chains(),
        );
        assert_eq!(cla, vec![Section::Tooling]);
        let test = sections_for(
            "test: canonical ETH fixtures",
            &files(&["src/integration/tests/parser.rs"]),
            &chains(),
        );
        assert_eq!(test, vec![Section::Tooling]);
        let feat = sections_for(
            "feat: new proto field",
            &files(&["proto/parser/parser.proto"]),
            &chains(),
        );
        assert_eq!(feat, vec![Section::Core]);
    }

    #[test]
    fn wide_sweeps_go_to_core() {
        let f = files(&[
            "src/chain_parsers/visualsign-bitcoin/a.rs",
            "src/chain_parsers/visualsign-near/a.rs",
            "src/chain_parsers/visualsign-sui/a.rs",
            "src/chain_parsers/visualsign-tron/a.rs",
        ]);
        assert_eq!(
            sections_for("refactor: lint sweep", &f, &chains()),
            vec![Section::Core]
        );
    }

    #[test]
    fn tooling_type_wins_over_chain_files() {
        let s = sections_for(
            "test(sui): add fixtures",
            &files(&["src/chain_parsers/visualsign-sui/tests/fixtures.rs"]),
            &chains(),
        );
        assert_eq!(s, vec![Section::Tooling]);
    }

    #[test]
    fn scoped_wide_sweeps_go_to_core() {
        let f = files(&[
            "src/chain_parsers/visualsign-bitcoin/a.rs",
            "src/chain_parsers/visualsign-near/a.rs",
            "src/chain_parsers/visualsign-sui/a.rs",
            "src/chain_parsers/visualsign-tron/a.rs",
        ]);
        assert_eq!(
            sections_for("refactor(near): lint sweep", &f, &chains()),
            vec![Section::Core]
        );
    }

    #[test]
    fn unknown_chain_dirs_are_ignored() {
        let s = sections_for(
            "fix: fallback",
            &files(&["src/chain_parsers/visualsign-unspecified/src/lib.rs"]),
            &chains(),
        );
        assert_eq!(s, vec![Section::Core]);
    }

    #[test]
    fn release_note_section_overrides_title() {
        let body = "## Problem\nstuff\n\n## Release note\n\nEthereum unsigned input must be\nthe signing encoding.\n\nMore detail.\n## Testing\n";
        assert_eq!(
            display_line("fix(ethereum): reject unsigned input", body),
            "Ethereum unsigned input must be the signing encoding."
        );
        assert_eq!(
            display_line("fix(ethereum): reject unsigned input", "## Problem\nx"),
            "Reject unsigned input"
        );
    }

    #[test]
    fn integrator_note_and_breaking() {
        let body = "## Change\nx\n### Integrator note:\nSend the signing encoding,\nnot a signed tx.\n## Testing\ny";
        assert_eq!(
            integrator_note(body).as_deref(),
            Some("Send the signing encoding, not a signed tx.")
        );
        assert!(is_breaking("fix!: x", &[], ""));
        assert!(is_breaking("fix: x", &["breaking".into()], ""));
        assert!(is_breaking("fix: x", &[], "BREAKING CHANGE: y"));
        assert!(!is_breaking("fix: x", &[], "nothing"));
    }

    #[test]
    fn co_author_trailers_skip_bots_and_duplicates() {
        let msg = "subject\n\nbody\n\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>\nCo-authored-by: prasanna-anchorage <48452975+prasanna-anchorage@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>\nCo-authored-by: github-actions[bot] <41898282+github-actions[bot]@users.noreply.github.com>\nCo-authored-by: Jane Doe <jane@example.com>\nCo-authored-by: prasanna-anchorage <48452975+prasanna-anchorage@users.noreply.github.com>\n";
        let got = co_author_trailers(msg);
        let names: Vec<_> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["prasanna-anchorage", "Jane Doe"]);
        assert_eq!(
            got[0].noreply_login().as_deref(),
            Some("prasanna-anchorage")
        );
        assert_eq!(got[1].noreply_login(), None);
    }

    #[test]
    fn co_author_trailers_skip_bots_by_noreply_login() {
        let msg = "subject\n\nCo-authored-by: GitHub Copilot <175728472+Copilot@users.noreply.github.com>\nCo-authored-by: Some Bot <1+helper[bot]@users.noreply.github.com>\nCo-authored-by: Jane Doe <jane@example.com>\n";
        let names: Vec<_> = co_author_trailers(msg)
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Jane Doe"]);
    }

    #[test]
    fn github_apps_are_bots() {
        assert!(is_bot("app/github-actions", ""));
        assert!(!is_bot("zmanian", "zaki@example.com"));
    }
}
