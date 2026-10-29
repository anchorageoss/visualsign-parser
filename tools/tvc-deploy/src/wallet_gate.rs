//! Wallet-integration deploy gate for `deploy`.
//!
//! Finds the merged PRs labeled `breaking-wallet-integration` between the live
//! version and the image being deployed, prints each with the requirements its
//! `## Wallet integration` section declares, and refuses to deploy unless
//! `--ack-breaking-wallet-integration` lists exactly those PR numbers.
//!
//! - Live version: the live deployment's image tag (`tvc app status` plus its
//!   `create_tvc_deployment` activity), or `--since <ref>` when that can't be
//!   read. Target: the deployed image's tag, or `--target-ref`. A tag is a
//!   release `v<semver>` or a 40-hex commit sha (see stagex.yml).
//! - PRs: the GitHub API through `gh`, authenticated with `GH_TOKEN` only. A PR
//!   is in range when its merge commit is not in the live ref but is in the
//!   target ref (two compare calls).
//! - Fails closed: if the range or the PR list can't be resolved, the deploy
//!   stops. `--dangerous-skip-wallet-gate <REASON>` deploys anyway, with a
//!   banner and the reason in the output.

use std::collections::BTreeSet;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use wallet_integration::{ack, body, LABEL};
use xshell::{cmd, Shell};

use crate::invite;

const TOKEN_ENV: &str = "GH_TOKEN";

#[derive(clap::Args)]
pub struct WalletGateArgs {
    /// Exactly the `breaking-wallet-integration` PRs this deploy ships, comma
    /// separated (e.g. 625,640), after confirming every integrator listed in
    /// their `## Wallet integration` sections has upgraded. `none` when the
    /// gate finds none. A missing or extra number refuses the deploy
    #[arg(long, value_name = "PRS")]
    pub ack_breaking_wallet_integration: Option<String>,
    /// Git ref (release tag or commit) the live deployment runs. Required when
    /// it can't be read from TVC (no live deployment, or an untagged image)
    #[arg(long, value_name = "REF")]
    pub since: Option<String>,
    /// Git ref of the image being deployed; defaults to its tag
    #[arg(long, value_name = "REF")]
    pub target_ref: Option<String>,
    /// GitHub repository the PRs live in
    #[arg(long, default_value = "anchorageoss/visualsign-parser")]
    pub github_repo: String,
    /// Deploy even if the gate refuses or can't run (e.g. GitHub is down, or a
    /// non-production test app). Printed loudly and recorded in the output
    #[arg(long, value_name = "REASON")]
    pub dangerous_skip_wallet_gate: Option<String>,
}

/// A labeled PR as the gate reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    pub html_url: String,
}

/// Run the gate. Returns the line to record in the deploy output.
pub fn check(
    sh: &Shell,
    args: &WalletGateArgs,
    org: Option<&str>,
    app_id: &str,
    image_url: &str,
) -> Result<String> {
    let outcome = evaluate(sh, args, org, app_id, image_url);
    match (&args.dangerous_skip_wallet_gate, outcome) {
        (None, outcome) => outcome,
        (Some(reason), outcome) => {
            let why = match outcome {
                Ok(record) => format!("the gate passed anyway ({record})"),
                Err(e) => format!("the gate would have refused: {e:#}"),
            };
            println!("{}", "!".repeat(72));
            println!("WARNING: wallet-integration gate SKIPPED (--dangerous-skip-wallet-gate)");
            println!("  reason: {reason}");
            println!("  {why}");
            println!("{}", "!".repeat(72));
            Ok(format!("wallet-integration gate: SKIPPED ({reason})"))
        }
    }
}

