//! Which chains a release ships, read from the repo at a git ref.
//!
//! A chain is *enabled* when its feature is in `parser_app`'s `default` set and in the
//! `parser_app` image's `CHAIN_FEATURES`; those two decide what the attested binary parses.

use std::collections::BTreeSet;

use serde::Serialize;

/// Raw chain surface at one ref.
#[derive(Debug, Default, Clone)]
pub struct ChainSurface {
    /// `visualsign-<chain>` crate dirs, without the prefix.
    pub crates: BTreeSet<String>,
    /// `parser_app` `[features] default`.
    pub app_default: BTreeSet<String>,
    /// `ARG CHAIN_FEATURES` in `images/parser_app/Containerfile`.
    pub image_features: BTreeSet<String>,
    /// `CHAIN_<X>` values in the proto's `Chain` enum, lowercased.
    pub proto: BTreeSet<String>,
}

/// One chain as the announcement shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainRow {
    pub name: String,
    pub display: String,
    /// The crate did not exist at the previous release.
    pub new: bool,
    pub app_default: bool,
    pub image: bool,
    pub proto: bool,
}

impl ChainRow {
    pub fn enabled(&self) -> bool {
        self.app_default && self.image
    }
}

/// Chains that exist as crates but are not a parser you'd announce.
const NOT_ANNOUNCED: &[&str] = &["unspecified"];

pub fn rows(at: &ChainSurface, previous: &ChainSurface) -> Vec<ChainRow> {
    at.crates
        .iter()
        .filter(|c| !NOT_ANNOUNCED.contains(&c.as_str()))
        .map(|name| ChainRow {
            name: name.clone(),
            display: display_name(name),
            new: !previous.crates.contains(name),
            app_default: at.app_default.contains(name),
            image: at.image_features.contains(name),
            proto: at.proto.contains(name),
        })
        .collect()
}

pub fn display_name(chain: &str) -> String {
    match chain {
        "near" => "NEAR".to_string(),
        "evm" => "EVM".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

/// Crate dir names under `src/chain_parsers/` -> chain names.
pub fn crates_from_dir_listing(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .filter_map(|l| l.trim().trim_end_matches('/').strip_prefix("visualsign-"))
        .map(str::to_string)
        .collect()
}

/// The `default = [...]` list in a Cargo.toml's `[features]` table, single- or multi-line.
pub fn parse_default_features(cargo_toml: &str) -> BTreeSet<String> {
    let mut in_features = false;
    let mut collecting = false;
    let mut buf = String::new();
    for line in cargo_toml.lines() {
        // Feature names never contain '#', so this only drops comments.
        let trimmed = line.split('#').next().unwrap_or_default().trim();
        if !collecting && trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if !in_features {
            continue;
        }
        if !collecting {
            let Some(rest) = trimmed.strip_prefix("default") else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix('=') else {
                continue;
            };
            collecting = true;
            buf.push_str(rest);
        } else {
            buf.push_str(trimmed);
        }
        if buf.contains(']') {
            break;
        }
    }
    quoted_strings(&buf)
}

/// `ARG CHAIN_FEATURES="a b c"` in a Containerfile.
pub fn parse_chain_features(containerfile: &str) -> BTreeSet<String> {
    containerfile
        .lines()
        .find_map(|l| l.trim().strip_prefix("ARG CHAIN_FEATURES="))
        .map(|v| {
            v.trim_matches('"')
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `CHAIN_<X> = n;` entries of `enum Chain`, lowercased, without UNSPECIFIED/CUSTOM.
pub fn parse_proto_chains(proto: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_enum = false;
    for line in proto.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("enum Chain ") || trimmed == "enum Chain{" {
            in_enum = true;
            continue;
        }
        if !in_enum {
            continue;
        }
        if trimmed.starts_with('}') {
            break;
        }
        let Some(rest) = trimmed.strip_prefix("CHAIN_") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>()
            .to_ascii_lowercase();
        if name != "unspecified" && name != "custom" && !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

fn quoted_strings(s: &str) -> BTreeSet<String> {
    s.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_features_single_line_with_comments_around() {
        let toml = r#"
[dependencies]
default = ["not-this"]

[features]
# diagnostics is intentionally NOT in `default`
default = ["bitcoin", "ethereum", "near"] # trailing "comment"
bitcoin = ["dep:visualsign-bitcoin"]
"#;
        assert_eq!(
            parse_default_features(toml),
            set(&["bitcoin", "ethereum", "near"])
        );
    }

    #[test]
    fn default_features_multi_line() {
        let toml = "[features]\ndefault = [\n  \"solana\",\n  # \"commented-out\",\n  \"sui\",\n]\nsolana = []\n";
        assert_eq!(parse_default_features(toml), set(&["solana", "sui"]));
    }

    #[test]
    fn default_features_absent_is_empty() {
        assert!(parse_default_features("[package]\nname = \"x\"\n").is_empty());
    }

    #[test]
    fn chain_features_from_containerfile() {
        let cf = "FROM x\n# --build-arg CHAIN_FEATURES=\"<subset>\"\nARG CHAIN_FEATURES=\"bitcoin ethereum near\"\nENV CHAIN_FEATURES=$CHAIN_FEATURES\n";
        assert_eq!(
            parse_chain_features(cf),
            set(&["bitcoin", "ethereum", "near"])
        );
    }

    #[test]
    fn proto_chain_enum_only() {
        let proto = r#"
enum Other {
  CHAIN_NOT_THIS = 0;
}
enum Chain {
  CHAIN_UNSPECIFIED = 0;
  CHAIN_BITCOIN = 1;
  CHAIN_NEAR = 6; // comment
  CHAIN_CUSTOM = 999;
}
message M { Chain chain = 1; }
"#;
        assert_eq!(parse_proto_chains(proto), set(&["bitcoin", "near"]));
    }

    #[test]
    fn rows_mark_new_enabled_and_hide_unspecified() {
        let at = ChainSurface {
            crates: set(&["bitcoin", "cosmos", "near", "unspecified"]),
            app_default: set(&["bitcoin", "near", "unspecified"]),
            image_features: set(&["bitcoin", "near", "unspecified"]),
            proto: set(&["bitcoin", "near"]),
        };
        let previous = ChainSurface {
            crates: set(&["near", "unspecified"]),
            ..Default::default()
        };
        let rows = rows(&at, &previous);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["bitcoin", "cosmos", "near"]);
        assert!(rows[0].new && rows[0].enabled() && rows[0].proto);
        assert!(rows[1].new && !rows[1].enabled() && !rows[1].proto);
        assert!(!rows[2].new && rows[2].enabled());
        assert_eq!(rows[2].display, "NEAR");
    }

    #[test]
    fn crate_listing_ignores_other_entries() {
        let listing = "visualsign-bitcoin\nvisualsign-near/\nREADME.md\n";
        assert_eq!(crates_from_dir_listing(listing), set(&["bitcoin", "near"]));
    }
}
