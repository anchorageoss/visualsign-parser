//! Write a release into the Mintlify docs site (`docs/`): an `<Update>` per chain
//! changelog page and the core page, and for a newly enabled chain its own changelog
//! page, a blog post, the blog index stub and the `docs.json` navigation entries.
//!
//! Every function here maps existing file contents to new contents, and re-running for
//! the same release changes nothing. Only the public view of a release is written: the
//! docs site is public.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::chains::ChainRow;
use crate::entries::Section;
use crate::render::{Audience, Entry, Release};

/// Docs-relative paths.
const DOCS_JSON: &str = "docs.json";
const BLOG_INDEX: &str = "blog.mdx";
const CHANGELOG_INDEX: &str = "changelog.mdx";
const CHAIN_STREAMS_GROUP: &str = "Chain streams";
const BLOG_GROUP: &str = "Blog";

/// What to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Changelog entries for every chain and core, plus blog posts for new chains.
    All,
    /// Only when the release enables a new chain: its posts and changelog page.
    NewChainsOnly,
}

/// File path (relative to the docs dir) -> new contents, for files that change.
pub type Plan = BTreeMap<PathBuf, String>;

/// Reads a docs file; `None` when it doesn't exist.
pub trait Docs {
    fn read(&self, path: &str) -> Option<String>;
}

pub struct DocsDir<'a>(pub &'a Path);

impl Docs for DocsDir<'_> {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path)).ok()
    }
}

pub fn apply(dir: &Path, plan: &Plan) -> Result<()> {
    for (path, contents) in plan {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&full, contents).with_context(|| format!("write {}", full.display()))?;
    }
    Ok(())
}

/// Every file change needed to publish `release` (dated `date`, `YYYY-MM-DD`).
pub fn plan(release: &Release, date: &str, scope: Scope, docs: &dyn Docs) -> Result<Plan> {
    let release = release.visible_to(Audience::Public);
    let mut plan = Plan::new();
    let new_chains: Vec<&ChainRow> = release
        .chains
        .iter()
        .filter(|c| c.new && c.enabled())
        .collect();
    if scope == Scope::NewChainsOnly && new_chains.is_empty() {
        return Ok(plan);
    }

    // Reads see earlier edits in this plan, so several edits to one file compose.
    let read = |plan: &Plan, path: &str| -> Option<String> {
        plan.get(Path::new(path))
            .cloned()
            .or_else(|| docs.read(path))
    };
    let mut docs_json =
        read(&plan, DOCS_JSON).context("docs.json not found; is --write-mintlify the docs dir?")?;

    for chain in &new_chains {
        let page = format!("changelog/{}.mdx", chain.name);
        if read(&plan, &page).is_none() {
            plan.insert(
                page.into(),
                chain_changelog_page(&chain.display, &chain.name),
            );
            docs_json = add_nav_page(
                &docs_json,
                CHAIN_STREAMS_GROUP,
                &format!("changelog/{}", chain.name),
                Position::End,
            )?;
        }
        if let Some(index) = read(&plan, CHANGELOG_INDEX)
            && let Some(updated) = add_changelog_card(&index, &chain.display, &chain.name)
        {
            plan.insert(CHANGELOG_INDEX.into(), updated);
        }

        let chain_doc = read(&plan, &format!("chains/{}.mdx", chain.name));
        let summary = chain_doc
            .as_deref()
            .map(chain_doc_summary)
            .unwrap_or_default();
        let slug = format!("{date}-{}-support", chain.name);
        let entries = entries_in(&release, &Section::Chain(chain.name.clone()));
        let post_path = format!("blog/{slug}.mdx");
        if read(&plan, &post_path).is_none() {
            plan.insert(
                post_path.into(),
                blog_post(&release, chain, &entries, &summary, chain_doc.is_some()),
            );
            docs_json = add_nav_page(
                &docs_json,
                BLOG_GROUP,
                &format!("blog/{slug}"),
                Position::AfterFirst,
            )?;
        }
        if let Some(index) = read(&plan, BLOG_INDEX)
            && let Some(updated) =
                insert_update(&index, &blog_stub(date, chain, &summary, &slug), &slug)
        {
            plan.insert(BLOG_INDEX.into(), updated);
        }
    }

    // Changelog entries: every section in the public view, or only the new chains.
    let mut sections: Vec<Section> = Vec::new();
    for e in &release.entries {
        for s in &e.sections {
            if !sections.contains(s) {
                sections.push(s.clone());
            }
        }
    }
    for section in sections {
        let page = match &section {
            Section::Chain(c)
                if scope == Scope::NewChainsOnly && !new_chains.iter().any(|n| &n.name == c) =>
            {
                continue;
            }
            Section::Chain(c) => format!("changelog/{c}.mdx"),
            Section::Core if scope == Scope::All => "changelog/core.mdx".to_string(),
            Section::Core | Section::Tooling => continue,
        };
        let Some(existing) = read(&plan, &page) else {
            eprintln!(
                "skipping {page}: no such changelog page (add one to publish this chain's entries)"
            );
            continue;
        };
        let block = update_block(
            &format!("{date} \u{2014} {}", release.to),
            &release.to,
            &entries_in(&release, &section),
        );
        if let Some(updated) = insert_update(
            &existing,
            &block,
            &format!("description=\"{}\"", release.to),
        ) {
            plan.insert(page.into(), updated);
        }
    }

    if docs.read(DOCS_JSON).as_deref() != Some(docs_json.as_str()) {
        plan.insert(DOCS_JSON.into(), docs_json);
    }
    Ok(plan)
}

