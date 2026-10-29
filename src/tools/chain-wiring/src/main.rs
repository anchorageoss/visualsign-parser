//! `chain-wiring`: checks a chain parser's wiring and keeps its tracking issue current.
//!
//! ```text
//! chain-wiring check <chain> [--variant V] [--format text|markdown]   exit 1 on any MISSING
//! chain-wiring check-all                                              every discovered chain
//! chain-wiring issue-body <chain> [--variant V]                      the M1 block for the issue
//! chain-wiring splice <chain> [--variant V] < body > new-body         swap the M1 block in
//! chain-wiring issue-chain < body                                     prints "<chain> <Variant>"
//! ```

use chain_wiring::{Names, Report, Rules, check_with, issue_names, splice};
use clap::{Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod pr_cmd;

#[derive(Parser)]
#[command(
    name = "chain-wiring",
    about = "Check that a chain parser is wired into the repo"
)]
struct Cli {
    /// Repo root. Defaults to the nearest ancestor of the current directory with `src/chain_parsers`.
    #[arg(long, global = true)]
    repo: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Markdown,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check one chain. Exits 1 if any touch point is missing.
    Check {
        chain: String,
        /// Rust enum variant, if not the chain name with its first letter upper-cased.
        #[arg(long)]
        variant: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Check every chain under src/chain_parsers. Exits 1 if any touch point is missing.
    CheckAll,
    /// Print the M1 wiring block for a chain's tracking issue, markers included.
    IssueBody {
        chain: String,
        #[arg(long)]
        variant: Option<String>,
    },
    /// Read an issue body on stdin; print it with only the M1 wiring block replaced.
    Splice {
        chain: String,
        #[arg(long)]
        variant: Option<String>,
    },
    /// Read a chain-support issue body on stdin; print "<chain> <Variant>" from its form answers.
    IssueChain,
    /// Print the chains a pull request affects, one per line.
    ChangedChains(pr_cmd::ChangedArgs),
    /// Validate the chain wiring of a pull request (CI). Writes markdown to $GITHUB_STEP_SUMMARY.
    PrCheck(pr_cmd::PrCheckArgs),
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("chain-wiring: {e}");
            ExitCode::from(2)
        }
    }
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn run() -> Res<ExitCode> {
    let cli = Cli::parse();
    let root = match cli.repo {
        Some(r) => r,
        None => find_root(&std::env::current_dir()?)?,
    };
    let rules = Rules::builtin()?;
    let report = |chain: &str, variant: &Option<String>| -> Res<Report> {
        let names = Names::new(chain, variant.as_deref())?;
        Ok(check_with(&rules, &root, &names)?)
    };
    let fail_if = |missing: bool| {
        if missing {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        }
    };

    match cli.cmd {
        Cmd::Check {
            chain,
            variant,
            format,
        } => {
            let r = report(&chain, &variant)?;
            match format {
                Format::Text => print!("{}", r.render_text()),
                Format::Markdown => print!("{}", r.render_markdown()),
            }
            Ok(fail_if(r.missing() > 0))
        }
        Cmd::CheckAll => {
            let mut any = false;
            for (i, chain) in rules.discover_chains(&root)?.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                let r = report(chain, &None)?;
                print!("{}", r.render_text());
                any |= r.missing() > 0;
            }
            Ok(fail_if(any))
        }
        Cmd::IssueBody { chain, variant } => {
            print!("{}", report(&chain, &variant)?.issue_block());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Splice { chain, variant } => {
            let body = read_stdin()?;
            let block = report(&chain, &variant)?.issue_block();
            let out = splice(&body, &block).map_err(chain_wiring::Error::Splice)?;
            std::io::stdout().write_all(out.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::IssueChain => {
            let n = issue_names(&read_stdin()?)?;
            println!("{} {}", n.chain, n.variant);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::ChangedChains(args) => pr_cmd::changed_chains(&root, &args),
        Cmd::PrCheck(args) => pr_cmd::pr_check(&root, &rules, &args),
    }
}

fn read_stdin() -> Res<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

fn find_root(start: &Path) -> Res<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join("src/chain_parsers").is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| "no src/chain_parsers above the current directory; pass --repo".into())
}
