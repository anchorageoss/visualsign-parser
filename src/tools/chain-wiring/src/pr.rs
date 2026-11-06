//! Pure pieces of the per-PR check: which chains a PR touches, what its body
//! declares, how `known-gaps.toml` changed, and which proto slots collide.
//! The binary feeds these from git, the event JSON and `gh`.

use crate::{Error, Names};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

/// `[<chain>] <touchpoint-id> = "<reason>"`, as in `known-gaps.toml`.
pub type KnownGaps = BTreeMap<String, BTreeMap<String, String>>;

/// `CHAIN_<X> = <n>` entries of the proto `Chain` enum, as (`X`, `n`).
pub type Slots = Vec<(String, u32)>;

/// Label that lets a PR add to `known-gaps.toml`.
pub const ALLOW_GAP_LABEL: &str = "chain-wiring:allow-gap";

pub fn parse_known_gaps(text: &str) -> Result<KnownGaps, Error> {
    toml::from_str(text).map_err(|e| Error::Rules(format!("known-gaps.toml: {e}")))
}

/// Entries in `head` that aren't in `base`: a new chain table or a new id.
pub fn added_gaps(base: &KnownGaps, head: &KnownGaps) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (chain, ids) in head {
        for id in ids.keys() {
            if !base.get(chain).is_some_and(|b| b.contains_key(id)) {
                out.push((chain.clone(), id.clone()));
            }
        }
    }
    out
}

/// The gaps `chain` may have. An existing chain gets its own `known-gaps.toml`
/// entries. A new chain gets none, except `repo_wide` checks (ones that don't
/// depend on the chain) that some existing chain already lists.
pub fn allowed_gaps(
    chain: &str,
    is_new: bool,
    known: &KnownGaps,
    repo_wide: &[&str],
) -> BTreeMap<String, String> {
    if !is_new {
        return known.get(chain).cloned().unwrap_or_default();
    }
    let mut out = BTreeMap::new();
    for id in repo_wide {
        if let Some(reason) = known.values().find_map(|ids| ids.get(*id)) {
            out.insert(id.to_string(), format!("repo-wide: {reason}"));
        }
    }
    out
}

/// One line of `git diff --name-status`: `A\tpath`, `M\tpath`, `R100\told\tnew`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub status: char,
    pub path: String,
}

pub fn parse_name_status(text: &str) -> Vec<Change> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let status = parts.next()?.chars().next()?;
            let path = parts.next_back()?.to_string();
            Some(Change { status, path })
        })
        .collect()
}

/// `src/chain_parsers/visualsign-<x>/...` -> `x`.
fn crate_chain(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("src/chain_parsers/visualsign-")?;
    let (chain, _) = rest.split_once('/')?;
    Some(chain)
}

/// `CHAIN_<X> = <n>;` lines of the proto `enum Chain` block, in file order.
pub fn proto_chain_slots(proto: &str) -> Slots {
    let Ok(re) = Regex::new(r"^\s*CHAIN_([A-Z0-9_]+)\s*=\s*([0-9]+)\s*;") else {
        return Vec::new();
    };
    let mut in_enum = false;
    let mut out = Vec::new();
    for line in proto.lines() {
        let t = line.trim();
        if t.starts_with("enum Chain ") || t == "enum Chain{" || t == "enum Chain" {
            in_enum = true;
            continue;
        }
        if in_enum && t.starts_with('}') {
            break;
        }
        if in_enum
            && let Some(c) = re.captures(line)
            && let (Some(name), Ok(n)) = (c.get(1), c[2].parse())
        {
            out.push((name.as_str().to_string(), n));
        }
    }
    out
}

/// `CHAIN_<X>` names that a unified diff of the proto adds (and doesn't remove).
pub fn proto_added_chains(diff: &str) -> BTreeSet<String> {
    let Ok(re) = Regex::new(r"^\s*CHAIN_([A-Z0-9_]+)\s*=\s*[0-9]+\s*;") else {
        return BTreeSet::new();
    };
    let names = |sign: char| -> BTreeSet<String> {
        diff.lines()
            .filter(|l| l.starts_with(sign) && !l.starts_with("+++") && !l.starts_with("---"))
            .filter_map(|l| re.captures(&l[1..]).and_then(|c| c.get(1)))
            .map(|m| m.as_str().to_string())
            .collect()
    };
    let removed = names('-');
    names('+')
        .into_iter()
        .filter(|n| !removed.contains(n))
        .collect()
}

