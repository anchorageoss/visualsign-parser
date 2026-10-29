//! `wallet-integration`: find changes that break wallet integrators, and check
//! that a PR making them is labeled and says who must upgrade first.
//!
//! - `detect --base <ref> --head <ref>`: list findings (always exits 0).
//! - `check-pr --base <sha> --head <sha> --labeled <bool> --body-file <path>`:
//!   the PR check. Fails when findings exist but the PR isn't labeled
//!   `breaking-wallet-integration`, or when a labeled PR's description lacks a
//!   valid `## Wallet integration` section.
//! - `parse-body --body-file <path>`: print a description's parsed section.
//!
//! The rules live in `src/rules.rs`, the section format in `src/body.rs`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use wallet_integration::{body, detect_refs, diff::Side, rules::Finding, LABEL};

#[derive(Parser)]
#[command(
    name = "wallet-integration",
    about = "Detect wallet-integration-breaking changes and check the PR declares them"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List wallet-breaking changes `head` makes since its merge base with `base`
    Detect(RangeArgs),
    /// PR check: findings need the label, the label needs a valid section
    CheckPr(CheckPrArgs),
    /// Parse and print a PR description's `## Wallet integration` section
    ParseBody(BodyArgs),
}

#[derive(clap::Args)]
struct RangeArgs {
    #[arg(long)]
    base: String,
    #[arg(long)]
    head: String,
    /// Repository to diff in
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}

#[derive(clap::Args)]
struct CheckPrArgs {
    #[command(flatten)]
    range: RangeArgs,
    /// Whether the PR carries the `breaking-wallet-integration` label
    #[arg(long, action = clap::ArgAction::Set)]
    labeled: bool,
    #[command(flatten)]
    body: BodyArgs,
    /// Also emit GitHub Actions `::error` annotations
    #[arg(long)]
    github: bool,
}

#[derive(clap::Args)]
struct BodyArgs {
    /// File holding the PR description
    #[arg(long)]
    body_file: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Detect(args) => {
            let findings = findings(&args)?;
            print_findings(&findings);
            Ok(())
        }
        Command::CheckPr(args) => check_pr(&args),
        Command::ParseBody(args) => {
            let text = read_body(&args)?;
            let section = body::parse_section(&text).map_err(section_error)?;
            print!("{section}");
            Ok(())
        }
    }
}

fn findings(args: &RangeArgs) -> Result<Vec<Finding>> {
    detect_refs(&args.repo, &args.base, &args.head).map_err(|e| anyhow!(e))
}

fn read_body(args: &BodyArgs) -> Result<String> {
    std::fs::read_to_string(&args.body_file)
        .with_context(|| format!("read {}", args.body_file.display()))
}

fn print_findings(findings: &[Finding]) {
    if findings.is_empty() {
        println!("no wallet-integration-breaking changes found");
        return;
    }
    println!("{} wallet-integration-breaking change(s):", findings.len());
    for f in findings {
        println!("  {f}");
    }
}

fn section_error(problems: Vec<String>) -> anyhow::Error {
    anyhow!(
        "invalid `## {}` section:\n  - {}\n\n{}",
        body::SECTION_TITLE,
        problems.join("\n  - "),
        body::FORMAT_HELP
    )
}

fn check_pr(args: &CheckPrArgs) -> Result<()> {
    let findings = findings(&args.range)?;
    print_findings(&findings);

    if !args.labeled {
        if findings.is_empty() {
            return Ok(());
        }
        if args.github {
            for f in &findings {
                annotate(f);
            }
        }
        return Err(anyhow!(
            "this PR changes the wallet integration surface but is not labeled `{LABEL}`.\n\
             Integrators must upgrade before it deploys. Apply the `{LABEL}` label and add a \
             `## {}` section to the description (see CONTRIBUTING.md):\n\n{}",
            body::SECTION_TITLE,
            body::FORMAT_HELP
        ));
    }

    let text = read_body(&args.body)?;
    let section = body::parse_section(&text).map_err(|p| {
        if args.github {
            println!(
                "::error title=Wallet integration section::PR is labeled {LABEL} but its \
                 description has no valid `## {}` section",
                body::SECTION_TITLE
            );
        }
        section_error(p)
    })?;
    println!("labeled `{LABEL}`; declared requirements:\n{section}");
    if findings.is_empty() {
        println!(
            "note: labeled without detector findings; the label stands (it can mark breaks \
             the rules don't cover)"
        );
    }
    Ok(())
}

/// One `::error` per finding, on its file and line when it's a head-side line.
fn annotate(f: &Finding) {
    let loc = match f.first {
        Some((Side::Head, n)) => format!("file={},line={n},", f.path),
        _ => format!("file={},", f.path),
    };
    println!(
        "::error {loc}title=wallet-integration ({})::{}",
        f.rule, f.reason
    );
}
