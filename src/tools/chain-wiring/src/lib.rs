//! Checks that a chain parser is wired into every place the repo needs it.
//!
//! The touch points are data, in `touchpoints.toml` next to this crate's
//! manifest. [`check`] runs them against a repo tree for one chain and returns a
//! [`Report`]; the report renders as text (for people) or as a GitHub task list
//! (for the chain's tracking issue). [`splice`] swaps the generated block into an
//! issue body without touching anything outside it.

pub mod pr;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// The rules shipped with the tool.
pub const TOUCHPOINTS_TOML: &str = include_str!("../touchpoints.toml");

/// First line of the generated block in a tracking issue.
pub const WIRING_START: &str = "<!-- add-chain:wiring:start -->";
/// Last line of the generated block in a tracking issue.
pub const WIRING_END: &str = "<!-- add-chain:wiring:end -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `touchpoints.toml` doesn't parse or breaks a rule about its shape.
    Rules(String),
    /// A chain or variant name with characters outside the allowed set.
    BadName(String),
    /// An I/O error outside the checks themselves (which treat unreadable files as missing).
    Io(String),
    /// The issue body has no usable marker pair.
    Splice(SpliceError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rules(m) => write!(f, "touchpoints.toml: {m}"),
            Error::BadName(m) => write!(f, "{m}"),
            Error::Io(m) => write!(f, "{m}"),
            Error::Splice(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpliceError {
    NoStart,
    NoEnd,
    Duplicate,
}

impl fmt::Display for SpliceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpliceError::NoStart => write!(f, "no {WIRING_START} line; not a chain tracking issue"),
            SpliceError::NoEnd => write!(f, "no {WIRING_END} line after {WIRING_START}"),
            SpliceError::Duplicate => {
                write!(f, "more than one wiring block; fix the issue by hand")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    FileExists,
    FileContains,
    GlobExists,
    AnyFileContains,
    DefaultSetEquality,
    InfoOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Extract {
    /// `default = [...]` of a Cargo.toml, minus `not_chains`.
    CargoDefault,
    /// `ARG CHAIN_FEATURES="..."` of a Containerfile or Dockerfile.
    ChainFeaturesArg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InfoTest {
    /// Whether the chain is in the base default set.
    InDefaultSet,
    /// Whether `path` matches `pattern` (or, if `guard` doesn't match, neither).
    FileContains,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TouchPoint {
    pub id: String,
    pub section: String,
    pub kind: Kind,
    pub description: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(default)]
    pub missing: Option<String>,
    #[serde(default)]
    pub extract: Option<Extract>,
    #[serde(default)]
    pub test: Option<InfoTest>,
    #[serde(default)]
    pub guard: Option<String>,
    #[serde(default)]
    pub yes: Option<String>,
    #[serde(default)]
    pub no: Option<String>,
    #[serde(default)]
    pub unguarded: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub dir: String,
    pub prefix: String,
    #[serde(default)]
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultSet {
    pub base: String,
    #[serde(default)]
    pub not_chains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub discovery: Discovery,
    pub default_set: DefaultSet,
    #[serde(rename = "touchpoint")]
    pub touchpoints: Vec<TouchPoint>,
}

impl Rules {
    /// The rules shipped in `touchpoints.toml`.
    pub fn builtin() -> Result<Rules, Error> {
        Rules::parse(TOUCHPOINTS_TOML)
    }

    /// Parses and validates a rules file.
    pub fn parse(text: &str) -> Result<Rules, Error> {
        let rules: Rules = toml::from_str(text).map_err(|e| Error::Rules(e.to_string()))?;
        rules.validate()?;
        Ok(rules)
    }

    fn validate(&self) -> Result<(), Error> {
        let mut ids = HashSet::new();
        let sample = Names::new("sample", None)?;
        for tp in &self.touchpoints {
            let bad = |m: &str| Error::Rules(format!("touchpoint {}: {m}", tp.id));
            if !ids.insert(tp.id.as_str()) {
                return Err(bad("duplicate id"));
            }
            let needs_path = !matches!(
                (tp.kind, tp.test),
                (Kind::AnyFileContains, _) | (Kind::InfoOnly, Some(InfoTest::InDefaultSet))
            );
            if needs_path != tp.path.is_some() {
                return Err(bad(if needs_path {
                    "needs `path`"
                } else {
                    "takes no `path`"
                }));
            }
            let needs_pattern = matches!(
                (tp.kind, tp.test),
                (Kind::FileContains | Kind::AnyFileContains, _)
                    | (Kind::InfoOnly, Some(InfoTest::FileContains))
            );
            if needs_pattern != tp.pattern.is_some() {
                return Err(bad(if needs_pattern {
                    "needs `pattern`"
                } else {
                    "takes no `pattern`"
                }));
            }
            if (tp.kind == Kind::AnyFileContains) == tp.paths.is_empty() {
                return Err(bad(
                    "`paths` is for any-file-contains only, and it needs at least one",
                ));
            }
            if (tp.kind == Kind::DefaultSetEquality) != tp.extract.is_some() {
                return Err(bad(
                    "`extract` is for default-set-equality only, and it needs one",
                ));
            }
            if tp.kind == Kind::InfoOnly {
                if tp.test.is_none() || tp.yes.is_none() || tp.no.is_none() {
                    return Err(bad("info-only needs `test`, `yes` and `no`"));
                }
                if tp.guard.is_some() != tp.unguarded.is_some() {
                    return Err(bad("`guard` and `unguarded` go together"));
                }
            } else if tp.test.is_some() || tp.yes.is_some() || tp.no.is_some() || tp.guard.is_some()
            {
                return Err(bad("`test`, `yes`, `no` and `guard` are for info-only"));
            }
            for p in tp.paths.iter().chain(tp.path.iter()) {
                if matches!(tp.kind, Kind::GlobExists | Kind::AnyFileContains) {
                    glob_parts(p).map_err(|m| bad(&m))?;
                }
            }
            for p in tp.pattern.iter().chain(tp.guard.iter()) {
                compile(&sample.fill_regex(p), tp.case_insensitive).map_err(|m| bad(&m))?;
            }
        }
        Ok(())
    }

    /// Chains with a `visualsign-<chain>` crate dir, minus `skip`, sorted.
    pub fn discover_chains(&self, repo_root: &Path) -> Result<Vec<String>, Error> {
        let dir = repo_root.join(&self.discovery.dir);
        let entries =
            fs::read_dir(&dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let mut chains = Vec::new();
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(chain) = name.strip_prefix(&self.discovery.prefix)
                && !chain.is_empty()
                && !self.discovery.skip.iter().any(|s| s == chain)
            {
                chains.push(chain.to_string());
            }
        }
        chains.sort();
        Ok(chains)
    }
}

// ---------------------------------------------------------------------------
// Names and templates
// ---------------------------------------------------------------------------

/// The spellings of one chain's name that the touch points use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// `foo-bar`: crate dir suffix, Cargo feature, CLI value.
    pub chain: String,
    /// `FooBar`... or whatever `--variant` says: the Rust enum variant.
    pub variant: String,
    /// `FOO-BAR`: the proto enum suffix.
    pub upper: String,
    /// `foo_bar`: the Rust crate name suffix.
    pub snake: String,
}

impl Names {
    /// `variant` defaults to `chain` with its first letter upper-cased.
    pub fn new(chain: &str, variant: Option<&str>) -> Result<Names, Error> {
        let chain_ok = !chain.is_empty()
            && chain
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && chain.starts_with(|c: char| c.is_ascii_lowercase());
        if !chain_ok {
            return Err(Error::BadName(format!(
                "chain {chain:?}: use lowercase ASCII letters, digits and '-', starting with a letter"
            )));
        }
        let variant = match variant {
            Some(v) => v.to_string(),
            None => {
                let mut cs = chain.chars();
                cs.next()
                    .map(|c| c.to_ascii_uppercase().to_string() + cs.as_str())
                    .unwrap_or_default()
            }
        };
        let variant_ok = !variant.is_empty()
            && variant
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && variant.starts_with(|c: char| c.is_ascii_uppercase());
        if !variant_ok {
            return Err(Error::BadName(format!(
                "variant {variant:?}: use an ASCII Rust identifier starting with an upper-case letter (pass --variant)"
            )));
        }
        Ok(Names {
            chain: chain.to_string(),
            variant,
            upper: chain.to_ascii_uppercase(),
            snake: chain.replace('-', "_"),
        })
    }

    /// Fills the placeholders with the names as they are.
    pub fn fill(&self, template: &str) -> String {
        self.fill_with(template, |s| s.to_string())
    }

    /// Fills the placeholders with regex-escaped names.
    pub fn fill_regex(&self, template: &str) -> String {
        self.fill_with(template, regex::escape)
    }

    fn fill_with(&self, template: &str, f: impl Fn(&str) -> String) -> String {
        template
            .replace("{chain}", &f(&self.chain))
            .replace("{Variant}", &f(&self.variant))
            .replace("{CHAIN}", &f(&self.upper))
            .replace("{snake}", &f(&self.snake))
    }
}

fn compile(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("bad pattern {pattern:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Missing,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub section: String,
    pub status: Status,
    /// The line to print: the description when OK, the description with where
    /// to look when missing, the policy statement for INFO.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub names: Names,
    pub items: Vec<Item>,
}

impl Report {
    pub fn missing(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == Status::Missing)
            .count()
    }

    /// Touch points checked (INFO lines aren't touch points).
    pub fn total(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status != Status::Info)
            .count()
    }

    pub fn missing_ids(&self) -> BTreeSet<&str> {
        self.items
            .iter()
            .filter(|i| i.status == Status::Missing)
            .map(|i| i.id.as_str())
            .collect()
    }

    /// `28/34 touch points present, 6 missing` or `34/34 touch points present`.
    pub fn summary(&self) -> String {
        let (missing, total) = (self.missing(), self.total());
        if missing > 0 {
            format!(
                "{}/{total} touch points present, {missing} missing",
                total - missing
            )
        } else {
            format!("{total}/{total} touch points present")
        }
    }

    fn sections(&self) -> Vec<(&str, Vec<&Item>)> {
        let mut out: Vec<(&str, Vec<&Item>)> = Vec::new();
        for item in &self.items {
            match out.last_mut() {
                Some((s, items)) if *s == item.section => items.push(item),
                _ => out.push((item.section.as_str(), vec![item])),
            }
        }
        out
    }

    /// One line per check, grouped by section, with a summary at the end.
    pub fn render_text(&self) -> String {
        let n = &self.names;
        let mut out = format!(
            "Wiring for chain '{}' (variant {}, proto CHAIN_{})\n",
            n.chain, n.variant, n.upper
        );
        for (section, items) in self.sections() {
            out.push_str(section);
            out.push('\n');
            for i in items {
                let tag = match i.status {
                    Status::Ok => "OK      ",
                    Status::Missing => "MISSING ",
                    Status::Info => "INFO    ",
                };
                out.push_str(&format!("  {tag} {}\n", i.text));
            }
        }
        out.push('\n');
        out.push_str(&self.summary());
        out.push('\n');
        out
    }

    /// The checks as GitHub task lists, one per section, without the summary.
    fn markdown_sections(&self) -> String {
        let mut out = String::new();
        for (section, items) in self.sections() {
            out.push_str(&format!("\n**{section}**\n\n"));
            for i in items {
                let line = match i.status {
                    Status::Ok => format!("- [x] {}\n", i.text),
                    Status::Missing => format!("- [ ] {}\n", i.text),
                    Status::Info => format!("- {}\n", i.text),
                };
                out.push_str(&line);
            }
        }
        out
    }

    /// The checks as GitHub task lists, then the summary.
    pub fn render_markdown(&self) -> String {
        format!("{}\n{}\n", self.markdown_sections(), self.summary())
    }

    /// The M1 block for the tracking issue, markers included, ending in a newline.
    pub fn issue_block(&self) -> String {
        format!(
            "{WIRING_START}\n{}\n_Checked by `chain-wiring check {}` on the default branch: {}._\n{WIRING_END}\n",
            self.markdown_sections(),
            self.names.chain,
            self.summary()
        )
    }
}

// ---------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------

/// Runs every touch point in `rules` for one chain against the tree at `repo_root`.
pub fn check_with(rules: &Rules, repo_root: &Path, names: &Names) -> Result<Report, Error> {
    let base = read_set(
        &repo_root.join(&rules.default_set.base),
        Extract::CargoDefault,
        &rules.default_set.not_chains,
    );
    let mut items = Vec::with_capacity(rules.touchpoints.len());
    for tp in &rules.touchpoints {
        let path = tp.path.as_deref().map(|p| names.fill(p));
        let fill = |t: &str| {
            let t = names.fill(t);
            match &path {
                Some(p) => t.replace("{path}", p),
                None => t,
            }
        };
        let description = fill(&tp.description);
        let pattern = match &tp.pattern {
            Some(p) => Some(
                compile(&names.fill_regex(p), tp.case_insensitive)
                    .map_err(|m| Error::Rules(format!("touchpoint {}: {m}", tp.id)))?,
            ),
            None => None,
        };
        let full = |p: &Option<String>| repo_root.join(p.as_deref().unwrap_or_default());
        let missing_text = |default: String| tp.missing.as_deref().map(&fill).unwrap_or(default);

        let (status, text) = match tp.kind {
            Kind::FileExists => {
                if full(&path).exists() {
                    (Status::Ok, description)
                } else {
                    (Status::Missing, missing_text(description))
                }
            }
            Kind::FileContains => {
                let hit = pattern
                    .as_ref()
                    .is_some_and(|re| file_has_line(&full(&path), re));
                let p = path.clone().unwrap_or_default();
                if hit {
                    (Status::Ok, description)
                } else {
                    (
                        Status::Missing,
                        missing_text(format!("{description} ({p})")),
                    )
                }
            }
            Kind::GlobExists => {
                let p = path.clone().unwrap_or_default();
                if !glob(repo_root, &p).is_empty() {
                    (Status::Ok, description)
                } else {
                    (
                        Status::Missing,
                        missing_text(format!("{description} ({p})")),
                    )
                }
            }
            Kind::AnyFileContains => {
                let hit = pattern.as_ref().is_some_and(|re| {
                    tp.paths
                        .iter()
                        .flat_map(|g| glob(repo_root, &names.fill(g)))
                        .any(|f| file_has_line(&f, re))
                });
                if hit {
                    (Status::Ok, description)
                } else {
                    (Status::Missing, missing_text(description))
                }
            }
            Kind::DefaultSetEquality => {
                let extract = tp.extract.unwrap_or(Extract::CargoDefault);
                let set = read_set(&full(&path), extract, &rules.default_set.not_chains);
                if set == base {
                    (
                        Status::Ok,
                        format!("{description} matches parser_app default"),
                    )
                } else {
                    let t = format!(
                        "{description} differs from parser_app default [{}] vs [{}]",
                        bracket(&base),
                        bracket(&set)
                    );
                    (Status::Missing, t)
                }
            }
            Kind::InfoOnly => {
                let yes = tp.yes.as_deref().map(&fill).unwrap_or_default();
                let no = tp.no.as_deref().map(&fill).unwrap_or_default();
                let text = match tp.test {
                    Some(InfoTest::InDefaultSet) => {
                        if base.contains(&names.chain) {
                            yes
                        } else {
                            no
                        }
                    }
                    Some(InfoTest::FileContains) => {
                        let file = full(&path);
                        let guard = match &tp.guard {
                            Some(g) => {
                                Some(compile(&names.fill_regex(g), false).map_err(|m| {
                                    Error::Rules(format!("touchpoint {}: {m}", tp.id))
                                })?)
                            }
                            None => None,
                        };
                        let guarded = guard.as_ref().is_none_or(|g| file_has_line(&file, g));
                        if !guarded {
                            tp.unguarded.as_deref().map(&fill).unwrap_or_default()
                        } else if pattern.as_ref().is_some_and(|re| file_has_line(&file, re)) {
                            yes
                        } else {
                            no
                        }
                    }
                    None => String::new(),
                };
                (Status::Info, text)
            }
        };
        items.push(Item {
            id: tp.id.clone(),
            section: tp.section.clone(),
            status,
            text,
        });
    }
    Ok(Report {
        names: names.clone(),
        items,
    })
}

/// Runs the built-in touch points for `chain` against the tree at `repo_root`.
pub fn check(repo_root: &Path, chain: &str, variant: Option<&str>) -> Result<Report, Error> {
    check_with(&Rules::builtin()?, repo_root, &Names::new(chain, variant)?)
}

/// `a b c ` (each name followed by a space), or a single space for an empty set,
/// as the shell version printed it.
fn bracket(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        return " ".to_string();
    }
    set.iter().map(|s| format!("{s} ")).collect()
}

/// Reads a file as text; unreadable or missing files read as `None`.
fn read_text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn file_has_line(path: &Path, re: &Regex) -> bool {
    read_text(path).is_some_and(|t| t.lines().any(|l| re.is_match(l)))
}

/// The features a line like `default = ["a", "b"]` or `ARG CHAIN_FEATURES="a b"` lists.
pub fn extract_set(text: &str, extract: Extract, not_chains: &[String]) -> BTreeSet<String> {
    let (re, drop): (&str, &[String]) = match extract {
        Extract::CargoDefault => (r"^default *= *\[(.*)\]", not_chains),
        Extract::ChainFeaturesArg => (r#"^ARG CHAIN_FEATURES="(.*)""#, &[]),
    };
    let Ok(re) = Regex::new(re) else {
        return BTreeSet::new();
    };
    text.lines()
        .filter_map(|l| re.captures(l).and_then(|c| c.get(1)))
        .flat_map(|m| m.as_str().split([' ', ',', '"']))
        .filter(|s| !s.is_empty() && !drop.iter().any(|d| d == s))
        .map(str::to_string)
        .collect()
}

fn read_set(path: &Path, extract: Extract, not_chains: &[String]) -> BTreeSet<String> {
    read_text(path)
        .map(|t| extract_set(&t, extract, not_chains))
        .unwrap_or_default()
}

/// Splits `dir/sub/name-*.ext` into the literal directory and a file-name regex.
fn glob_parts(pattern: &str) -> Result<(&str, Regex), String> {
    let (dir, name) = pattern.rsplit_once('/').unwrap_or(("", pattern));
    if dir.contains(['*', '?', '[']) {
        return Err(format!(
            "glob {pattern:?}: wildcards are allowed in the last segment only"
        ));
    }
    let re = regex::escape(name)
        .replace(r"\*", "[^/]*")
        .replace(r"\?", "[^/]");
    Regex::new(&format!("^{re}$"))
        .map(|r| (dir, r))
        .map_err(|e| format!("glob {pattern:?}: {e}"))
}

/// Files matching a glob with wildcards in its last segment only, sorted.
fn glob(repo_root: &Path, pattern: &str) -> Vec<PathBuf> {
    let Ok((dir, name)) = glob_parts(pattern) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(repo_root.join(dir)) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| name.is_match(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Tracking issues
// ---------------------------------------------------------------------------

/// Replaces the lines from [`WIRING_START`] to [`WIRING_END`] in `body` with
/// `block` (which should carry both markers). Everything outside the markers,
/// including boxes ticked by hand, comes back byte for byte. A CRLF body gets
/// CRLF in the new block too.
pub fn splice(body: &str, block: &str) -> Result<String, SpliceError> {
    let lines: Vec<&str> = body.split('\n').collect();
    let is = |l: &str, marker: &str| l.trim_end_matches('\r').trim() == marker;
    let starts: Vec<usize> = (0..lines.len())
        .filter(|&i| is(lines[i], WIRING_START))
        .collect();
    let ends: Vec<usize> = (0..lines.len())
        .filter(|&i| is(lines[i], WIRING_END))
        .collect();
    let start = match starts.as_slice() {
        [] => return Err(SpliceError::NoStart),
        [s] => *s,
        _ => return Err(SpliceError::Duplicate),
    };
    let end = match ends.as_slice() {
        [] => return Err(SpliceError::NoEnd),
        [e] if *e > start => *e,
        [_] => return Err(SpliceError::NoEnd),
        _ => return Err(SpliceError::Duplicate),
    };
    let crlf = body.contains("\r\n");
    let new_block = block.lines().map(|l| {
        if crlf {
            format!("{l}\r")
        } else {
            l.to_string()
        }
    });
    let out: Vec<String> = lines[..start]
        .iter()
        .map(|l| l.to_string())
        .chain(new_block)
        .chain(lines[end + 1..].iter().map(|l| l.to_string()))
        .collect();
    Ok(out.join("\n"))
}

/// The answer to a field of an issue form, which GitHub renders as
/// `### <label>` followed by the answer. `None` if the field is absent or empty.
pub fn issue_field(body: &str, label: &str) -> Option<String> {
    let mut lines = body.lines().map(|l| l.trim_end_matches('\r'));
    lines.find(|l| l.trim() == format!("### {label}"))?;
    let answer = lines
        .take_while(|l| !l.starts_with("### "))
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    if answer == "_No response_" {
        None
    } else {
        Some(answer.to_string())
    }
}

/// The chain names a tracking issue made from the chain-support form declares.
pub fn issue_names(body: &str) -> Result<Names, Error> {
    let chain = issue_field(body, "Chain name")
        .ok_or_else(|| Error::BadName("issue has no \"Chain name\" answer".to_string()))?;
    let variant = issue_field(body, "Rust variant");
    Names::new(&chain, variant.as_deref())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn builtin_rules_parse_and_keep_every_check() {
        let rules = Rules::builtin().unwrap();
        let counted = rules
            .touchpoints
            .iter()
            .filter(|t| t.kind != Kind::InfoOnly)
            .count();
        // The shell script this replaced checked 34 touch points; dropping one changes behaviour.
        assert_eq!(counted, 34);
        assert_eq!(rules.touchpoints.len() - counted, 2);
    }

    #[test]
    fn names_default_and_override() {
        let n = Names::new("near", None).unwrap();
        assert_eq!(
            (n.variant.as_str(), n.upper.as_str(), n.snake.as_str()),
            ("Near", "NEAR", "near")
        );
        // A hyphenated chain can't derive a Rust variant; it has to be given.
        assert!(Names::new("foo-bar", None).is_err());
        let n = Names::new("foo-bar", Some("FooBar")).unwrap();
        assert_eq!(
            (n.variant.as_str(), n.upper.as_str(), n.snake.as_str()),
            ("FooBar", "FOO-BAR", "foo_bar")
        );
    }

    #[test]
    fn names_reject_bad_input() {
        assert!(Names::new("Near", None).is_err());
        assert!(Names::new("ne ar", None).is_err());
        assert!(Names::new("", None).is_err());
        assert!(Names::new("near", Some("near")).is_err());
        assert!(Names::new("near", Some("Ne$r")).is_err());
        assert_eq!(Names::new("near", Some("NearX")).unwrap().variant, "NearX");
    }

    #[test]
    fn placeholders_are_escaped_in_patterns() {
        let n = Names::new("a-b", Some("AB")).unwrap();
        assert_eq!(n.fill_regex("x{chain}y{Variant}"), r"xa\-byAB");
        assert_eq!(n.fill("x{chain}y{snake}"), "xa-bya_b");
    }

    #[test]
    fn extract_set_reads_cargo_default_and_chain_features() {
        let skip = vec!["vsock".to_string()];
        let cargo = "[features]\ndefault = [\"b\", \"a\", \"vsock\"]\nother = []\n";
        let set: Vec<_> = extract_set(cargo, Extract::CargoDefault, &skip)
            .into_iter()
            .collect();
        assert_eq!(set, ["a", "b"]);
        let docker = "FROM x\nARG CHAIN_FEATURES=\"c a b\"\n";
        let set: Vec<_> = extract_set(docker, Extract::ChainFeaturesArg, &skip)
            .into_iter()
            .collect();
        assert_eq!(set, ["a", "b", "c"]);
    }

    #[test]
    fn rules_reject_bad_shapes() {
        let head = "[discovery]\ndir='d'\nprefix='p-'\n[default_set]\nbase='b'\n";
        let ok = format!(
            "{head}[[touchpoint]]\nid='a'\nsection='s'\nkind='file-exists'\npath='x'\ndescription='d'\n"
        );
        assert!(Rules::parse(&ok).is_ok());
        let dup = format!("{ok}{}", &ok[head.len()..]);
        assert!(Rules::parse(&dup).is_err(), "duplicate id");
        let bad_re = format!(
            "{head}[[touchpoint]]\nid='a'\nsection='s'\nkind='file-contains'\npath='x'\npattern='('\ndescription='d'\n"
        );
        assert!(Rules::parse(&bad_re).is_err(), "bad regex");
        let no_pattern = format!(
            "{head}[[touchpoint]]\nid='a'\nsection='s'\nkind='file-contains'\npath='x'\ndescription='d'\n"
        );
        assert!(Rules::parse(&no_pattern).is_err(), "missing pattern");
        let typo = format!(
            "{head}[[touchpoint]]\nid='a'\nsection='s'\nkind='file-exists'\npath='x'\ndescription='d'\npatern='x'\n"
        );
        assert!(Rules::parse(&typo).is_err(), "unknown field");
        let deep_glob = format!(
            "{head}[[touchpoint]]\nid='a'\nsection='s'\nkind='glob-exists'\npath='a/*/b'\ndescription='d'\n"
        );
        assert!(Rules::parse(&deep_glob).is_err(), "wildcard in a directory");
    }

    #[test]
    fn issue_field_reads_form_answers() {
        let body =
            "### Chain name\n\nnear\n\n### Rust variant\n\n_No response_\n\n### Proto slot\n\n9\n";
        assert_eq!(issue_field(body, "Chain name").as_deref(), Some("near"));
        assert_eq!(issue_field(body, "Rust variant"), None);
        assert_eq!(issue_field(body, "Proto slot").as_deref(), Some("9"));
        assert_eq!(issue_field(body, "Missing"), None);
        let n = issue_names(body).unwrap();
        assert_eq!((n.chain.as_str(), n.variant.as_str()), ("near", "Near"));
        let crlf = body.replace('\n', "\r\n");
        assert_eq!(issue_names(&crlf).unwrap(), n);
        assert!(issue_names("### Chain name\n\nrm -rf /\n").is_err());
    }
}