fn evaluate(
    sh: &Shell,
    args: &WalletGateArgs,
    org: Option<&str>,
    app_id: &str,
    image_url: &str,
) -> Result<String> {
    // Parse the ack first: a typo should fail before any network call.
    let acked = args
        .ack_breaking_wallet_integration
        .as_deref()
        .map(ack::parse_ack)
        .transpose()
        .map_err(|e| anyhow!("--ack-breaking-wallet-integration: {e}"))?;

    let target = match &args.target_ref {
        Some(r) => r.clone(),
        None => image_ref(image_url).with_context(|| {
            format!(
                "can't tell which commit {image_url} was built from (tag isn't v<semver> or a \
                 commit sha); pass --target-ref <ref>"
            )
        })?,
    };
    let since = match &args.since {
        Some(r) => r.clone(),
        None => live_ref(sh, org, app_id)?,
    };
    println!(
        "wallet-integration gate: checking {} PRs in {since}..{target}",
        args.github_repo
    );

    let token = std::env::var(TOKEN_ENV)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .with_context(|| {
            format!(
                "{TOKEN_ENV} is not set; the wallet-integration gate needs it to list PRs \
                 (locally: {TOKEN_ENV}=$(gh auth token))"
            )
        })?;
    let gh = Gh {
        sh,
        token: &token,
        repo: &args.github_repo,
    };
    let prs = gh
        .labeled_prs_in_range(&since, &target)
        .context("listing breaking-wallet-integration PRs (failing closed)")?;

    for pr in &prs {
        print_pr(pr);
    }
    let shipped: BTreeSet<u64> = prs.iter().map(|p| p.number).collect();
    decide(&shipped, acked.as_ref())
}

/// The exact-set rule, given the shipped PRs and the operator's ack.
fn decide(shipped: &BTreeSet<u64>, acked: Option<&BTreeSet<u64>>) -> Result<String> {
    let flag = "--ack-breaking-wallet-integration";
    let Some(acked) = acked else {
        if shipped.is_empty() {
            return Ok(format!(
                "wallet-integration gate: no `{LABEL}` PRs in range"
            ));
        }
        bail!(
            "this deploy ships `{LABEL}` PRs; confirm every integrator above has upgraded, then \
             re-run with {flag} {}",
            ack::format_ack(shipped)
        );
    };
    ack::check_ack(shipped, acked).map_err(|m| {
        anyhow!(
            "{flag} must list exactly the `{LABEL}` PRs in this deploy ({}): {m}",
            if shipped.is_empty() {
                "none".to_owned()
            } else {
                ack::format_ack(shipped)
            }
        )
    })?;
    Ok(if shipped.is_empty() {
        format!("wallet-integration gate: no `{LABEL}` PRs in range (ack: none)")
    } else {
        let list: Vec<String> = shipped.iter().map(|n| format!("#{n}")).collect();
        format!(
            "wallet-integration gate: operator acknowledged {}",
            list.join(", ")
        )
    })
}

fn print_pr(pr: &PullRequest) {
    println!("  #{} {} ({})", pr.number, pr.title, pr.html_url);
    match body::parse_section(pr.body.as_deref().unwrap_or_default()) {
        Ok(section) => print!("{section}"),
        Err(problems) => println!(
            "  WARNING: no valid `## {}` section ({}); check the PR by hand",
            body::SECTION_TITLE,
            problems.join("; ")
        ),
    }
}

/// The ref the live deployment runs, read from TVC.
fn live_ref(sh: &Shell, org: Option<&str>, app_id: &str) -> Result<String> {
    let hint = "pass --since <tag or commit the live deployment runs>";
    let url = invite::live_image_url(sh, org, app_id)
        .with_context(|| format!("reading the live deployment of {app_id}; {hint}"))?
        .with_context(|| format!("can't find the live deployment's image for {app_id}; {hint}"))?;
    image_ref(&url)
        .with_context(|| format!("live image {url} isn't tagged v<semver> or a commit sha; {hint}"))
}

/// The git ref an image URL names: `repo:v0.1076.0@sha256:...` -> `v0.1076.0`,
/// `repo:<40-hex sha>` -> the sha. Other tags (`latest`, `pr-N`) move, so they
/// name no commit.
pub fn image_ref(image_url: &str) -> Option<String> {
    let without_digest = image_url.split('@').next()?;
    let last = without_digest.rsplit('/').next()?;
    let (_, tag) = last.split_once(':')?;
    let is_semver = tag.strip_prefix('v').is_some_and(|v| {
        let parts: Vec<&str> = v.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    });
    let is_sha = tag.len() == 40 && tag.bytes().all(|b| b.is_ascii_hexdigit());
    (is_semver || is_sha).then(|| tag.to_owned())
}

