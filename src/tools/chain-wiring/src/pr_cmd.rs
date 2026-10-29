//! The impure side of the per-PR check: git, the event JSON, `cargo tree`, `gh`.
//! Decisions live in `chain_wiring::pr`, where they are unit-tested.

use crate::Res;
use chain_wiring::pr::{self, ALLOW_GAP_LABEL};
use chain_wiring::{Kind, Names, Rules, Status, check_with};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const KNOWN_GAPS: &str = "src/tools/chain-wiring/known-gaps.toml";
const PROTO: &str = "proto/parser/parser.proto";
/// First line of the PR summary; the comment job finds its sticky comment by it.
pub const PR_MARKER: &str = "<!-- chain-wiring:pr -->";

#[derive(clap::Args)]
pub struct ChangedArgs {
    /// Base commit to diff against.
    #[arg(long)]
    base: String,
    /// Head commit.
    #[arg(long, default_value = "HEAD")]
    head: String,
    /// PR labels, comma-separated.
    #[arg(long, value_delimiter = ',')]
    labels: Vec<String>,
}

#[derive(clap::Args)]
pub struct PrCheckArgs {
    /// Base commit. Defaults to the event's pull_request.base.sha.
    #[arg(long)]
    base: Option<String>,
    /// Head commit.
    #[arg(long, default_value = "HEAD")]
    head: String,
    /// The pull_request event JSON ($GITHUB_EVENT_PATH).
    #[arg(long)]
    event: PathBuf,
    /// owner/repo for the open-PR proto slot check. Defaults to $GITHUB_REPOSITORY.
    #[arg(long)]
    repo_slug: Option<String>,
    /// Skip the open-PR proto slot check (it needs `gh` and the network).
    #[arg(long)]
    offline: bool,
}

fn git(root: &Path, args: &[&str]) -> Res<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn changes(root: &Path, base: &str, head: &str) -> Res<(Vec<pr::Change>, String)> {
    let names = git(root, &["diff", "--name-status", "-M", base, head])?;
    let proto = git(root, &["diff", base, head, "--", PROTO])?;
    Ok((pr::parse_name_status(&names), proto))
}

pub fn changed_chains(root: &Path, args: &ChangedArgs) -> Res<ExitCode> {
    let rules = Rules::builtin()?;
    let (changes, proto) = changes(root, &args.base, &args.head)?;
    for c in pr::changed_chains(&changes, &args.labels, &proto, &rules.discovery.skip) {
        println!("{c}");
    }
    Ok(ExitCode::SUCCESS)
}

struct Event {
    body: String,
    labels: Vec<String>,
    number: Option<u64>,
    base: Option<String>,
}

fn read_event(path: &Path) -> Res<Event> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let pr = &v["pull_request"];
    Ok(Event {
        body: pr["body"].as_str().unwrap_or_default().to_string(),
        labels: pr["labels"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        number: pr["number"].as_u64(),
        base: pr["base"]["sha"].as_str().map(str::to_string),
    })
}

/// `cargo tree ... -i cc` for one chain crate on the iOS target. `Ok("")` means C-free.
fn c_deps(root: &Path, chain: &str) -> Res<String> {
    let pkg = format!("visualsign-{chain}");
    let out = Command::new("cargo")
        .current_dir(root.join("src"))
        .args([
            "tree",
            "-q",
            "-p",
            &pkg,
            "-e",
            "normal,build",
            "--target",
            "aarch64-apple-ios",
            "-i",
            "cc",
        ])
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        return Ok(stdout);
    }
    // `-i cc` errors when no package in the whole lockfile is named cc; that's C-free too.
    if stderr.contains("did not match any packages") {
        return Ok(String::new());
    }
    Err(format!("cargo tree -p {pkg}: {}", stderr.trim()).into())
}

/// The first `n` lines, and a count of the rest.
fn head_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.len() <= n {
        return lines.join("\n");
    }
    format!(
        "{}\n... ({} more lines)",
        lines[..n].join("\n"),
        lines.len() - n
    )
}

fn in_apple_no_c(root: &Path, chain: &str) -> bool {
    let Ok(re) = regex::Regex::new(&format!(
        r"^APPLE_NO_C_CRATES *:=.*visualsign-{}\b",
        regex::escape(chain)
    )) else {
        return false;
    };
    std::fs::read_to_string(root.join("src/Makefile"))
        .is_ok_and(|t| t.lines().any(|l| re.is_match(l)))
}