/// The chains a PR affects: crate paths it touches, `chain:<x>` labels, and
/// `CHAIN_<X>` values it adds to the proto. Names that aren't valid chain
/// names, and `skip`ped ones, are dropped.
pub fn changed_chains(
    changes: &[Change],
    labels: &[String],
    proto_diff: &str,
    skip: &[String],
) -> BTreeSet<String> {
    let from_paths = changes
        .iter()
        .filter_map(|c| crate_chain(&c.path))
        .map(str::to_string);
    let from_labels = labels
        .iter()
        .filter_map(|l| l.strip_prefix("chain:"))
        .map(str::to_string);
    let from_proto = proto_added_chains(proto_diff)
        .into_iter()
        .map(|n| n.to_ascii_lowercase());
    from_paths
        .chain(from_labels)
        .chain(from_proto)
        .filter(|c| Names::new(c, None).is_ok() && !skip.contains(c))
        .collect()
}

/// Chains whose crate manifest the PR adds.
pub fn new_chain_crates(changes: &[Change]) -> BTreeSet<String> {
    changes
        .iter()
        .filter(|c| c.status == 'A')
        .filter_map(|c| {
            let chain = crate_chain(&c.path)?;
            (c.path == format!("src/chain_parsers/visualsign-{chain}/Cargo.toml"))
                .then(|| chain.to_string())
        })
        .collect()
}

/// A heading line (`#`..`######`, or a line that is all bold), lower-cased with
/// punctuation folded to single spaces.
fn heading(line: &str) -> Option<String> {
    let t = line.trim();
    let text = if let Some(h) = t.strip_prefix('#') {
        h.trim_start_matches('#')
            .trim()
            .trim_end_matches('#')
            .trim()
    } else if t.len() > 4 && t.starts_with("**") {
        t.trim_end_matches(':')
            .strip_prefix("**")?
            .strip_suffix("**")?
            .trim()
    } else {
        return None;
    };
    let norm: Vec<String> = text
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    (!norm.is_empty()).then(|| norm.join(" "))
}

/// The PR body split into (normalised heading, text under it).
fn sections(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        match heading(line) {
            Some(h) => out.push((h, String::new())),
            None => {
                if let Some((_, text)) = out.last_mut() {
                    text.push_str(line.trim_end_matches('\r'));
                    text.push('\n');
                }
            }
        }
    }
    out
}

fn is_default_on(h: &str) -> bool {
    h.contains("default on") && h.contains("opt in")
}
fn is_network_trust(h: &str) -> bool {
    h.contains("network trust")
}
fn is_c_deps(h: &str) -> bool {
    h.contains("c dependencies") || h.contains("c deps")
}

/// What a new chain's PR body must have and doesn't: the three policy sections
/// and a `Tracks #<n>` link to the tracking issue.
pub fn missing_policy(body: &str) -> Vec<&'static str> {
    let heads: Vec<String> = sections(body).into_iter().map(|(h, _)| h).collect();
    let has = |f: fn(&str) -> bool| heads.iter().any(|h| f(h));
    let mut out = Vec::new();
    if !has(is_default_on) {
        out.push("a \"Default-on or opt-in\" section");
    }
    if !has(is_network_trust) {
        out.push("a \"Network trust\" section");
    }
    if !has(is_c_deps) {
        out.push("a \"C dependencies\" section");
    }
    let tracks = Regex::new(r"(?i)\btracks:?\s+(#[0-9]+|https://github\.com/\S+/issues/[0-9]+)")
        .is_ok_and(|re| re.is_match(body));
    if !tracks {
        out.push("a `Tracks #<n>` reference to the chain's tracking issue");
    }
    out
}

/// Whether the body's "C dependencies" section says the crate is C-free.
pub fn claims_c_free(body: &str) -> bool {
    let Ok(re) = Regex::new(r"(?i)\bc[- ]free\b|\bno c (dependenc|deps?\b)") else {
        return false;
    };
    sections(body)
        .iter()
        .any(|(h, text)| is_c_deps(h) && (re.is_match(text) || re.is_match(h)))
}

