//! Wallet-integration break detection, shared by the PR check and `tvc-deploy`.
//!
//! - [`rules`]: the data-driven list of changes that break wallet integrators,
//!   matched against a git diff ([`detect_refs`]).
//! - [`body`]: the `## Wallet integration` section a labeled PR must carry.
//! - [`ack`]: the exact-set acknowledgement `tvc-deploy deploy` requires.

use std::path::Path;
use std::process::Command;

pub mod ack;
pub mod body;
pub mod diff;
pub mod rules;

/// The repo label that marks a PR as wallet-breaking.
pub const LABEL: &str = "breaking-wallet-integration";

/// Findings for everything `head` changes since its merge base with `base`.
pub fn detect_refs(repo: &Path, base: &str, head: &str) -> Result<Vec<rules::Finding>, String> {
    let merge_base = git(repo, &["merge-base", base, head])?;
    let merge_base = merge_base.trim();
    let raw = git(
        repo,
        &[
            "diff",
            "--unified=0",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            merge_base,
            head,
        ],
    )?;
    let files = diff::parse(&raw);
    let test_start = |path: &str, side: diff::Side| {
        let rev = match side {
            diff::Side::Base => merge_base,
            diff::Side::Head => head,
        };
        git(repo, &["show", &format!("{rev}:{path}")])
            .ok()
            .and_then(|src| rules::find_test_start(&src))
    };
    Ok(rules::detect(&files, rules::RULES, &test_start))
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("running git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("git output is not UTF-8: {e}"))
}
