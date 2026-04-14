//! Minimal tool for IDL metadata extraction.
//!
//! Replaces the Python snippets in `scripts/fuzz_all_idls.sh`.
//!
//! Usage:
//!   idl-meta locate-idls --manifest-path <path>
//!   idl-meta counts <idl-file.json>

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let subcmd = args.get(1).map(String::as_str);

    match subcmd {
        Some("locate-idls") => {
            let manifest_path = args
                .iter()
                .position(|a| a == "--manifest-path")
                .and_then(|i| args.get(i + 1))
                .context("usage: idl-meta locate-idls --manifest-path <Cargo.toml>")?;
            let dir = locate_idl_dir(manifest_path)?;
            println!("{}", dir.display());
        }
        Some("counts") => {
            let file = args
                .get(2)
                .context("usage: idl-meta counts <idl-file.json>")?;
            let (instructions, types) = idl_counts(file)?;
            println!("{instructions} {types}");
        }
        _ => {
            anyhow::bail!(
                "usage: idl-meta <locate-idls|counts> [args]\n\
                 \n  locate-idls --manifest-path <Cargo.toml>  \
                 Print the solana_parser IDL directory\n  \
                 counts <file.json>                          \
                 Print instruction and type counts"
            );
        }
    }
    Ok(())
}

/// Run `cargo metadata`, find the `solana_parser` package, and return
/// `<package_root>/src/solana/idls`.
fn locate_idl_dir(manifest_path: &str) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--manifest-path",
            manifest_path,
            "--format-version",
            "1",
        ])
        .output()
        .context("failed to run `cargo metadata`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("cargo metadata failed: {stderr}");
    }

    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse cargo metadata JSON")?;

    let packages = meta["packages"]
        .as_array()
        .context("no 'packages' array in cargo metadata")?;

    for pkg in packages {
        if pkg["name"].as_str() == Some("solana_parser") {
            let manifest = pkg["manifest_path"]
                .as_str()
                .context("missing manifest_path for solana_parser")?;
            let pkg_dir = Path::new(manifest)
                .parent()
                .context("manifest_path has no parent")?;
            let idl_dir = pkg_dir.join("src").join("solana").join("idls");
            if idl_dir.is_dir() {
                return Ok(idl_dir);
            }
            anyhow::bail!(
                "solana_parser found at {manifest} but IDL dir does not exist: {}",
                idl_dir.display()
            );
        }
    }

    anyhow::bail!("package 'solana_parser' not found in cargo metadata")
}

/// Parse an Anchor IDL JSON file and return (instruction_count, type_count).
fn idl_counts(path: &str) -> Result<(usize, usize)> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let idl: serde_json::Value =
        serde_json::from_str(&contents).with_context(|| format!("invalid JSON in {path}"))?;

    let instructions = idl["instructions"].as_array().map_or(0, Vec::len);
    let types = idl["types"].as_array().map_or(0, Vec::len);
    Ok((instructions, types))
}
