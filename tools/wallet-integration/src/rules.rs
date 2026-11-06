//! The wallet-breaking change rules, and matching them against a diff.
//!
//! Every rule is one entry in [`RULES`]. To cover a new kind of change, add an
//! entry (and a case to `tests::each_rule_fires_on_its_example`); nothing else
//! needs to know about it.

use crate::diff::{ChangedLine, FileDiff, Side};

#[derive(Debug)]
pub struct Rule {
    /// Short stable name, printed with each finding.
    pub id: &'static str,
    /// Why a match breaks integrators, printed with each finding.
    pub reason: &'static str,
    /// Repo-relative globs. `*` matches within one path segment, `**` matches
    /// any number of segments.
    pub paths: &'static [&'static str],
    /// A changed line must contain one of these. Empty: any change to a
    /// matching file fires, including a binary one.
    pub needles: &'static [&'static str],
    /// Ignore blank lines, `//` comment lines and the file's `mod tests` block,
    /// so doc and test edits next to a wire type don't fire.
    pub code_only: bool,
}

pub const RULES: &[Rule] = &[
    Rule {
        id: "proto",
        reason: "protobuf API changed (parser.proto, ChainMetadata messages): clients that build \
                 requests or verify responses must move to the new schema",
        paths: &["proto/**"],
        needles: &[],
        code_only: false,
    },
    Rule {
        id: "generated-proto",
        reason: "generated protobuf types or their codegen config changed: ChainMetadata's Borsh \
                 encoding (hashed into metadata_digest) and the signed ParsedTransactionPayload \
                 derive from them",
        paths: &["src/generated/src/**", "src/codegen/src/**"],
        needles: &[],
        code_only: false,
    },
    Rule {
        id: "schema-version",
        reason: "an intermediate-output schema version constant changed: decoders gate on it and \
                 must accept the new version first",
        paths: &["src/chain_parsers/*/src/**"],
        needles: &["_INTERMEDIATE_SCHEMA_VERSION: u16"],
        code_only: true,
    },
    Rule {
        id: "intermediate-types",
        reason: "Borsh-encoded intermediate-output types changed: mirrored decoders must match \
                 the new encoding byte for byte",
        paths: &[
            "src/chain_parsers/*/src/intermediate.rs",
            "src/chain_parsers/*/src/intermediate/**",
        ],
        needles: &[],
        code_only: true,
    },
    Rule {
        id: "digest-computation",
        reason: "code that builds the signed payload or the bytes hashed into metadata_digest \
                 changed: clients that recompute either digest must match",
        paths: &[
            "src/parser/app/src/routes/parse.rs",
            "src/parser/app/src/payment_verify.rs",
            "src/host_primitives/src/**",
        ],
        needles: &[
            "metadata_digest",
            "chain_metadata_bytes",
            "signing_digest_bytes",
            "intermediate_output",
        ],
        code_only: true,
    },
    Rule {
        id: "golden-fixtures",
        reason: "committed intermediate-output or digest golden bytes changed: the encoding they \
                 pin changed",
        paths: &["src/**"],
        needles: &["_GOLDEN"],
        code_only: false,
    },
    Rule {
        id: "intermediate-oracle",
        reason: "the intermediate-output oracle fixtures changed: they pin the Borsh bytes \
                 decoders are tested against",
        paths: &["src/chain_parsers/visualsign-sui/tests/oracle/**"],
        needles: &[],
        code_only: false,
    },
];

/// One rule firing on one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub reason: &'static str,
    pub path: String,
    /// First matching head-side line (else base-side), or None for a binary file.
    pub first: Option<(Side, u32)>,
    /// Matching changed lines in this file (0 for a binary file).
    pub lines: usize,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Finding {
            rule, reason, path, ..
        } = self;
        match self.first {
            Some((side, n)) => {
                let base = if side == Side::Base { " (base)" } else { "" };
                write!(f, "[{rule}] {path}:{n}{base}")?;
                if self.lines > 1 {
                    write!(f, " (+{} more line(s))", self.lines - 1)?;
                }
            }
            None => write!(f, "[{rule}] {path} (binary)")?,
        }
        write!(f, ": {reason}")
    }
}

/// Where a file's `mod tests` block starts on a given side, if it has one.
/// Lines at or after it are test code for `code_only` rules.
pub type TestStart<'a> = &'a dyn Fn(&str, Side) -> Option<u32>;

