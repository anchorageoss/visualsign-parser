//! The binary's exit codes and stdin/stdout plumbing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_wiring::{WIRING_END, WIRING_START};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], stdin: &str) -> Output {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repo-gaps");
    let mut child = Command::new(env!("CARGO_BIN_EXE_chain-wiring"))
        .arg("--repo")
        .arg(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn check_exits_1_on_missing() {
    let out = run(&["check", "gappy"], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .ends_with("29/34 touch points present, 5 missing\n")
    );
}

#[test]
fn check_rejects_bad_names() {
    assert_eq!(run(&["check", "Bad Name"], "").status.code(), Some(2));
}

#[test]
fn splice_round_trip_and_refusal() {
    let body = format!("- [x] hand\n{WIRING_START}\nold\n{WIRING_END}\n");
    let out = run(&["splice", "gappy"], &body);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("- [x] hand\n<!-- add-chain:wiring:start -->\n"));
    assert!(text.contains("- [ ] parser_app registry test"));
    assert!(!text.contains("\nold\n"));
    let refused = run(&["splice", "gappy"], "no markers\n");
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        String::from_utf8(refused.stderr)
            .unwrap()
            .contains("not a chain tracking issue")
    );
}

#[test]
fn issue_chain_reads_the_form() {
    let out = run(
        &["issue-chain"],
        "### Chain name\n\ncosmos\n\n### Rust variant\n\nCosmos\n",
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "cosmos Cosmos\n");
    assert_eq!(
        run(&["issue-chain"], "### Chain name\n\n$(evil)\n")
            .status
            .code(),
        Some(2)
    );
}