fn entries_in<'a>(release: &'a Release, section: &Section) -> Vec<&'a Entry> {
    release
        .entries
        .iter()
        .filter(|e| e.sections.contains(section))
        .collect()
}

fn entry_line(e: &Entry) -> String {
    let who = if e.author.is_empty() {
        String::new()
    } else {
        format!(", {}", e.author)
    };
    format!(
        "- {} ([#{}]({}){who})",
        escape_mdx(&e.line),
        e.number,
        e.url
    )
}

/// An `<Update>` listing `entries`, with their integrator notes after the list.
pub fn update_block(label: &str, description: &str, entries: &[&Entry]) -> String {
    let mut body: Vec<String> = entries.iter().map(|e| entry_line(e)).collect();
    for e in entries {
        if let Some(note) = &e.integrator_note {
            body.push(String::new());
            body.push(format!(
                "**Integrator note** ([#{}]({})): {}",
                e.number,
                e.url,
                escape_mdx(note)
            ));
        }
    }
    let body: Vec<String> = body
        .into_iter()
        .map(|l| if l.is_empty() { l } else { format!("  {l}") })
        .collect();
    format!(
        "<Update label=\"{}\" description=\"{}\">\n{}\n</Update>\n",
        attr(label),
        attr(description),
        body.join("\n")
    )
}

/// Put `block` before the page's first `<Update>` (newest first), or at the end. `None`
/// when `marker` is already on the page, so a release is never added twice.
pub fn insert_update(page: &str, block: &str, marker: &str) -> Option<String> {
    if page.contains(marker) {
        return None;
    }
    Some(match page.find("<Update") {
        Some(at) => format!("{}{block}\n{}", &page[..at], &page[at..]),
        None => format!("{}\n\n{block}", page.trim_end()),
    })
}

pub fn chain_changelog_page(display: &str, chain: &str) -> String {
    format!(
        "---\ntitle: \"Changelog \u{2014} {display}\"\ndescription: \"{display} decoders and decoder fixes. One entry per meaningful release.\"\n---\n\n<Icon icon=\"rss\" /> [Subscribe](/changelog/{chain}/rss.xml)\n"
    )
}

