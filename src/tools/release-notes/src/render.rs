//! Render a release as Slack mrkdwn or Markdown.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::chains::{ChainRow, display_name};
use crate::entries::Section;

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    /// `owner/name`.
    pub repo: String,
    pub from: String,
    pub to: String,
    pub chains: Vec<ChainRow>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub number: u64,
    pub url: String,
    pub line: String,
    pub author: String,
    pub co_authors: Vec<String>,
    pub sections: Vec<Section>,
    pub breaking: bool,
    pub integrator_note: Option<String>,
}

impl Release {
    pub fn release_url(&self) -> String {
        format!("https://github.com/{}/releases/tag/{}", self.repo, self.to)
    }

    pub fn compare_url(&self) -> String {
        format!(
            "https://github.com/{}/compare/{}...{}",
            self.repo, self.from, self.to
        )
    }

    fn name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    /// The release as `audience` sees it: entries it doesn't show are dropped, with
    /// their authors, co-authors and integrator notes.
    pub fn visible_to(&self, audience: Audience) -> Release {
        Release {
            entries: self
                .entries
                .iter()
                .filter(|e| audience.shows(e))
                .cloned()
                .collect(),
            ..self.clone()
        }
    }

    /// People to thank, in order of first appearance, bots already removed. Tooling-only
    /// contributors are included for an internal audience only.
    pub fn contributors(&self, audience: Audience) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in self.entries.iter().filter(|e| audience.shows(e)) {
            for who in std::iter::once(&e.author).chain(&e.co_authors) {
                if !who.is_empty() && !out.contains(who) {
                    out.push(who.clone());
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Audience {
    /// Chains, core and integrator notes. Safe for a shared channel.
    Public,
    /// Public, plus the tooling/infra section and co-author credit on each line.
    Internal,
}

impl Audience {
    fn shows(self, e: &Entry) -> bool {
        self == Audience::Internal || e.sections.iter().any(|s| *s != Section::Tooling)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Style {
    Slack,
    Markdown,
}

impl Style {
    fn bold(self, s: &str) -> String {
        match self {
            Style::Slack => format!("*{s}*"),
            Style::Markdown => format!("**{s}**"),
        }
    }

    fn link(self, url: &str, text: &str) -> String {
        match self {
            Style::Slack => format!("<{url}|{text}>"),
            Style::Markdown => format!("[{text}]({url})"),
        }
    }

    fn bullet(self) -> &'static str {
        match self {
            Style::Slack => "\u{2022} ",
            Style::Markdown => "- ",
        }
    }
}

pub fn render(release: &Release, style: Style, audience: Audience, footer: Option<&str>) -> String {
    let mut out = String::new();
    let b = style.bullet();

    out.push_str(&format!(
        "{} ({}, {})\n",
        style.bold(&format!("{} {}", release.name(), release.to)),
        style.link(&release.release_url(), "release"),
        style.link(
            &release.compare_url(),
            &format!("diff from {}", release.from)
        ),
    ));

    let enabled: Vec<String> = release
        .chains
        .iter()
        .filter(|c| c.enabled())
        .map(|c| {
            if c.new {
                format!("{} (new)", c.display)
            } else {
                c.display.clone()
            }
        })
        .collect();
    if !enabled.is_empty() {
        out.push_str(&format!(
            "{} {}\n",
            style.bold("Chains enabled:"),
            enabled.join(", ")
        ));
    }
    let not_enabled: Vec<&ChainRow> = release.chains.iter().filter(|c| !c.enabled()).collect();
    if !not_enabled.is_empty() {
        let names: Vec<&str> = not_enabled.iter().map(|c| c.display.as_str()).collect();
        out.push_str(&format!(
            "{} {} (in the repo, not in the default build)\n",
            style.bold("Opt-in:"),
            names.join(", ")
        ));
    }

    // Chain sections: new chains first, then by name; then Core, then Tooling.
    let mut grouped: BTreeMap<(u8, String), Vec<&Entry>> = BTreeMap::new();
    for e in release.entries.iter().filter(|e| audience.shows(e)) {
        for s in &e.sections {
            let key = match s {
                Section::Chain(c) => {
                    let new = release.chains.iter().any(|r| &r.name == c && r.new);
                    (if new { 0 } else { 1 }, c.clone())
                }
                Section::Core => (2, String::new()),
                Section::Tooling if audience == Audience::Internal => (3, String::new()),
                Section::Tooling => continue,
            };
            grouped.entry(key).or_default().push(e);
        }
    }
    for ((order, chain), entries) in &grouped {
        let heading = match order {
            0 => format!("{} (new)", display_name(chain)),
            1 => display_name(chain),
            2 => "Core".to_string(),
            _ => "Tooling/infra".to_string(),
        };
        out.push_str(&format!("\n{}\n", style.bold(&heading)));
        for e in entries {
            out.push_str(&format!("{b}{} ({})\n", e.line, credit(style, audience, e)));
        }
    }

    let notes: Vec<&Entry> = release
        .entries
        .iter()
        .filter(|e| audience.shows(e) && (e.breaking || e.integrator_note.is_some()))
        .collect();
    if !notes.is_empty() {
        out.push_str(&format!("\n{}\n", style.bold("Integrator notes")));
        for e in notes {
            let text = e.integrator_note.clone().unwrap_or_else(|| e.line.clone());
            out.push_str(&format!(
                "{b}{text} ({})\n",
                style.link(&e.url, &format!("#{}", e.number))
            ));
        }
    }

    let thanks = release.contributors(audience);
    if !thanks.is_empty() {
        out.push_str(&format!(
            "\nThanks to everyone who contributed to this release: {}.\n",
            thanks.join(", ")
        ));
    }

    if let Some(footer) = footer.map(str::trim).filter(|f| !f.is_empty()) {
        out.push_str(&format!("\n{footer}\n"));
    }
    out
}

fn credit(style: Style, audience: Audience, e: &Entry) -> String {
    let mut parts = vec![style.link(&e.url, &format!("#{}", e.number))];
    // A bot-authored PR has no author to credit.
    if !e.author.is_empty() {
        parts.push(e.author.clone());
    }
    if audience == Audience::Internal && !e.co_authors.is_empty() {
        parts.push(format!("co-author {}", e.co_authors.join(", ")));
    }
    parts.join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn row(name: &str, new: bool, enabled: bool) -> ChainRow {
        ChainRow {
            name: name.into(),
            display: display_name(name),
            new,
            app_default: enabled,
            image: enabled,
            proto: true,
        }
    }

    fn entry(n: u64, line: &str, author: &str, sections: Vec<Section>) -> Entry {
        Entry {
            number: n,
            url: format!("https://github.com/o/r/pull/{n}"),
            line: line.into(),
            author: author.into(),
            co_authors: vec![],
            sections,
            breaking: false,
            integrator_note: None,
        }
    }

    fn release() -> Release {
        let mut breaking = entry(
            2,
            "Reject non-canonical unsigned input",
            "bob",
            vec![Section::Chain("ethereum".into())],
        );
        breaking.breaking = true;
        breaking.integrator_note = Some("Send the signing encoding.".into());
        breaking.co_authors = vec!["carol".into()];
        Release {
            repo: "o/r".into(),
            from: "v1.0.0".into(),
            to: "v1.1.0".into(),
            chains: vec![
                row("bitcoin", true, true),
                row("cosmos", true, false),
                row("ethereum", false, true),
            ],
            entries: vec![
                entry(
                    1,
                    "Add PSBT parser",
                    "alice",
                    vec![Section::Chain("bitcoin".into())],
                ),
                breaking,
                entry(3, "Pin workflows", "dave", vec![Section::Tooling]),
            ],
        }
    }

    #[test]
    fn public_slack() {
        let got = render(&release(), Style::Slack, Audience::Public, None);
        let want = "\
*r v1.1.0* (<https://github.com/o/r/releases/tag/v1.1.0|release>, <https://github.com/o/r/compare/v1.0.0...v1.1.0|diff from v1.0.0>)
*Chains enabled:* Bitcoin (new), Ethereum
*Opt-in:* Cosmos (in the repo, not in the default build)

*Bitcoin (new)*
\u{2022} Add PSBT parser (<https://github.com/o/r/pull/1|#1>, alice)

*Ethereum*
\u{2022} Reject non-canonical unsigned input (<https://github.com/o/r/pull/2|#2>, bob)

*Integrator notes*
\u{2022} Send the signing encoding. (<https://github.com/o/r/pull/2|#2>)

Thanks to everyone who contributed to this release: alice, bob, carol.
";
        assert_eq!(got, want);
    }

    #[test]
    fn internal_markdown_adds_tooling_coauthors_and_footer() {
        let got = render(
            &release(),
            Style::Markdown,
            Audience::Internal,
            Some("Deploy: private notes\n"),
        );
        assert!(got.contains(
            "\n**Tooling/infra**\n- Pin workflows ([#3](https://github.com/o/r/pull/3), dave)\n"
        ));
        assert!(got.contains("([#2](https://github.com/o/r/pull/2), bob, co-author carol)"));
        assert!(got.contains("contributed to this release: alice, bob, carol, dave."));
        assert!(got.ends_with("\nDeploy: private notes\n"));
    }

    #[test]
    fn public_never_shows_tooling_or_its_people() {
        let got = render(&release(), Style::Markdown, Audience::Public, None);
        assert!(!got.contains("Tooling"));
        assert!(!got.contains("dave"));
    }

    #[test]
    fn bot_authored_entry_credits_only_people() {
        let mut r = release();
        r.entries[2].author = String::new();
        r.entries[2].co_authors = vec!["erin".into()];
        let got = render(&r, Style::Markdown, Audience::Internal, None);
        assert!(
            got.contains("- Pin workflows ([#3](https://github.com/o/r/pull/3), co-author erin)\n")
        );
    }

    #[test]
    fn public_view_drops_tooling_entries() {
        let mut r = release();
        r.entries[2].integrator_note = Some("Bump the runner image.".into());
        let public = r.visible_to(Audience::Public);
        let numbers: Vec<u64> = public.entries.iter().map(|e| e.number).collect();
        assert_eq!(numbers, [1, 2]);
        let json = serde_json::to_string(&public).unwrap();
        assert!(!json.contains("dave"));
        assert!(!json.contains("Bump the runner image."));
        assert_eq!(r.visible_to(Audience::Internal).entries.len(), 3);
    }
}
