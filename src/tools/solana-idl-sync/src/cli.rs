//! Command-line surface of the `solana-idl-sync` tool.
//!
//! ```text
//! solana-idl-sync list-presets
//! solana-idl-sync apply --fetched-dir DIR [--repo-root DIR] [--sections-dir DIR]
//! solana-idl-sync replay --program-id ID --before BIN --after BIN [--limit N] --out-dir DIR
//! solana-idl-sync report --dir DIR [--max-bytes N]
//! ```
//!
//! Exit codes: 0 success (including "nothing changed"), 1 every error.
//! The RPC URL is read from `SOLANA_RPC_URL` only, and never printed.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::{apply, presets, replay, report};

#[derive(Parser, Debug)]
#[command(about = "Keep the Solana preset IDLs in sync with chain.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print every preset IDL as a single-line JSON matrix
    /// (program_id + idl_path), sorted by program id, for the workflow.
    ListPresets,
    /// Write fetched IDLs over the committed preset IDLs where they
    /// differ (with their provenance records), and print the changed
    /// program ids as a JSON array.
    Apply {
        /// directory holding `<program_id>.json` (+ `.provenance.json`)
        /// fetched by solana-idl-fetch
        #[arg(long = "fetched-dir")]
        fetched_dir: PathBuf,
        /// repository root the preset IDL paths are relative to
        #[arg(long = "repo-root", default_value = ".")]
        repo_root: PathBuf,
        /// also write one `compare-<program_id>.md` section per changed
        /// program here, for the report
        #[arg(long = "sections-dir")]
        sections_dir: Option<PathBuf>,
    },
    /// Decode recent successful mainnet transactions of one program with
    /// parser_cli built before and after the IDL change, writing one
    /// `replay-<program_id>.md` section. The RPC URL comes from
    /// SOLANA_RPC_URL only (never printed; it carries the API key).
    Replay {
        #[arg(long = "program-id")]
        program_id: String,
        /// parser_cli built from the pre-change tree
        #[arg(long)]
        before: PathBuf,
        /// parser_cli built from the updated tree
        #[arg(long)]
        after: PathBuf,
        /// how many transactions to replay
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long = "out-dir")]
        out_dir: PathBuf,
    },
    /// Assemble report.md (PR-body sized) and report.full.md from the
    /// sections in `--dir`.
    Report {
        #[arg(long)]
        dir: PathBuf,
        /// cap for report.md (the PR body); report.full.md is untruncated
        #[arg(long = "max-bytes", default_value_t = 60_000)]
        max_bytes: usize,
    },
}

/// Parse the process arguments and run. Returns the process exit code.
pub fn main() -> i32 {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::ListPresets => {
            println!("{}", presets::list_presets_json()?);
        }
        Command::Apply {
            fetched_dir,
            repo_root,
            sections_dir,
        } => {
            let changed = apply::apply(fetched_dir, repo_root, sections_dir.as_deref())?;
            println!("{}", apply::changed_json(&changed));
        }
        Command::Replay {
            program_id,
            before,
            after,
            limit,
            out_dir,
        } => {
            replay::replay_cli(program_id, before, after, *limit, out_dir)?;
        }
        Command::Report { dir, max_bytes } => {
            report::write_reports(dir, *max_bytes)?;
        }
    }
    Ok(())
}