/// A card for the chain on the changelog index, before the Core card. `None` when the
/// index already links the chain or has no Core card to anchor on.
pub fn add_changelog_card(index: &str, display: &str, chain: &str) -> Option<String> {
    let href = format!("href=\"/changelog/{chain}\"");
    if index.contains(&href) {
        return None;
    }
    let at = index.find("<Card title=\"Core\"")?;
    let indent: String = index[..at]
        .chars()
        .rev()
        .take_while(|c| *c == ' ')
        .collect();
    let card = format!(
        "<Card title=\"{display}\" icon=\"link\" {href}>\n{indent}  {display} transaction decoding and decoder fixes.\n{indent}</Card>\n{indent}"
    );
    Some(format!("{}{card}{}", &index[..at], &index[at..]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Right after the group's first page (the index page), so newest posts come first.
    AfterFirst,
    End,
}

/// Add `page` to the `pages` array of the navigation group named `group`, keeping the
/// file's formatting. Unchanged if the page is already listed anywhere.
pub fn add_nav_page(
    docs_json: &str,
    group: &str,
    page: &str,
    position: Position,
) -> Result<String> {
    let quoted = format!("\"{page}\"");
    if docs_json.contains(&quoted) {
        return Ok(docs_json.to_string());
    }
    let group_key = format!("\"group\": \"{group}\"");
    let Some(g) = docs_json.find(&group_key) else {
        bail!("docs.json has no navigation group {group:?}");
    };
    let Some(open) = docs_json[g..]
        .find("\"pages\": [")
        .map(|i| g + i + "\"pages\": [".len())
    else {
        bail!("docs.json group {group:?} has no pages array");
    };
    let Some(close) = docs_json[open..].find(']').map(|i| open + i) else {
        bail!("docs.json group {group:?} pages array is not closed");
    };
    let items = &docs_json[open..close];
    let indent: String = items
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| *c == ' ').collect())
        .unwrap_or_else(|| "  ".to_string());
    let insert_at = match position {
        Position::End => items.trim_end().len(),
        Position::AfterFirst => match items.find(',') {
            Some(comma) => comma + 1,
            None => items.trim_end().len(),
        },
    };
    let (before, after) = items.split_at(insert_at);
    let new_items = if before.trim().is_empty() {
        format!("\n{indent}{quoted}{after}")
    } else if position == Position::AfterFirst && !after.trim().is_empty() {
        format!("{before}\n{indent}{quoted},{after}")
    } else {
        format!("{before},\n{indent}{quoted}{after}")
    };
    Ok(format!(
        "{}{new_items}{}",
        &docs_json[..open],
        &docs_json[close..]
    ))
}

/// What a chain's docs page says about it: the frontmatter description and the first
/// prose paragraph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChainSummary {
    pub description: String,
    pub lead: String,
}

pub fn chain_doc_summary(mdx: &str) -> ChainSummary {
    let mut summary = ChainSummary::default();
    let mut rest = mdx;
    if let Some(after) = mdx.strip_prefix("---\n")
        && let Some(end) = after.find("\n---")
    {
        for line in after[..end].lines() {
            if let Some(d) = line.strip_prefix("description:") {
                summary.description = d.trim().trim_matches('"').to_string();
            }
        }
        rest = &after[end + "\n---".len()..];
    }
    summary.lead = rest
        .split("\n\n")
        .map(str::trim)
        .find(|p| {
            !p.is_empty()
                && !p.starts_with('#')
                && !p.starts_with('<')
                && !p.starts_with("import ")
                && !p.starts_with("```")
                && !p.starts_with('-')
                && !p.starts_with('|')
        })
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    summary
}

