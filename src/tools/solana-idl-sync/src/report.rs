//! The `report` subcommand: assemble `report.md` from the section files
//! the other subcommands wrote, sized to fit a PR body, plus the full
//! untruncated report for the workflow artifact.
//!
//! Input files in `--dir`, all produced by this tool or by the workflow's
//! own shell (never by anything untrusted):
//!
//! * `context-*.md` -- standalone sections (e.g. the nightly
//!   `cargo test -p visualsign-solana` result), sorted by file name;
//! * `compare-<program_id>.md` -- the IDL compare summary of a changed
//!   program (written by `apply`);
//! * `replay-<program_id>.md` -- the replay section of a changed program
//!   (written by `replay`).
//!
//! Output: `report.md` (truncated to `--max-bytes` for the PR body and
//! the step summary) and `report.full.md` (never truncated; the workflow
//! uploads it with the artifact).

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const INTRO: &str = "\
# Solana preset IDL sync

Nightly sync of the preset IDLs compiled into `visualsign-solana` with the
Anchor IDLs the programs publish on chain. For each program whose on-chain
IDL changed, recent successful mainnet transactions were replayed through
`parser_cli` built before and after the IDL update, so the sections below
show what the change does to real transactions.

";

const CHANGED_PROGRAMS_HEADING: &str = "## Changed programs\n";

const TRUNCATION_NOTE: &str = "\
---

<report truncated to fit the PR body; the untruncated report is attached \
to the workflow run as the `report` artifact.>";

/// Assemble the full (untruncated) report from the section files in
/// `dir`. A missing or empty dir yields just the intro.
pub fn assemble(dir: &Path) -> Result<String> {
    let names = list_dir(dir)?;
    let mut report = String::from(INTRO);

    // Context sections, sorted by file name, right after the intro.
    let mut context_names: Vec<&String> = names
        .iter()
        .filter(|name| name.starts_with("context-") && name.ends_with(".md"))
        .collect();
    context_names.sort();
    for name in context_names {
        report.push_str(&read_section(dir, name)?);
    }

    // One section per changed program: compare summary then replay.
    let program_ids: BTreeSet<String> = names
        .iter()
        .filter_map(|name| {
            ["compare-", "replay-"]
                .iter()
                .find_map(|prefix| section_program_id(name, prefix))
        })
        .collect();
    if !program_ids.is_empty() {
        report.push_str(CHANGED_PROGRAMS_HEADING);
        report.push('\n');
    }
    for program_id in &program_ids {
        for prefix in ["compare-", "replay-"] {
            let name = format!("{prefix}{program_id}.md");
            if names.contains(&name) {
                report.push_str(&read_section(dir, &name)?);
            }
        }
    }
    Ok(report)
}

