//! `serve` subcommand: scans a directory of raw-transaction files, decodes
//! every file once, and serves a small local web UI for browsing the results.
//!
//! The server binds to `127.0.0.1` only — this is intentional, the feature
//! is for local triage, not network exposure. There is no auth and no TLS.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use clap::Args;
use serde::{Deserialize, Serialize};
use visualsign::SignablePayload;
use visualsign::registry::TransactionConverterRegistry;
use visualsign::vsptrait::VisualSignOptions;

use crate::PluginArgs;
use crate::cli::{Runtime, prepare_runtime};

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Args for the `serve` subcommand.
#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Chain identifier (e.g. `ethereum`, `solana`).
    #[arg(short, long, help = "Chain type")]
    pub chain: String,

    /// Optional network override; same semantics as on `decode`.
    #[arg(
        long,
        short = 'n',
        value_name = "NETWORK",
        help = "Network identifier - same as the decode subcommand"
    )]
    pub network: Option<String>,

    /// Directory to scan recursively for raw-transaction files.
    #[arg(
        long,
        value_name = "DIR",
        help = "Directory of raw-transaction files (recursive scan)"
    )]
    pub dir: PathBuf,

    /// TCP port to bind on `127.0.0.1`.
    #[arg(long, default_value_t = 8080, help = "Port to bind on 127.0.0.1")]
    pub port: u16,

    /// Ethereum-specific CLI args (ABI mappings, etc.).
    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    pub ethereum: crate::ethereum::EthereumArgs,

    /// Solana-specific CLI args (IDL mappings, etc.).
    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub solana: crate::solana::SolanaArgs,
}

impl ServeArgs {
    fn plugin_args(&self) -> PluginArgs {
        PluginArgs {
            #[cfg(feature = "ethereum")]
            ethereum: self.ethereum.clone(),
            #[cfg(feature = "solana")]
            solana: self.solana.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct DecodedEntry {
    rel_path: String,
    result: Result<SignablePayload, String>,
}

#[derive(Clone)]
struct AppState {
    entries: Arc<Vec<DecodedEntry>>,
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

#[derive(Serialize)]
struct FileResponse<'a> {
    path: &'a str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a SignablePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Entry point for the `serve` subcommand. Decodes every file in
/// `args.dir` once, then serves a small local web UI on `127.0.0.1:port`.
///
/// # Panics
///
/// Panics if the tokio runtime cannot be constructed — only happens in
/// catastrophic environments (e.g. lacking the ability to create threads).
pub fn execute_serve(args: &ServeArgs) {
    let plugin_args = args.plugin_args();
    let runtime = match prepare_runtime(&args.chain, args.network.clone(), &plugin_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let chain_str = args.chain.clone();
    let entries = match decode_directory(&args.dir, &chain_str, &runtime) {
        Ok(es) => es,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    if entries.is_empty() {
        eprintln!("No transaction files found in {}", args.dir.display());
        return;
    }

    let ok = entries.iter().filter(|e| e.result.is_ok()).count();
    let err = entries.len() - ok;
    eprintln!(
        "Loaded {} entries from {} ({ok} ok, {err} error)",
        entries.len(),
        args.dir.display(),
    );

    let state = AppState {
        entries: Arc::new(entries),
    };
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/api/file", get(handle_file))
        .with_state(state);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], args.port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to bind to {addr}: {e}");
                std::process::exit(1);
            }
        };
        match listener.local_addr() {
            Ok(bound) => println!("Serving on http://{bound}"),
            Err(_) => println!("Serving on http://{addr}"),
        }
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("Server error: {e}");
            std::process::exit(1);
        }
    });
}

fn decode_directory(
    dir: &Path,
    chain_str: &str,
    runtime: &Runtime,
) -> Result<Vec<DecodedEntry>, String> {
    if !dir.exists() {
        return Err(format!("directory does not exist: {}", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }

    let mut entries = Vec::new();
    walk(dir, dir, &mut entries, chain_str, runtime)?;
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(entries)
}

fn walk(
    base: &Path,
    current: &Path,
    out: &mut Vec<DecodedEntry>,
    chain_str: &str,
    runtime: &Runtime,
) -> Result<(), String> {
    let read =
        std::fs::read_dir(current).map_err(|e| format!("read_dir({}): {e}", current.display()))?;

    for entry in read {
        let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                out.push(DecodedEntry {
                    rel_path: rel_path(base, &path),
                    result: Err(format!("metadata: {e}")),
                });
                continue;
            }
        };
        if metadata.is_dir() {
            walk(base, &path, out, chain_str, runtime)?;
        } else if metadata.is_file() {
            let rel = rel_path(base, &path);
            if metadata.len() > MAX_FILE_SIZE {
                out.push(DecodedEntry {
                    rel_path: rel,
                    result: Err(format!("file exceeds {MAX_FILE_SIZE} bytes")),
                });
                continue;
            }
            let result = decode_file(&path, chain_str, &runtime.registry, &runtime.options);
            out.push(DecodedEntry {
                rel_path: rel,
                result,
            });
        }
    }
    Ok(())
}

fn rel_path(base: &Path, full: &Path) -> String {
    full.strip_prefix(base)
        .unwrap_or(full)
        .display()
        .to_string()
}

fn decode_file(
    path: &Path,
    chain_str: &str,
    registry: &TransactionConverterRegistry,
    options: &VisualSignOptions,
) -> Result<SignablePayload, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty file".to_string());
    }
    let chain = crate::chains::parse_chain(chain_str);
    registry
        .convert_transaction(&chain, trimmed, options.clone())
        .map_err(|e| format!("{e:?}"))
}