pub fn blog_post(
    release: &Release,
    chain: &ChainRow,
    entries: &[&Entry],
    summary: &ChainSummary,
    has_chain_doc: bool,
) -> String {
    let display = &chain.display;
    let description = if summary.description.is_empty() {
        format!("VisualSign now parses {display} transactions into signable payloads.")
    } else {
        // Chain doc descriptions are title-cased phrases; keep them as written.
        format!(
            "VisualSign now parses {display}: {}.",
            summary.description.trim_end_matches('.')
        )
    };
    let mut out = format!(
        "---\ntitle: \"{display} support in VisualSign\"\ndescription: \"{}\"\n---\n\n",
        attr(&description)
    );
    out.push_str(&format!(
        "VisualSign [{}]({}) adds {display} to the chains the parser decodes, and it is enabled in the default build.",
        release.to,
        release.release_url()
    ));
    if !summary.lead.is_empty() {
        out.push_str(&format!(" {}", summary.lead));
    }
    out.push_str("\n\n## What shipped\n\n");
    for e in entries {
        out.push_str(&entry_line(e));
        out.push('\n');
    }
    out.push_str("\n## Learn more\n\n");
    if has_chain_doc {
        out.push_str(&format!(
            "- [{display} chain guide](/chains/{}): input formats, what the parser shows, and what it refuses.\n",
            chain.name
        ));
    }
    out.push_str(&format!(
        "- [{display} changelog](/changelog/{}): further {display} changes, release by release.\n",
        chain.name
    ));
    out.push_str(&format!(
        "- [Release notes for {}]({}).\n",
        release.to,
        release.release_url()
    ));

    let mut people: Vec<&str> = Vec::new();
    for e in entries {
        for who in std::iter::once(&e.author).chain(&e.co_authors) {
            if !who.is_empty() && !people.contains(&who.as_str()) {
                people.push(who);
            }
        }
    }
    if !people.is_empty() {
        out.push_str(&format!(
            "\nThanks to {} for the {display} work.\n",
            people.join(", ")
        ));
    }
    out
}

pub fn blog_stub(date: &str, chain: &ChainRow, summary: &ChainSummary, slug: &str) -> String {
    let display = &chain.display;
    let lead = if summary.lead.is_empty() {
        format!("VisualSign now parses {display} transactions.")
    } else {
        escape_mdx(&summary.lead)
    };
    format!(
        "<Update label=\"{date} \u{2014} {display} support\" tags={{[\"{display}\", \"Ecosystem\"]}}>\n  {lead} [Read the full post](/blog/{slug}).\n</Update>\n"
    )
}

/// Escape MDX's JSX and expression characters outside backtick code spans.
pub fn escape_mdx(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_code = false;
    for c in s.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                out.push(c);
            }
            '<' if !in_code => out.push_str("&lt;"),
            '>' if !in_code => out.push_str("&gt;"),
            '{' if !in_code => out.push_str("&#123;"),
            '}' if !in_code => out.push_str("&#125;"),
            c => out.push(c),
        }
    }
    out
}