/// Whether a merge commit is in this deploy, from the GitHub compare statuses of
/// `live...merge` and `merge...target`.
fn in_range(live_to_merge: &str, merge_to_target: &str) -> Result<bool> {
    let in_live = match live_to_merge {
        "behind" | "identical" => true,
        "ahead" | "diverged" => false,
        other => bail!("unexpected compare status {other:?}"),
    };
    let in_target = match merge_to_target {
        "ahead" | "identical" => true,
        "behind" | "diverged" => false,
        other => bail!("unexpected compare status {other:?}"),
    };
    Ok(!in_live && in_target)
}

/// `gh api` with an explicit token, nothing from gh's own login.
struct Gh<'a> {
    sh: &'a Shell,
    token: &'a str,
    repo: &'a str,
}

impl Gh<'_> {
    fn api(&self, args: &[&str]) -> Result<String> {
        let sh = self.sh;
        cmd!(sh, "gh api {args...}")
            .env("GH_TOKEN", self.token)
            .env("GH_PROMPT_DISABLED", "1")
            .quiet()
            .read()
            .with_context(|| format!("gh api {}", args.join(" ")))
    }

    fn labeled_prs_in_range(&self, since: &str, target: &str) -> Result<Vec<PullRequest>> {
        let repo = self.repo;
        let numbers = self.api(&[
            "--paginate",
            "-X",
            "GET",
            &format!("repos/{repo}/issues"),
            "-f",
            &format!("labels={LABEL}"),
            "-f",
            "state=closed",
            "-f",
            "per_page=100",
            "--jq",
            ".[] | select(.pull_request.merged_at != null) | .number",
        ])?;
        let mut prs = Vec::new();
        for n in parse_numbers(&numbers)? {
            let pr: PullRequest =
                serde_json::from_str(&self.api(&[&format!("repos/{repo}/pulls/{n}")])?)
                    .with_context(|| format!("parsing PR #{n}"))?;
            let sha = pr
                .merge_commit_sha
                .clone()
                .with_context(|| format!("PR #{n} is merged but has no merge commit"))?;
            let a = self.compare(since, &sha)?;
            let b = self.compare(&sha, target)?;
            if in_range(&a, &b).with_context(|| format!("PR #{n}"))? {
                prs.push(pr);
            }
        }
        prs.sort_by_key(|p| p.number);
        Ok(prs)
    }

    fn compare(&self, base: &str, head: &str) -> Result<String> {
        let repo = self.repo;
        Ok(self
            .api(&[
                &format!("repos/{repo}/compare/{base}...{head}?per_page=1"),
                "--jq",
                ".status",
            ])?
            .trim()
            .to_owned())
    }
}