/// A slot this PR claims that another open PR also claims, under another name
/// (or the same name under another number).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotClash {
    pub ours: (String, u32),
    pub pr: u64,
    pub theirs: (String, u32),
}

/// Compares the slots this PR adds over `base` with the slots other open PRs add.
pub fn slot_clashes(
    base: &[(String, u32)],
    ours: &[(String, u32)],
    others: &[(u64, Slots)],
) -> Vec<SlotClash> {
    let new = |slots: &[(String, u32)]| -> Vec<(String, u32)> {
        slots
            .iter()
            .filter(|s| !base.contains(s))
            .cloned()
            .collect()
    };
    let mine = new(ours);
    let mut out = Vec::new();
    for (pr, slots) in others {
        for theirs in new(slots) {
            for o in &mine {
                let same_num = o.1 == theirs.1 && o.0 != theirs.0;
                let same_name = o.0 == theirs.0 && o.1 != theirs.1;
                if same_num || same_name {
                    out.push(SlotClash {
                        ours: o.clone(),
                        pr: *pr,
                        theirs: theirs.clone(),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn name_status_parses_adds_and_renames() {
        let c = parse_name_status("A\tsrc/a.rs\nM\tREADME.md\nR087\told.rs\tnew.rs\n\n");
        assert_eq!(c.len(), 3);
        assert_eq!(
            c[0],
            Change {
                status: 'A',
                path: "src/a.rs".into()
            }
        );
        assert_eq!(
            c[2],
            Change {
                status: 'R',
                path: "new.rs".into()
            }
        );
    }

    #[test]
    fn changed_chains_from_paths_labels_and_proto() {
        let changes = parse_name_status(
            "M\tsrc/chain_parsers/visualsign-near/src/lib.rs\nM\tsrc/chain_parsers/visualsign-unspecified/src/lib.rs\nM\tsrc/chain_parsers/README.md\nM\tdocs/x.md\n",
        );
        let diff = "--- a/proto/parser/parser.proto\n+++ b/proto/parser/parser.proto\n   CHAIN_NEAR = 6;\n+  CHAIN_COSMOS = 8;\n-  reserved 8 to 998;\n+  reserved 9 to 998;\n";
        let got = changed_chains(
            &changes,
            &s(&["chain:sui", "bug", "chain:Bad Name"]),
            diff,
            &s(&["unspecified"]),
        );
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            ["cosmos", "near", "sui"]
        );
    }

    #[test]
    fn proto_moved_line_is_not_new() {
        let diff = "-  CHAIN_NEAR = 6;\n+  CHAIN_NEAR = 6; // moved\n+  CHAIN_FOO = 9;\n";
        assert_eq!(
            proto_added_chains(diff).into_iter().collect::<Vec<_>>(),
            ["FOO"]
        );
    }

    #[test]
    fn new_crate_needs_an_added_manifest() {
        let changes = parse_name_status(
            "A\tsrc/chain_parsers/visualsign-foo/Cargo.toml\nA\tsrc/chain_parsers/visualsign-near/src/new.rs\nM\tsrc/chain_parsers/visualsign-bar/Cargo.toml\n",
        );
        assert_eq!(
            new_chain_crates(&changes).into_iter().collect::<Vec<_>>(),
            ["foo"]
        );
    }

    #[test]
    fn known_gaps_additions() {
        let base = parse_known_gaps("[near]\nreadme = \"r\"\n").unwrap();
        let same = parse_known_gaps("[near]\nreadme = \"reworded\"\n").unwrap();
        assert!(added_gaps(&base, &same).is_empty());
        let shrunk = parse_known_gaps("").unwrap();
        assert!(added_gaps(&base, &shrunk).is_empty());
        let grown = parse_known_gaps(
            "[near]\nreadme = \"r\"\ncodeowners = \"c\"\n[sui]\nlabeler = \"l\"\n",
        )
        .unwrap();
        assert_eq!(
            added_gaps(&base, &grown),
            [
                ("near".to_string(), "codeowners".to_string()),
                ("sui".to_string(), "labeler".to_string())
            ]
        );
        assert!(parse_known_gaps("[near]\nreadme = 3\n").is_err());
    }

    #[test]
    fn new_chains_get_only_repo_wide_gaps() {
        let known = parse_known_gaps("[near]\nreadme = \"r\"\noss = \"o\"\n").unwrap();
        let near = allowed_gaps("near", false, &known, &["oss"]);
        assert_eq!(near.keys().collect::<Vec<_>>(), ["oss", "readme"]);
        let foo = allowed_gaps("foo", true, &known, &["oss", "other"]);
        assert_eq!(
            foo.into_iter().collect::<Vec<_>>(),
            [("oss".to_string(), "repo-wide: o".to_string())]
        );
        assert!(allowed_gaps("foo", false, &known, &["oss"]).is_empty());
    }

    const GOOD_BODY: &str = "## Summary\n\nAdds Foo.\n\n## Default-on or opt-in\n\nOpt-in.\n\n### NETWORK TRUST\n\nchain id is signed\n\n**C dependencies:**\n\nC-free: `cargo tree` prints nothing.\n\nTracks #123\n";

    #[test]
    fn policy_sections_any_level_and_case() {
        assert!(missing_policy(GOOD_BODY).is_empty());
        let crlf = GOOD_BODY.replace('\n', "\r\n");
        assert!(missing_policy(&crlf).is_empty());
        let url = GOOD_BODY.replace(
            "Tracks #123",
            "tracks: https://github.com/anchorageoss/visualsign-parser/issues/7",
        );
        assert!(missing_policy(&url).is_empty());
    }

    #[test]
    fn policy_sections_reported_when_absent() {
        assert_eq!(missing_policy("").len(), 4);
        let no_trust = GOOD_BODY.replace("### NETWORK TRUST", "Network trust is fine");
        assert_eq!(missing_policy(&no_trust), ["a \"Network trust\" section"]);
        let no_tracks = GOOD_BODY.replace("Tracks #123", "Related to #123");
        assert_eq!(missing_policy(&no_tracks).len(), 1);
        // A mention in prose is not a heading.
        let prose =
            "We discuss default-on or opt-in, network trust and C dependencies here. Tracks #1";
        assert_eq!(missing_policy(prose).len(), 3);
    }

    #[test]
    fn c_free_claim_only_in_its_section() {
        assert!(claims_c_free(GOOD_BODY));
        assert!(claims_c_free("# C dependencies\nNo C dependencies.\n"));
        assert!(!claims_c_free(
            "# C dependencies\nsecp256k1-sys via bitcoin, no pure-Rust option.\n"
        ));
        assert!(!claims_c_free("# Summary\nThis crate is C-free.\n"));
    }

    const PROTO: &str = "syntax = \"proto3\";\nenum Chain {\n  CHAIN_UNSPECIFIED = 0;\n  CHAIN_SOLANA = 1;\n  CHAIN_NEAR = 6;\n\n  reserved 7 to 998;\n}\nenum Other {\n  CHAIN_FAKE = 3;\n}\n";

    #[test]
    fn proto_slots_read_only_the_chain_enum() {
        let slots = proto_chain_slots(PROTO);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[2], ("NEAR".to_string(), 6));
    }

    #[test]
    fn slot_clashes_between_open_prs() {
        let base = proto_chain_slots(PROTO);
        let with = |extra: &str| {
            proto_chain_slots(&PROTO.replace("\n\n  reserved", &format!("\n{extra}\n  reserved")))
        };
        let ours = with("  CHAIN_COSMOS = 7;");
        let zcash = with("  CHAIN_ZCASH = 7;");
        let cosmos_elsewhere = with("  CHAIN_COSMOS = 8;");
        let unrelated = with("  CHAIN_FOO = 9;");
        let same = with("  CHAIN_COSMOS = 7;");
        let got = slot_clashes(
            &base,
            &ours,
            &[
                (578, zcash),
                (600, cosmos_elsewhere),
                (601, unrelated),
                (602, same),
            ],
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].pr, 578);
        assert_eq!(got[0].theirs, ("ZCASH".to_string(), 7));
        assert_eq!(got[1].pr, 600);
        assert!(slot_clashes(&base, &base, &[(578, with("  CHAIN_ZCASH = 7;"))]).is_empty());
    }
}