/// Run `rules` over `files`, one finding per (rule, file).
pub fn detect(files: &[FileDiff], rules: &'static [Rule], test_start: TestStart) -> Vec<Finding> {
    let mut findings = Vec::new();
    for rule in rules {
        for file in files {
            if !rule.paths.iter().any(|g| glob_match(g, &file.path)) {
                continue;
            }
            if file.binary {
                if rule.needles.is_empty() {
                    findings.push(Finding {
                        rule: rule.id,
                        reason: rule.reason,
                        path: file.path.clone(),
                        first: None,
                        lines: 0,
                    });
                }
                continue;
            }
            let hits: Vec<&ChangedLine> = file
                .lines
                .iter()
                .filter(|l| line_fires(rule, &file.path, l, test_start))
                .collect();
            // Point at a head-side line when there is one: that's the line a
            // reviewer sees in the PR.
            let first = hits
                .iter()
                .find(|l| l.side == Side::Head)
                .or_else(|| hits.first());
            if let Some(first) = first {
                findings.push(Finding {
                    rule: rule.id,
                    reason: rule.reason,
                    path: file.path.clone(),
                    first: Some((first.side, first.number)),
                    lines: hits.len(),
                });
            }
        }
    }
    findings
}

fn line_fires(rule: &Rule, path: &str, line: &ChangedLine, test_start: TestStart) -> bool {
    if rule.code_only {
        let t = line.text.trim();
        if t.is_empty() || t.starts_with("//") {
            return false;
        }
        if test_start(path, line.side).is_some_and(|start| line.number >= start) {
            return false;
        }
    }
    rule.needles.is_empty() || rule.needles.iter().any(|n| line.text.contains(n))
}

/// The line number of the first top-level `mod tests` in `source`, if any.
pub fn find_test_start(source: &str) -> Option<u32> {
    source
        .lines()
        .position(|l| l.starts_with("mod tests"))
        .and_then(|i| u32::try_from(i + 1).ok())
}

/// Segment-wise glob: `*` within a segment, `**` for any number of segments.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    match_segments(&pat, &segs)
}

fn match_segments(pat: &[&str], segs: &[&str]) -> bool {
    match pat.split_first() {
        None => segs.is_empty(),
        Some((&"**", rest)) => (0..=segs.len()).any(|i| match_segments(rest, &segs[i..])),
        Some((p, rest)) => match segs.split_first() {
            Some((s, srest)) => match_segment(p, s) && match_segments(rest, srest),
            None => false,
        },
    }
}