fn attr(s: &str) -> String {
    s.replace('"', "&quot;")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::chains::display_name;

    struct Fake(BTreeMap<String, String>);

    impl Docs for Fake {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }

    const DOCS_JSON_FIXTURE: &str = r#"{
  "navigation": {
    "tabs": [
      {
        "tab": "Changelog",
        "groups": [
          {
            "group": "Chain streams",
            "pages": [
              "changelog/solana",
              "changelog/near"
            ]
          }
        ]
      },
      {
        "tab": "Blog",
        "groups": [
          {
            "group": "Blog",
            "pages": [
              "blog",
              "blog/2026-05-13-composable-solana-presets"
            ]
          }
        ]
      }
    ]
  }
}
"#;

    const CHANGELOG_INDEX_FIXTURE: &str = "<CardGroup cols={2}>\n  <Card title=\"NEAR\" icon=\"link\" href=\"/changelog/near\">\n    NEAR.\n  </Card>\n  <Card title=\"Core\" icon=\"cube\" href=\"/changelog/core\">\n    Core.\n  </Card>\n</CardGroup>\n";

    fn row(name: &str, new: bool) -> ChainRow {
        ChainRow {
            name: name.into(),
            display: display_name(name),
            new,
            app_default: true,
            image: true,
            proto: true,
        }
    }

    fn entry(n: u64, line: &str, section: Section) -> Entry {
        Entry {
            number: n,
            url: format!("https://github.com/o/r/pull/{n}"),
            line: line.into(),
            author: "alice".into(),
            co_authors: vec![],
            sections: vec![section],
            breaking: false,
            integrator_note: None,
        }
    }

    fn release() -> Release {
        let mut eth = entry(
            2,
            "Reject non-canonical input",
            Section::Chain("ethereum".into()),
        );
        eth.integrator_note = Some("Send the signing encoding.".into());
        Release {
            repo: "o/r".into(),
            from: "v1.0.0".into(),
            to: "v1.1.0".into(),
            chains: vec![row("bitcoin", true), row("ethereum", false)],
            entries: vec![
                entry(
                    1,
                    "Add PSBT parser for Vec<u8> {inputs}",
                    Section::Chain("bitcoin".into()),
                ),
                eth,
                entry(3, "Pin CI", Section::Tooling),
                entry(4, "New proto field", Section::Core),
            ],
        }
    }

    fn docs() -> Fake {
        let mut m = BTreeMap::new();
        m.insert("docs.json".into(), DOCS_JSON_FIXTURE.into());
        m.insert("changelog.mdx".into(), CHANGELOG_INDEX_FIXTURE.into());
        m.insert(
            "blog.mdx".into(),
            "---\ntitle: \"Blog\"\n---\n\nIntro.\n\n<Update label=\"old\">\n  Old.\n</Update>\n"
                .into(),
        );
        m.insert("changelog/ethereum.mdx".into(), "---\ntitle: \"Changelog\"\n---\n\nIntro.\n\n<Update label=\"old\" description=\"v0.9.0\">\n  Old.\n</Update>\n".into());
        m.insert(
            "changelog/core.mdx".into(),
            "---\ntitle: \"Core\"\n---\n\nIntro.\n".into(),
        );
        m.insert(
            "chains/bitcoin.mdx".into(),
            "---\ntitle: Bitcoin\ndescription: Partially Signed Bitcoin Transactions (BIP-174)\n---\n\nThe Bitcoin module decodes PSBTs\ninto payloads.\n\n## Architecture\n".into(),
        );
        Fake(m)
    }

    fn path(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    #[test]
    fn full_plan_for_a_release_with_a_new_chain() {
        let plan = plan(&release(), "2026-10-27", Scope::All, &docs()).unwrap();
        let mut keys: Vec<_> = plan.keys().map(|p| p.display().to_string()).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "blog.mdx",
                "blog/2026-10-27-bitcoin-support.mdx",
                "changelog.mdx",
                "changelog/bitcoin.mdx",
                "changelog/core.mdx",
                "changelog/ethereum.mdx",
                "docs.json",
            ]
        );

        let btc = &plan[&path("changelog/bitcoin.mdx")];
        assert!(btc.starts_with("---\ntitle: \"Changelog \u{2014} Bitcoin\""));
        assert!(btc.contains(
            "<Update label=\"2026-10-27 \u{2014} v1.1.0\" description=\"v1.1.0\">\n  - Add PSBT parser for Vec&lt;u8&gt; &#123;inputs&#125; ([#1](https://github.com/o/r/pull/1), alice)\n</Update>\n"
        ));

        let eth = &plan[&path("changelog/ethereum.mdx")];
        let new_at = eth.find("description=\"v1.1.0\"").unwrap();
        assert!(
            new_at < eth.find("description=\"v0.9.0\"").unwrap(),
            "newest first"
        );
        assert!(eth.contains("  **Integrator note** ([#2](https://github.com/o/r/pull/2)): Send the signing encoding.\n"));

        assert!(plan[&path("changelog/core.mdx")].contains("New proto field"));
        assert!(
            plan.values().all(|f| !f.contains("Pin CI")),
            "tooling never reaches the public site"
        );

        let json = &plan[&path("docs.json")];
        assert!(json.contains("\"changelog/near\",\n              \"changelog/bitcoin\"\n"));
        assert!(json.contains("\"blog\",\n              \"blog/2026-10-27-bitcoin-support\",\n              \"blog/2026-05-13"));
        serde_json::from_str::<serde_json::Value>(json).unwrap();

        let post = &plan[&path("blog/2026-10-27-bitcoin-support.mdx")];
        assert!(post.contains("title: \"Bitcoin support in VisualSign\""));
        assert!(post.contains("description: \"VisualSign now parses Bitcoin: Partially Signed Bitcoin Transactions (BIP-174).\""));
        assert!(post.contains("The Bitcoin module decodes PSBTs into payloads."));
        assert!(post.contains("- [Bitcoin chain guide](/chains/bitcoin)"));
        assert!(post.contains("Thanks to alice for the Bitcoin work."));

        let blog = &plan[&path("blog.mdx")];
        assert!(
            blog.find("2026-10-27-bitcoin-support").unwrap() < blog.find("label=\"old\"").unwrap()
        );
        assert!(
            plan[&path("changelog.mdx")]
                .contains("<Card title=\"Bitcoin\" icon=\"link\" href=\"/changelog/bitcoin\">")
        );
    }

    #[test]
    fn rerunning_changes_nothing() {
        let first = plan(&release(), "2026-10-27", Scope::All, &docs()).unwrap();
        let mut after = docs();
        for (p, c) in &first {
            after.0.insert(p.display().to_string(), c.clone());
        }
        let second = plan(&release(), "2026-10-27", Scope::All, &after).unwrap();
        assert!(
            second.is_empty(),
            "unexpected changes: {:?}",
            second.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn new_chains_only_skips_releases_without_one() {
        let mut r = release();
        r.chains = vec![row("bitcoin", false), row("ethereum", false)];
        assert!(
            plan(&r, "2026-10-27", Scope::NewChainsOnly, &docs())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn new_chains_only_writes_only_the_new_chain() {
        let plan = plan(&release(), "2026-10-27", Scope::NewChainsOnly, &docs()).unwrap();
        assert!(!plan.contains_key(&path("changelog/ethereum.mdx")));
        assert!(!plan.contains_key(&path("changelog/core.mdx")));
        assert!(plan.contains_key(&path("changelog/bitcoin.mdx")));
        assert!(plan.contains_key(&path("blog/2026-10-27-bitcoin-support.mdx")));
    }

    #[test]
    fn opt_in_new_chain_gets_no_post() {
        let mut r = release();
        r.chains[0].app_default = false;
        let plan = plan(&r, "2026-10-27", Scope::All, &docs()).unwrap();
        assert!(!plan.keys().any(|p| p.starts_with("blog")));
        assert!(!plan.contains_key(&path("changelog/bitcoin.mdx")));
    }

    #[test]
    fn escape_leaves_code_spans_alone() {
        assert_eq!(
            escape_mdx("a <b> `Vec<u8>` {c}"),
            "a &lt;b&gt; `Vec<u8>` &#123;c&#125;"
        );
    }

    #[test]
    fn nav_insert_keeps_valid_json_and_is_idempotent() {
        let once = add_nav_page(
            DOCS_JSON_FIXTURE,
            "Chain streams",
            "changelog/sui",
            Position::End,
        )
        .unwrap();
        serde_json::from_str::<serde_json::Value>(&once).unwrap();
        assert_eq!(
            add_nav_page(&once, "Chain streams", "changelog/sui", Position::End).unwrap(),
            once
        );
        assert!(add_nav_page(DOCS_JSON_FIXTURE, "Nope", "x", Position::End).is_err());
    }

    #[test]
    fn chain_summary_from_doc() {
        let s = chain_doc_summary(
            "---\ntitle: X\ndescription: \"Thing\"\n---\n\nimport A from 'a'\n\n## H\n\nLead para\nwraps.\n",
        );
        assert_eq!(s.description, "Thing");
        assert_eq!(s.lead, "Lead para wraps.");
    }
}