fn gh(args: &[&str]) -> Res<String> {
    let out = Command::new("gh").args(args).output()?;
    if !out.status.success() {
        return Err(format!(
            "gh {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Proto `Chain` slots on the heads of other open PRs that change the proto.
fn open_pr_slots(slug: &str, own: Option<u64>) -> Res<Vec<(u64, pr::Slots)>> {
    let list = gh(&[
        "pr",
        "list",
        "--repo",
        slug,
        "--state",
        "open",
        "--limit",
        "200",
        "--json",
        "number,files",
    ])?;
    let prs: serde_json::Value = serde_json::from_str(&list)?;
    let mut out = Vec::new();
    for p in prs.as_array().into_iter().flatten() {
        let Some(n) = p["number"].as_u64() else {
            continue;
        };
        let touches = p["files"]
            .as_array()
            .is_some_and(|f| f.iter().any(|f| f["path"] == PROTO));
        if Some(n) == own || !touches {
            continue;
        }
        let url = format!("repos/{slug}/contents/{PROTO}?ref=refs/pull/{n}/head");
        let text = gh(&["api", "-H", "Accept: application/vnd.github.raw", &url])?;
        out.push((n, pr::proto_chain_slots(&text)));
    }
    Ok(out)
}

pub fn pr_check(root: &Path, rules: &Rules, args: &PrCheckArgs) -> Res<ExitCode> {
    let event = read_event(&args.event)?;
    let base = args
        .base
        .clone()
        .or(event.base.clone())
        .ok_or("no --base and no pull_request.base.sha in the event")?;
    let (changes, proto_diff) = changes(root, &base, &args.head)?;
    let chains = pr::changed_chains(&changes, &event.labels, &proto_diff, &rules.discovery.skip);
    let new_crates = pr::new_chain_crates(&changes);
    let allow_gap = event.labels.iter().any(|l| l == ALLOW_GAP_LABEL);

    let mut failures: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut body = String::new();

    // known-gaps.toml may only shrink.
    let head_gaps =
        pr::parse_known_gaps(&std::fs::read_to_string(root.join(KNOWN_GAPS)).unwrap_or_default())?;
    match git(root, &["show", &format!("{base}:{KNOWN_GAPS}")]) {
        Ok(text) => {
            let base_gaps = pr::parse_known_gaps(&text)?;
            for (chain, id) in pr::added_gaps(&base_gaps, &head_gaps) {
                let msg =
                    format!("`known-gaps.toml` gains `{chain}.{id}`; the list may only shrink");
                if allow_gap {
                    warnings.push(format!("{msg} (allowed by the `{ALLOW_GAP_LABEL}` label)"));
                } else {
                    failures.push(format!(
                        "{msg}. Fix the wiring, or a maintainer adds the `{ALLOW_GAP_LABEL}` label."
                    ));
                }
            }
        }
        Err(_) => warnings.push(format!(
            "`{KNOWN_GAPS}` doesn't exist on the base; not compared"
        )),
    }

    let names_list: Vec<String> = chains.iter().map(|c| format!("`{c}`")).collect();
    let _ = writeln!(
        body,
        "Chains this PR affects: {}\n",
        if names_list.is_empty() {
            "none".to_string()
        } else {
            names_list.join(", ")
        }
    );

    // Checks that don't depend on the chain (the default-set comparisons): a gap
    // there is repo debt, so a new chain inherits it rather than being blocked.
    let repo_wide: Vec<&str> = rules
        .touchpoints
        .iter()
        .filter(|t| t.kind == Kind::DefaultSetEquality)
        .map(|t| t.id.as_str())
        .collect();
    for chain in &chains {
        let names = Names::new(chain, None)?;
        let report = check_with(rules, root, &names)?;
        let is_new = new_crates.contains(chain);
        let allowed = pr::allowed_gaps(chain, is_new, &head_gaps, &repo_wide);
        let _ = writeln!(
            body,
            "### `{chain}`{}: {}\n",
            if is_new { " (new crate)" } else { "" },
            report.summary()
        );
        let mut unexpected = 0;
        for item in &report.items {
            match item.status {
                Status::Missing => match allowed.get(&item.id) {
                    Some(reason) => {
                        let _ = writeln!(body, "- known gap: {} ({reason})", item.text);
                    }
                    None => {
                        let _ = writeln!(body, "- **missing:** {}", item.text);
                        unexpected += 1;
                    }
                },
                Status::Info => {
                    let _ = writeln!(body, "- {}", item.text);
                }
                Status::Ok => {}
            }
        }
        if unexpected > 0 {
            failures.push(format!(
                "`{chain}`: {unexpected} touch point(s) missing that aren't known gaps (listed under `{chain}` below)"
            ));
        }
        let missing = report.missing_ids();
        for id in allowed.keys().filter(|id| !missing.contains(id.as_str())) {
            failures.push(format!(
                "`{chain}`: known gap `{id}` is fixed; remove it from `known-gaps.toml`"
            ));
        }

        if is_new {
            for what in pr::missing_policy(&event.body) {
                failures.push(format!(
                    "`{chain}` is a new chain crate; the PR body needs {what}"
                ));
            }
        }

        if root
            .join(format!("src/chain_parsers/visualsign-{chain}"))
            .is_dir()
        {
            let claims = pr::claims_c_free(&event.body) || in_apple_no_c(root, chain);
            match c_deps(root, chain) {
                Ok(tree) if tree.trim().is_empty() => {
                    let _ = writeln!(
                        body,
                        "- C dependencies on `aarch64-apple-ios`: none (C-free)"
                    );
                }
                Ok(tree) => {
                    let _ = writeln!(
                        body,
                        "- C dependencies on `aarch64-apple-ios` (`cargo tree -i cc`):\n\n```text\n{}\n```",
                        head_lines(&tree, 20)
                    );
                    if claims {
                        failures.push(format!(
                            "`{chain}` is declared C-free (PR body or `APPLE_NO_C_CRATES`) but `cc` is in its iOS graph"
                        ));
                    }
                }
                Err(e) => warnings.push(format!("`{chain}`: could not run cargo tree: {e}")),
            }
        }
        body.push('\n');
    }

    if !pr::proto_added_chains(&proto_diff).is_empty() && !args.offline {
        let slug = args
            .repo_slug
            .clone()
            .or_else(|| std::env::var("GITHUB_REPOSITORY").ok());
        let ours = std::fs::read_to_string(root.join(PROTO))
            .map(|t| pr::proto_chain_slots(&t))
            .unwrap_or_default();
        let base_slots = git(root, &["show", &format!("{base}:{PROTO}")])
            .map(|t| pr::proto_chain_slots(&t))
            .unwrap_or_default();
        match slug
            .ok_or_else(|| "no --repo-slug or $GITHUB_REPOSITORY".into())
            .and_then(|s| open_pr_slots(&s, event.number))
        {
            Ok(others) => {
                let prs: Vec<String> = others.iter().map(|(n, _)| format!("#{n}")).collect();
                let _ = writeln!(
                    body,
                    "Proto `Chain` slots compared with {} other open PR(s) that change the proto{}.\n",
                    prs.len(),
                    if prs.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", prs.join(", "))
                    }
                );
                for c in pr::slot_clashes(&base_slots, &ours, &others) {
                    warnings.push(format!(
                        "proto slot: this PR has `CHAIN_{} = {}`, #{} has `CHAIN_{} = {}`; agree on slots before merging",
                        c.ours.0, c.ours.1, c.pr, c.theirs.0, c.theirs.1
                    ));
                }
            }
            Err(e) => warnings.push(format!("could not compare proto slots with open PRs: {e}")),
        }
    }

    let mut out = format!("{PR_MARKER}\n## Chain wiring\n\n");
    if failures.is_empty() {
        out.push_str("**Passed.**\n\n");
    } else {
        out.push_str("**Failed:**\n\n");
        for f in &failures {
            let _ = writeln!(out, "- {f}");
        }
        out.push('\n');
    }
    if !warnings.is_empty() {
        out.push_str("**Warnings:**\n\n");
        for w in &warnings {
            let _ = writeln!(out, "- {w}");
        }
        out.push('\n');
    }
    out.push_str(&body);
    out.push_str("_Run locally: `cargo run -q -p chain-wiring -- check <chain>` from `src/`._\n");

    print!("{out}");
    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY")
        && !path.is_empty()
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)?;
        f.write_all(out.as_bytes())?;
    }
    Ok(if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