async fn handle_index(State(state): State<AppState>) -> Html<String> {
    Html(render_html(&state.entries))
}

async fn handle_file(
    State(state): State<AppState>,
    Query(q): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let entry = state
        .entries
        .iter()
        .find(|e| e.rel_path == q.path)
        .ok_or(StatusCode::NOT_FOUND)?;

    let response = match &entry.result {
        Ok(payload) => FileResponse {
            path: &entry.rel_path,
            ok: true,
            payload: Some(payload),
            error: None,
        },
        Err(err) => FileResponse {
            path: &entry.rel_path,
            ok: false,
            payload: None,
            error: Some(err),
        },
    };
    serde_json::to_value(&response)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn render_html(entries: &[DecodedEntry]) -> String {
    use std::fmt::Write as _;

    const STYLE: &str = "body{font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;max-width:1100px;margin:1.5em auto;padding:0 1em;color:#222}\
h1{font-size:1.2em;margin-bottom:1em}\
details{margin:0.4em 0;border-left:3px solid #ccc;padding:0.4em 0.8em;background:#fafafa}\
details[open]{background:#fff;border-left-color:#456}\
summary{font-family:ui-monospace,Menlo,Consolas,monospace;cursor:pointer;font-weight:600}\
summary.err{color:#b00}\
pre{background:#f5f5f5;padding:0.8em;overflow:auto;font-size:12.5px;border-radius:3px;margin-top:0.6em}";

    let ok = entries.iter().filter(|e| e.result.is_ok()).count();
    let err = entries.len() - ok;

    let mut body = String::new();
    let _ = write!(
        body,
        "<h1>parser_cli &mdash; {} entries ({ok} ok, {err} error)</h1>",
        entries.len()
    );

    for entry in entries {
        match &entry.result {
            Ok(payload) => {
                let json = serde_json::to_string_pretty(payload)
                    .unwrap_or_else(|e| format!("(serialization error: {e})"));
                let _ = write!(
                    body,
                    "<details><summary>{}</summary><pre>{}</pre></details>",
                    html_escape(&entry.rel_path),
                    html_escape(&json),
                );
            }
            Err(err) => {
                let _ = write!(
                    body,
                    "<details><summary class=err>{} &mdash; error</summary><pre>{}</pre></details>",
                    html_escape(&entry.rel_path),
                    html_escape(err),
                );
            }
        }
    }

    format!(
        "<!DOCTYPE html><html lang=en><head><meta charset=utf-8><title>parser_cli serve</title><style>{STYLE}</style></head><body>{body}</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vsp_serve_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_runtime() -> Runtime {
        let plugin_args = PluginArgs::default();
        prepare_runtime(
            "ethereum",
            Some("ETHEREUM_MAINNET".to_string()),
            &plugin_args,
        )
        .unwrap()
    }

    /// A real EIP-1559 ETH transfer, also used by the integration fixture.
    const VALID_HEX: &str = "02f86c0180830f4240843b9aca00830186a094111111111111111111111111111111111111111180b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead00000000000000000000000000000000000000000000000000000000000f4240c0";

    #[test]
    fn decode_directory_mixes_ok_and_err() {
        let dir = temp_dir("mixed");
        fs::write(dir.join("a-good.hex"), format!("  {VALID_HEX}\n")).unwrap();
        fs::write(dir.join("b-bad.hex"), "definitely not hex").unwrap();
        fs::write(dir.join("c-empty.hex"), "   \n\n").unwrap();
        // Hidden file should be skipped
        fs::write(dir.join(".dotfile"), VALID_HEX).unwrap();

        let runtime = make_runtime();
        let entries = decode_directory(&dir, "ethereum", &runtime).unwrap();
        assert_eq!(entries.len(), 3, "got: {entries:#?}");
        // sorted by rel_path
        assert_eq!(entries[0].rel_path, "a-good.hex");
        assert_eq!(entries[1].rel_path, "b-bad.hex");
        assert_eq!(entries[2].rel_path, "c-empty.hex");
        assert!(entries[0].result.is_ok());
        assert!(entries[1].result.is_err());
        assert!(entries[2].result.is_err());
    }

    #[test]
    fn decode_directory_recurses() {
        let dir = temp_dir("nested");
        let nested = dir.join("inner");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("tx.hex"), VALID_HEX).unwrap();

        let runtime = make_runtime();
        let entries = decode_directory(&dir, "ethereum", &runtime).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].rel_path.contains("tx.hex"));
        assert!(entries[0].result.is_ok());
    }

    #[test]
    fn decode_directory_missing_path_errors() {
        let err = decode_directory(
            Path::new("/nonexistent/vsp/serve/path"),
            "ethereum",
            &make_runtime(),
        )
        .unwrap_err();
        assert!(err.contains("does not exist"), "got: {err}");
    }

    #[test]
    fn html_escape_handles_all_specials() {
        assert_eq!(
            html_escape("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn render_html_contains_paths_and_payload() {
        let dir = temp_dir("html");
        fs::write(dir.join("good.hex"), VALID_HEX).unwrap();
        fs::write(dir.join("bad.hex"), "garbage").unwrap();
        let entries = decode_directory(&dir, "ethereum", &make_runtime()).unwrap();
        let html = render_html(&entries);
        assert!(html.contains("good.hex"));
        assert!(html.contains("bad.hex"));
        assert!(html.contains("Ethereum Transaction"));
        assert!(html.contains("class=err"));
    }
}