fn parse_numbers(lines: &str) -> Result<BTreeSet<u64>> {
    lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.parse().with_context(|| format!("not a PR number: {l:?}")))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn set(v: &[u64]) -> BTreeSet<u64> {
        v.iter().copied().collect()
    }

    #[test]
    fn image_ref_reads_release_and_sha_tags() {
        assert_eq!(
            image_ref("ghcr.io/anchorageoss/parser_app:v0.1076.0@sha256:abcd").as_deref(),
            Some("v0.1076.0")
        );
        let sha = "cefff3ecaef088ebe07c63ca0e7bb5be90f9c606";
        assert_eq!(
            image_ref(&format!("ghcr.io/anchorageoss/parser_app:{sha}")).as_deref(),
            Some(sha)
        );
        // Registry port, not a tag.
        assert_eq!(image_ref("localhost:5000/parser_app@sha256:ab"), None);
        assert_eq!(image_ref("ghcr.io/anchorageoss/parser_app:latest"), None);
        assert_eq!(image_ref("ghcr.io/anchorageoss/parser_app:pr-625"), None);
        assert_eq!(image_ref("ghcr.io/anchorageoss/parser_app:v1.2"), None);
        assert_eq!(image_ref("ghcr.io/anchorageoss/parser_app"), None);
    }

    #[test]
    fn in_range_needs_not_live_and_in_target() {
        // #625 merged after the live v0.1058.0, deploying v0.1076.0.
        assert!(in_range("ahead", "ahead").unwrap());
        // The target is the merge commit itself.
        assert!(in_range("ahead", "identical").unwrap());
        // Already live.
        assert!(!in_range("behind", "ahead").unwrap());
        assert!(!in_range("identical", "ahead").unwrap());
        // Merged after the target was cut.
        assert!(!in_range("ahead", "behind").unwrap());
        // Live is a side-branch build: a main merge it lacks still ships.
        assert!(in_range("diverged", "ahead").unwrap());
        assert!(in_range("bogus", "ahead").is_err());
    }

    #[test]
    fn decide_requires_the_exact_set() {
        let shipped = set(&[625]);
        let err = decide(&shipped, None).unwrap_err().to_string();
        assert!(
            err.contains("--ack-breaking-wallet-integration 625"),
            "{err}"
        );

        let err = decide(&shipped, Some(&set(&[]))).unwrap_err().to_string();
        assert!(err.contains("not acknowledged: #625"), "{err}");

        let err = decide(&shipped, Some(&set(&[625, 640])))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not in this deploy: #640"), "{err}");

        let record = decide(&set(&[625, 640]), Some(&set(&[640, 625]))).unwrap();
        assert_eq!(
            record,
            "wallet-integration gate: operator acknowledged #625, #640"
        );
    }

    #[test]
    fn decide_with_nothing_shipped() {
        assert!(decide(&set(&[]), None).is_ok());
        assert!(decide(&set(&[]), Some(&set(&[]))).is_ok());
        let err = decide(&set(&[]), Some(&set(&[625])))
            .unwrap_err()
            .to_string();
        assert!(err.contains("(none)"), "{err}");
    }

    fn gate_args(skip: Option<&str>) -> WalletGateArgs {
        WalletGateArgs {
            ack_breaking_wallet_integration: None,
            since: Some("v0.1058.0".to_owned()),
            target_ref: None,
            github_repo: "anchorageoss/visualsign-parser".to_owned(),
            dangerous_skip_wallet_gate: skip.map(str::to_owned),
        }
    }

    // `:latest` names no commit, so the gate fails before any network call.
    #[test]
    fn gate_fails_closed_unless_skipped_with_a_reason() {
        let sh = Shell::new().unwrap();
        let image = "ghcr.io/anchorageoss/parser_app:latest";
        let err = check(&sh, &gate_args(None), None, "app", image)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--target-ref"), "{err}");

        let record = check(&sh, &gate_args(Some("dev TEST app")), None, "app", image).unwrap();
        assert_eq!(record, "wallet-integration gate: SKIPPED (dev TEST app)");
    }

    #[test]
    fn a_malformed_ack_fails_before_anything_else() {
        let sh = Shell::new().unwrap();
        let mut args = gate_args(None);
        args.ack_breaking_wallet_integration = Some("625;640".to_owned());
        let err = check(&sh, &args, None, "app", "img:latest")
            .unwrap_err()
            .to_string();
        assert!(err.contains("--ack-breaking-wallet-integration"), "{err}");
    }

    #[test]
    fn parse_numbers_reads_jq_lines() {
        assert_eq!(parse_numbers("625\n\n640\n").unwrap(), set(&[625, 640]));
        assert!(parse_numbers("625\nnull\n").is_err());
    }

    #[test]
    fn pull_request_json_tolerates_null_body() {
        let pr: PullRequest = serde_json::from_str(
            r#"{"number":625,"title":"t","body":null,"merge_commit_sha":"cefff3ec","html_url":"u","extra":1}"#,
        )
        .unwrap();
        assert_eq!(pr.number, 625);
        assert!(pr.body.is_none());
    }
}