fn match_segment(pat: &str, seg: &str) -> bool {
    match pat.split_once('*') {
        None => pat == seg,
        Some((prefix, rest)) => {
            let Some(tail) = seg.strip_prefix(prefix) else {
                return false;
            };
            if rest.is_empty() {
                return true;
            }
            (0..=tail.len())
                .filter(|&i| tail.is_char_boundary(i))
                .any(|i| match_segment(rest, &tail[i..]))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn file(path: &str, lines: &[(Side, u32, &str)]) -> FileDiff {
        FileDiff {
            path: path.to_owned(),
            binary: false,
            lines: lines
                .iter()
                .map(|&(side, number, text)| ChangedLine {
                    side,
                    number,
                    text: text.to_owned(),
                })
                .collect(),
        }
    }

    fn no_tests(_: &str, _: Side) -> Option<u32> {
        None
    }

    fn rule_ids(files: &[FileDiff]) -> Vec<&'static str> {
        detect(files, RULES, &no_tests)
            .into_iter()
            .map(|f| f.rule)
            .collect()
    }

    #[test]
    fn glob_semantics() {
        assert!(glob_match("proto/**", "proto/parser/parser.proto"));
        assert!(glob_match("proto/**", "proto/x"));
        assert!(!glob_match("proto/**", "protox/x"));
        assert!(glob_match(
            "src/chain_parsers/*/src/intermediate.rs",
            "src/chain_parsers/visualsign-sui/src/intermediate.rs"
        ));
        assert!(!glob_match(
            "src/chain_parsers/*/src/intermediate.rs",
            "src/chain_parsers/a/b/src/intermediate.rs"
        ));
        assert!(glob_match("src/**/*.rs", "src/a.rs"));
        assert!(glob_match("src/**/*.rs", "src/a/b/c.rs"));
        assert!(!glob_match("src/**/*.rs", "src/a/b/c.rsx"));
        assert!(glob_match("a/*-x-*/b", "a/1-x-2/b"));
    }

    #[test]
    fn each_rule_fires_on_its_example() {
        let head = |text| [(Side::Head, 10, text)];
        let cases: &[(&str, FileDiff)] = &[
            ("proto", file("proto/parser/parser.proto", &head("// doc"))),
            (
                "generated-proto",
                file("src/codegen/src/main.rs", &head("x")),
            ),
            (
                "schema-version",
                file(
                    "src/chain_parsers/visualsign-tron/src/lib.rs",
                    &head("pub const TRON_INTERMEDIATE_SCHEMA_VERSION: u16 = 2;"),
                ),
            ),
            (
                "intermediate-types",
                file(
                    "src/chain_parsers/visualsign-solana/src/intermediate/native_instruction_data.rs",
                    &head("    pub fee: u64,"),
                ),
            ),
            (
                "digest-computation",
                file(
                    "src/parser/app/src/routes/parse.rs",
                    &head("    metadata_digest: hash(x),"),
                ),
            ),
            (
                "golden-fixtures",
                file(
                    "src/chain_parsers/visualsign-sui/src/x.rs",
                    &head("    const PLAIN_GOLDEN: &str = \"00\";"),
                ),
            ),
            (
                "intermediate-oracle",
                file(
                    "src/chain_parsers/visualsign-sui/tests/oracle/decode_fixtures.json",
                    &head("{}"),
                ),
            ),
        ];
        for (id, f) in cases {
            assert_eq!(
                rule_ids(std::slice::from_ref(f)),
                vec![*id],
                "case for {id}"
            );
        }
        // Every rule has a case, so a new rule can't land untested.
        let mut covered: Vec<&str> = cases.iter().map(|(id, _)| *id).collect();
        let mut all: Vec<&str> = RULES.iter().map(|r| r.id).collect();
        covered.sort_unstable();
        all.sort_unstable();
        assert_eq!(covered, all);
    }

    #[test]
    fn unrelated_changes_do_not_fire() {
        let files = [
            file("docs/api-reference.mdx", &[(Side::Head, 1, "Max Fee")]),
            file(
                "src/chain_parsers/visualsign-solana/src/core/priority_fee.rs",
                &[(
                    Side::Head,
                    5,
                    "pub const DEFAULT_MAX_FEE_LAMPORTS: u64 = 1;",
                )],
            ),
            file(
                "src/parser/app/src/routes/parse.rs",
                &[(Side::Head, 5, "    let x = 1;")],
            ),
        ];
        assert!(rule_ids(&files).is_empty());
    }

    #[test]
    fn code_only_skips_comments_blanks_and_tests() {
        let path = "src/chain_parsers/visualsign-sui/src/intermediate.rs";
        let f = file(
            path,
            &[
                (Side::Head, 3, "//! docs"),
                (Side::Head, 4, "    /// field doc"),
                (Side::Head, 5, "   "),
                (
                    Side::Head,
                    600,
                    "    let x = SuiIntermediateOutput { a: 1 };",
                ),
                (Side::Base, 590, "    assert!(old);"),
            ],
        );
        let starts = |p: &str, side: Side| {
            assert_eq!(p, path);
            Some(if side == Side::Head { 500 } else { 480 })
        };
        assert!(detect(std::slice::from_ref(&f), RULES, &starts).is_empty());

        // The same body line outside the test block does fire.
        let f = file(path, &[(Side::Head, 100, "    pub a: u8,")]);
        let found = detect(std::slice::from_ref(&f), RULES, &starts);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].first, Some((Side::Head, 100)));
    }

    #[test]
    fn binary_file_fires_only_path_rules() {
        let f = FileDiff {
            path: "src/generated/src/generated/descriptor.bin".to_owned(),
            binary: true,
            lines: vec![],
        };
        let found = detect(&[f], RULES, &no_tests);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule, "generated-proto");
        assert_eq!(found[0].first, None);
        assert!(found[0].to_string().contains("(binary)"));
    }

    #[test]
    fn one_finding_per_rule_and_file_with_count() {
        let f = file(
            "proto/parser/parser.proto",
            &[
                (Side::Base, 7, "a"),
                (Side::Base, 8, "b"),
                (Side::Base, 9, "c"),
            ],
        );
        let found = detect(&[f], RULES, &no_tests);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lines, 3);
        assert_eq!(
            found[0].to_string().split(':').next(),
            Some("[proto] proto/parser/parser.proto")
        );
        assert!(found[0].to_string().contains(":7 (base) (+2 more line(s))"));
    }

    #[test]
    fn find_test_start_takes_top_level_mod_tests() {
        let src = "fn a() {}\n#[cfg(test)]\n    mod tests_inner {}\nmod tests {\n}\n";
        assert_eq!(find_test_start(src), Some(4));
        assert_eq!(find_test_start("fn a() {}\n"), None);
    }
}
