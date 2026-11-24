//! Command-line surface of the `solana-idl-fetch` tool.
//!
//! Modes and exit codes, byte-compatible with the Python script this crate
//! replaces:
//!
//! ```text
//! solana-idl-fetch <program_id> [--rpc URL] [--out FILE]
//!                 [--compare FILE] [--provenance FILE]
//! solana-idl-fetch --record-repo <https-url> --commit <40-hex-sha>
//!                 --path <path-in-repo> --idl <idl.json> --program-id <id>
//! solana-idl-fetch --record-issuer '<who supplied it>' [--notes '<text>']
//!                 --idl <idl.json> --program-id <id>
//! solana-idl-fetch --record-unknown '<why the origin is unknown>'
//!                 --idl <idl.json> --program-id <id>
//! ```
//!
//! Exit codes: 0 fetched (or recorded), 2 the program has no on-chain IDL
//! or the invocation itself is wrong, 1 every other error (record-mode
//! validation, RPC and decoding failures).

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::compare;
use crate::json;
use crate::record::{self, is_full_sha, record_path_for, sha256_hex, utc_now_iso};
use crate::rpc::{HttpRpc, Rpc};
use crate::solana::{
    decode_idl_account, idl_address, parse_program_id, program_last_deployed_slot,
};
use solana_pubkey::Pubkey;

pub const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";

#[derive(Parser, Debug)]
#[command(about = "Fetch the Anchor IDL a Solana program publishes on chain.")]
struct Cli {
    /// program id to fetch (base58)
    program_id: Option<String>,

    /// JSON-RPC endpoint (default: https://api.mainnet-beta.solana.com)
    #[arg(long, default_value = DEFAULT_RPC)]
    rpc: String,

    /// write the IDL JSON here instead of stdout
    #[arg(long)]
    out: Option<PathBuf>,

    /// committed IDL JSON to summarize differences against (to stderr)
    #[arg(long)]
    compare: Option<PathBuf>,

    /// with --out, also write an 'onchain' provenance record here for the IDL just written
    #[arg(long)]
    provenance: Option<PathBuf>,

    /// write a 'repo' provenance record for an existing IDL: https URL of the repo it came from
    #[arg(long)]
    record_repo: Option<String>,

    /// full 40-hex commit of the repo the IDL came from (with --record-repo)
    #[arg(long)]
    commit: Option<String>,

    /// path of the IDL inside that repo (with --record-repo)
    #[arg(long)]
    path: Option<String>,

    /// write an 'issuer' provenance record for an existing IDL: free text naming who supplied it
    #[arg(long)]
    record_issuer: Option<String>,

    /// write an 'unknown' provenance record for an existing IDL: free text on why the origin can't be established
    #[arg(long)]
    record_unknown: Option<String>,

    /// free-text notes for --record-issuer
    #[arg(long, default_value = "")]
    notes: String,

    /// existing IDL file the record mode describes
    #[arg(long)]
    idl: Option<PathBuf>,

    /// program id (base58) the record mode writes the record for
    #[arg(long = "program-id")]
    record_program_id: Option<String>,
}

impl Cli {
    /// How many record modes are selected (presence, not emptiness: the
    /// empty-value errors come later, exactly like the Python script).
    fn record_mode_count(&self) -> usize {
        [
            self.record_repo.is_some(),
            self.record_issuer.is_some(),
            self.record_unknown.is_some(),
        ]
        .into_iter()
        .filter(|selected| *selected)
        .count()
    }
}

/// Run the tool. Returns the process exit code; `Err` becomes code 1.
fn run(cli: &Cli) -> Result<i32> {
    let record_modes = cli.record_mode_count();
    if record_modes > 0 {
        return run_record_mode(cli, record_modes);
    }
    let Some(program_id) = cli.program_id.as_deref() else {
        // argparse exits 2 for a missing positional argument.
        eprintln!("error: program_id is required unless recording a provenance record");
        return Ok(2);
    };
    let rpc = HttpRpc::new(&cli.rpc)?;
    run_fetch(cli, &rpc, program_id)
}

