//! Each rule kind against the fixture repos in `tests/fixtures/`:
//! `repo-ok` wires chain `good` completely; `repo-gaps` wires chain `gappy`
//! with one gap of each kind.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_wiring::{Report, Rules, Status, check};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn item<'a>(r: &'a Report, id: &str) -> &'a chain_wiring::Item {
    r.items
        .iter()
        .find(|i| i.id == id)
        .expect("no item with that id")
}

#[test]
fn complete_chain_passes_every_check() {
    let r = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(r.missing(), 0, "{}", r.render_text());
    assert_eq!(r.total(), 34);
    assert_eq!(r.summary(), "34/34 touch points present");
}

#[test]
fn gaps_are_reported_once_per_kind() {
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    let missing: Vec<_> = r.missing_ids().into_iter().collect();
    assert_eq!(
        missing,
        [
            "changelog",
            "cli-fixture",
            "default-set-oss-scanner",
            "docs-page",
            "parser-app-registry-test",
        ]
    );
    assert_eq!(r.summary(), "29/34 touch points present, 5 missing");
}

#[test]
fn file_exists() {
    let ok = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(
        item(&ok, "crate-dir").text,
        "src/chain_parsers/visualsign-good exists"
    );
    let ghost = check(&fixture("repo-ok"), "ghost", None).unwrap();
    let i = item(&ghost, "crate-dir");
    assert_eq!(
        (i.status, i.text.as_str()),
        (Status::Missing, "src/chain_parsers/visualsign-ghost exists")
    );
}

#[test]
fn file_contains_missing_line_and_missing_file() {
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    let line = item(&r, "parser-app-registry-test");
    assert_eq!(
        line.text,
        "parser_app registry test (src/parser/app/src/registry.rs)"
    );
    let file = item(&r, "docs-page");
    assert_eq!(file.text, "docs/chains/gappy.mdx (docs/chains/gappy.mdx)");
    // The variant is a placeholder too: a wrong variant breaks the variant checks only.
    let wrong = check(&fixture("repo-ok"), "good", Some("Goodish")).unwrap();
    assert!(wrong.missing_ids().contains("registry-variant"));
    assert!(!wrong.missing_ids().contains("parser-app-feature"));
}

#[test]
fn glob_exists() {
    let ok = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(
        item(&ok, "cli-fixture").text,
        "parser_cli fixture good-*.input"
    );
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    assert_eq!(
        item(&r, "cli-fixture").text,
        "parser_cli fixture (src/parser/cli/tests/fixtures/gappy-*.input)"
    );
}

#[test]
fn any_file_contains_is_case_insensitive() {
    // repo-ok's changelog says "GOOD".
    let ok = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(item(&ok, "changelog").status, Status::Ok);
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    assert_eq!(
        item(&r, "changelog").text,
        "changelog entry (docs/changelog/_drafts/<date>-gappy-*.md)"
    );
}

#[test]
fn default_set_equality_ignores_non_chain_features() {
    // repo-ok's defaults carry `diagnostics` and `vsock`, which aren't chains.
    let ok = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(
        item(&ok, "default-set-http-server").text,
        "http-server default matches parser_app default"
    );
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    assert_eq!(
        item(&r, "default-set-oss-scanner").text,
        ".oss-scanner/Dockerfile CHAIN_FEATURES differs from parser_app default [gappy ] vs [ ]"
    );
}

#[test]
fn info_only_lines_are_never_counted() {
    let ok = check(&fixture("repo-ok"), "good", None).unwrap();
    assert_eq!(
        item(&ok, "default-on").text,
        "good is DEFAULT-ON in parser_app (state this in the PR)"
    );
    assert_eq!(
        item(&ok, "apple-no-c").text,
        "visualsign-good is in APPLE_NO_C_CRATES (C-free, checked on iOS)"
    );
    let ghost = check(&fixture("repo-ok"), "ghost", None).unwrap();
    assert_eq!(
        item(&ghost, "default-on").text,
        "ghost is OPT-IN in parser_app (state this in the PR)"
    );
    assert_eq!(
        item(&ghost, "apple-no-c").text,
        "visualsign-ghost is NOT in APPLE_NO_C_CRATES (state its C deps in the PR)"
    );
    let gaps = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    let apple = item(&gaps, "apple-no-c");
    assert_eq!(apple.status, Status::Info);
    assert!(apple.text.starts_with("no APPLE_NO_C_CRATES"));
}

#[test]
fn discovery_skips_unspecified() {
    let rules = Rules::builtin().unwrap();
    assert_eq!(
        rules.discover_chains(&fixture("repo-ok")).unwrap(),
        ["good"]
    );
    assert_eq!(
        rules.discover_chains(&fixture("repo-gaps")).unwrap(),
        ["gappy"]
    );
}

/// Compares with `tests/snapshots/<name>`; `UPDATE_SNAPSHOTS=1` rewrites it.
fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        actual, expected,
        "snapshot {name} differs; rerun with UPDATE_SNAPSHOTS=1 and review the diff"
    );
}

#[test]
fn snapshot_issue_block() {
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    snapshot("gappy-issue-block.md", &r.issue_block());
}

#[test]
fn snapshot_markdown_and_text() {
    let r = check(&fixture("repo-gaps"), "gappy", None).unwrap();
    snapshot("gappy-check.md", &r.render_markdown());
    snapshot("gappy-check.txt", &r.render_text());
}