/// `compare-<program_id>.md` / `replay-<program_id>.md` -> the program id.
/// Program ids are base58, so the first `-`/`.` boundary cannot appear
/// inside one; still, the file name must name a known section prefix.
fn section_program_id(name: &str, prefix: &str) -> Option<String> {
    let rest = name.strip_prefix(prefix)?.strip_suffix(".md")?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

fn list_dir(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(err) => {
            return Err(anyhow::Error::new(err))
                .context(format!("failed to list {}", dir.display()));
        }
    };
    for entry in entries {
        let entry = entry
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to list {}", dir.display()))?;
        if entry.path().is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

fn read_section(dir: &Path, name: &str) -> Result<String> {
    let text =
        fs::read_to_string(dir.join(name)).with_context(|| format!("failed to read {}", name))?;
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("{trimmed}\n\n"))
}

/// Truncate `full` to at most `max_bytes` for the PR body. The cut lands
/// on a line boundary with every fenced code block closed (content inside
/// fences is backtick-neutralized by construction, so fences are only
/// ever this tool's own delimiters), preferring a section header, and
/// the result says it was truncated.
pub fn truncate_for_pr(full: &str, max_bytes: usize) -> String {
    if full.len() <= max_bytes {
        return full.to_string();
    }
    let note = TRUNCATION_NOTE;
    let mut budget = match max_bytes.checked_sub(note.len()) {
        Some(budget) => budget,
        // Degenerate cap: keep at least a prefix of the note itself.
        None => return note.chars().take(max_bytes).collect(),
    };
    if budget > full.len() {
        budget = full.len();
    }
    while budget > 0 && !full.is_char_boundary(budget) {
        budget -= 1;
    }

    // The latest line start (within the byte budget) where every fence is
    // closed. Fences are this tool's own ``` delimiters; section content
    // has its backticks neutralized, so a fence line is unambiguous.
    let mut safe_cut = 0usize;
    let mut in_fence = false;
    let mut offset = 0usize;
    for line in full[..budget].split_inclusive('\n') {
        let without_newline = line.trim_end_matches('\n');
        if without_newline.starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence {
            safe_cut = offset + line.len();
        }
        offset += line.len();
    }

    // Prefer cutting just before a section header (### program sections)
    // when one fits inside the safe prefix.
    let safe_prefix = &full[..safe_cut];
    let cut = safe_prefix
        .rfind("\n### ")
        .map(|position| position + 1)
        .unwrap_or(safe_cut);
    format!("{}{}", &full[..cut], note)
}

/// Assemble and write `report.md` (PR-body sized) and `report.full.md`
/// into `dir`.
pub fn write_reports(dir: &Path, max_bytes: usize) -> Result<()> {
    let full = assemble(dir)?;
    let pr = truncate_for_pr(&full, max_bytes);
    let pr_path = dir.join("report.md");
    fs::write(&pr_path, pr.as_bytes())
        .with_context(|| format!("failed to write {}", pr_path.display()))?;
    let full_path = dir.join("report.full.md");
    fs::write(&full_path, full.as_bytes())
        .with_context(|| format!("failed to write {}", full_path.display()))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn section_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    fn repeat_char(byte: char, count: usize) -> String {
        std::iter::repeat_n(byte, count).collect()
    }

    #[test]
    fn assembles_intro_context_and_program_sections_in_order() {
        let dir = section_dir(&[
            ("context-tests.md", "### context\n\npassed\n"),
            ("compare-BBBBBB.md", "### `BBBBBB`\n\ncompare B\n"),
            ("replay-BBBBBB.md", "replay B\n"),
            ("compare-AAAAAA.md", "### `AAAAAA`\n\ncompare A\n"),
            ("replay-AAAAAA.md", "replay A\n"),
        ]);
        let report = assemble(dir.path()).unwrap();
        let intro_end = report.find("## Changed programs").unwrap();
        assert!(report.starts_with("# Solana preset IDL sync\n"));
        // The context section sits between intro and the program list.
        let context_at = report.find("### context").unwrap();
        assert!(context_at < intro_end);
        // Programs are sorted, compare before replay for each.
        let a = report.find("### `AAAAAA`").unwrap();
        let replay_a = report.find("replay A").unwrap();
        let b = report.find("### `BBBBBB`").unwrap();
        let replay_b = report.find("replay B").unwrap();
        assert!(a < replay_a && replay_a < b && b < replay_b);
        // Sections are separated by blank lines.
        assert!(report.contains("compare A\n\nreplay A\n\n### `BBBBBB`"));
    }

    #[test]
    fn an_empty_dir_is_just_the_intro() {
        let dir = tempfile::tempdir().unwrap();
        let report = assemble(dir.path()).unwrap();
        assert_eq!(report, INTRO);
        // A missing dir is the same.
        assert_eq!(assemble(&dir.path().join("nope")).unwrap(), INTRO);
    }

    #[test]
    fn a_replay_without_a_compare_still_lands_in_the_report() {
        let dir = section_dir(&[("replay-CCCC.md", "replay C\n")]);
        let report = assemble(dir.path()).unwrap();
        assert!(report.contains("replay C"));
    }

    #[test]
    fn truncation_keeps_whole_sections_and_says_so() {
        // One real section plus a filler section that pushes over the cap.
        let filler_body = repeat_char('x', 2000);
        let dir = section_dir(&[
            ("compare-AAAA.md", "### `AAAA`\n\ncompare A\n"),
            ("compare-BBBB.md", &format!("### `BBBB`\n\n{filler_body}\n")),
        ]);
        let full = assemble(dir.path()).unwrap();
        assert!(full.len() > 2000);

        let max = INTRO.len() + 600;
        let truncated = truncate_for_pr(&full, max);
        assert!(truncated.len() <= max, "len {}", truncated.len());
        assert!(truncated.contains(TRUNCATION_NOTE.trim()));
        assert!(
            truncated.contains("### `AAAA`"),
            "the section that fits must stay"
        );
        assert!(
            !truncated.contains("### `BBBB`"),
            "the cut must land before the section that does not fit"
        );
        // The cut keeps fences balanced: no unterminated block remains.
        assert_eq!(truncated.matches("```").count() % 2, 0);
        // A generous cap returns the report unchanged.
        assert_eq!(truncate_for_pr(&full, full.len() + 1), full);
        assert_eq!(truncate_for_pr(&full, full.len()), full);
    }

    #[test]
    fn truncation_never_cuts_inside_a_fence() {
        // A single long section with fenced content larger than the cap:
        // the cut must back up to before the opening fence.
        let fenced_body = repeat_char('z', 3000);
        let dir = section_dir(&[(
            "compare-AAAA.md",
            &format!("### `AAAA`\n\n```\n{fenced_body}\n```\n"),
        )]);
        let full = assemble(dir.path()).unwrap();
        let max = INTRO.len() + 100;
        let truncated = truncate_for_pr(&full, max);
        assert!(truncated.len() <= max, "len {}", truncated.len());
        assert_eq!(truncated.matches("```").count() % 2, 0);
        assert!(!truncated.contains('z'), "no partial fence content");
        assert!(truncated.contains(TRUNCATION_NOTE.trim()));
    }

    #[test]
    fn write_reports_writes_both_variants() {
        let dir = section_dir(&[
            ("context-tests.md", "### context\n\npassed\n"),
            ("compare-AAAA.md", "### `AAAA`\n\ncompare A\n"),
        ]);
        write_reports(dir.path(), 60_000).unwrap();
        let pr = fs::read_to_string(dir.path().join("report.md")).unwrap();
        let full = fs::read_to_string(dir.path().join("report.full.md")).unwrap();
        assert!(pr.contains("compare A"));
        assert_eq!(pr, full, "under the cap, both variants are the full report");
    }

    #[test]
    fn program_ids_parse_out_of_section_names() {
        assert_eq!(
            section_program_id("compare-JUP6.json.md", "compare-").as_deref(),
            Some("JUP6.json")
        );
        assert_eq!(section_program_id("replay-JUP6.json.md", "compare-"), None);
        assert_eq!(section_program_id("compare-.md", "compare-"), None);
        assert_eq!(section_program_id("compare-a/b.md", "compare-"), None);
        assert_eq!(section_program_id("report.md", "compare-"), None);
    }
}