fn run_fetch(cli: &Cli, rpc: &dyn Rpc, program_id: &str) -> Result<i32> {
    let program = parse_program_id(program_id)?;
    let address = idl_address(&program)?;
    let info = rpc.get_account_info(&address.to_string(), None)?;
    let Some(data) = info.data else {
        eprintln!("{program_id} has no on-chain Anchor IDL (account {address} not found)");
        return Ok(2);
    };
    let idl = decode_idl_account(&data)?;
    let text = format!("{}\n", json::pretty(&idl));
    if let Some(out) = &cli.out {
        write_text_file(out, &text)?;
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(text.as_bytes())
            .context("failed to write the IDL JSON to stdout")?;
        stdout.flush().context("failed to flush stdout")?;
    }
    eprintln!("IDL account: {address}");
    if let Some(committed_path) = &cli.compare {
        let committed_text = fs::read_to_string(committed_path)
            .with_context(|| format!("failed to read {}", committed_path.display()))?;
        let committed = json::parse(&committed_text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("{} is not valid JSON", committed_path.display()))?;
        for line in compare::compare_lines(&idl, &committed)? {
            eprintln!("{line}");
        }
    }
    if let Some(provenance) = &cli.provenance {
        if cli.out.is_none() {
            bail!("--provenance requires --out (the record hashes the exact IDL bytes written)");
        }
        // The slot is context in the record, never a reason to fail a
        // fetch: any failure along the way means "unknown".
        let last_deployed = program_last_deployed_slot(rpc, program_id);
        let record_text = record::onchain_record(
            program_id,
            &sha256_hex(text.as_bytes()),
            &utc_now_iso(),
            &address.to_string(),
            info.slot,
            last_deployed,
        );
        write_text_file(provenance, &record_text)?;
        eprintln!("provenance record: {}", provenance.display());
        eprintln!("rpc slot: {}", info.slot);
        match last_deployed {
            Some(slot) => eprintln!("program last deployed slot: {slot}"),
            None => eprintln!("program last deployed slot: unknown"),
        }
    }
    Ok(0)
}

fn run_record_mode(cli: &Cli, mode_count: usize) -> Result<i32> {
    if mode_count > 1 {
        bail!("pass at most one of --record-repo, --record-issuer, --record-unknown");
    }
    if cli.program_id.is_some() {
        bail!("record modes take the program id via --program-id, not the positional argument");
    }
    if (cli.commit.is_some() || cli.path.is_some()) && cli.record_repo.is_none() {
        bail!("--commit and --path only apply to --record-repo");
    }
    if !cli.notes.is_empty() && cli.record_issuer.is_none() {
        bail!("--notes only applies to --record-issuer");
    }
    for (flag, value) in [
        ("--out", &cli.out),
        ("--compare", &cli.compare),
        ("--provenance", &cli.provenance),
    ] {
        if value.is_some() {
            bail!("{flag} only applies to an on-chain fetch, not to a record mode");
        }
    }
    let (Some(idl), Some(record_program_id)) =
        (cli.idl.as_deref(), cli.record_program_id.as_deref())
    else {
        bail!("record modes need --idl FILE and --program-id ID");
    };
    if !idl.is_file() {
        bail!("{} not found", idl.display());
    }
    Pubkey::from_str(record_program_id)
        .map_err(|e| anyhow::anyhow!("invalid --program-id: {e}"))?;

    let idl_bytes = fs::read(idl).with_context(|| format!("failed to read {}", idl.display()))?;
    let idl_sha256 = sha256_hex(&idl_bytes);
    let recorded_at = utc_now_iso();

    let record_text = if let Some(repo) = cli.record_repo.as_deref() {
        if !repo.starts_with("https://") {
            bail!("--record-repo must be an https URL");
        }
        let (Some(commit), Some(path)) = (cli.commit.as_deref(), cli.path.as_deref()) else {
            bail!("--record-repo needs --commit and --path");
        };
        if !is_full_sha(commit) {
            bail!("--commit must be a full 40-character lowercase hex SHA");
        }
        if path.is_empty() {
            bail!("--path must not be empty");
        }
        record::repo_record(
            record_program_id,
            &idl_sha256,
            &recorded_at,
            repo,
            commit,
            path,
        )
    } else if let Some(issuer) = cli.record_issuer.as_deref() {
        if issuer.is_empty() {
            bail!("--record-issuer must not be empty");
        }
        record::issuer_record(
            record_program_id,
            &idl_sha256,
            &recorded_at,
            issuer,
            &cli.notes,
        )
    } else if let Some(unknown) = cli.record_unknown.as_deref() {
        if unknown.is_empty() {
            bail!("--record-unknown must not be empty (say why the origin is unknown)");
        }
        record::unknown_record(record_program_id, &idl_sha256, &recorded_at, unknown)
    } else {
        bail!("record mode not selected");
    };

    let record_path = record_path_for(Path::new(idl))?;
    write_text_file(&record_path, &record_text)?;
    eprintln!("provenance record: {}", record_path.display());
    Ok(0)
}

fn write_text_file(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text.as_bytes()).with_context(|| format!("failed to write {}", path.display()))
}

/// Parse the process arguments and run. Returns the exit code.
pub fn main() -> i32 {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    }
}
