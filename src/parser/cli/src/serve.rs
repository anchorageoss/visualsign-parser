//! `serve` subcommand: scans a directory of raw-transaction files, decodes
//! every file on each request, and serves a small local web UI for browsing
//! the results. `.json` files are passed through as-is; other files are
//! decoded as raw transactions through the chain registry.
//!
//! The server binds to `127.0.0.1` only — this is intentional, the feature
//! is for local triage, not network exposure. There is no auth and no TLS.
//!
//! Re-decoding happens on every HTTP request rather than once at startup,
//! so editing a fixture and refreshing the browser is enough to see the
//! new state — no server restart needed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use clap::Args;
use parser_cli_core::chains::parse_chain;
use parser_cli_core::{Runtime, prepare_runtime};
use serde::{Deserialize, Serialize};
use visualsign::registry::TransactionConverterRegistry;
use visualsign::vsptrait::VisualSignOptions;

use crate::ChainArgs;

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
    #[arg(long, default_value_t = 47474, help = "Port to bind on 127.0.0.1")]
    pub port: u16,

    /// Per-chain CLI args (ABI/IDL mappings, etc.).
    #[command(flatten)]
    pub chains: ChainArgs,
}

#[derive(Debug, Clone)]
struct DecodedEntry {
    rel_path: String,
    result: Result<serde_json::Value, String>,
}

#[derive(Clone)]
struct AppState {
    dir: Arc<PathBuf>,
    chain: Arc<String>,
    runtime: Arc<Runtime>,
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
    payload: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Entry point for the `serve` subcommand. Validates the directory, then
/// serves a small local web UI on `127.0.0.1:port`. Each request triggers
/// a fresh re-walk and re-decode of the directory — refresh the browser to
/// see edits.
pub fn run(args: &ServeArgs) -> Result<(), String> {
    let plugins = args.chains.build_plugins();
    let runtime = prepare_runtime(&args.chain, args.network.clone(), &plugins)?;

    validate_dir(&args.dir)?;

    eprintln!("Watching {} (re-decoded per request)", args.dir.display());

    let state = AppState {
        dir: Arc::new(args.dir.clone()),
        chain: Arc::new(args.chain.clone()),
        runtime: Arc::new(runtime),
    };
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/api/file", get(handle_file))
        .route("/{*path}", get(handle_payload))
        .with_state(state);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;

    rt.block_on(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], args.port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Failed to bind to {addr}: {e}"))?;
        match listener.local_addr() {
            Ok(bound) => println!("Serving on http://{bound}"),
            Err(_) => println!("Serving on http://{addr}"),
        }
        axum::serve(listener, app)
            .await
            .map_err(|e| format!("Server error: {e}"))
    })
}

fn validate_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("directory does not exist: {}", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    Ok(())
}

fn decode_directory(
    dir: &Path,
    chain_str: &str,
    runtime: &Runtime,
) -> Result<Vec<DecodedEntry>, String> {
    validate_dir(dir)?;

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
) -> Result<serde_json::Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty file".to_string());
    }

    if path.extension().and_then(|s| s.to_str()) == Some("json") {
        return serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|e| format!("invalid json: {e}"));
    }

    let chain = parse_chain(chain_str);
    let payload = registry
        .convert_transaction(&chain, trimmed, options.clone())
        .map_err(|e| format!("{e:?}"))?;
    serde_json::to_value(&payload).map_err(|e| format!("serialize: {e}"))
}

async fn load_entries(state: &AppState) -> Result<Vec<DecodedEntry>, String> {
    let dir = Arc::clone(&state.dir);
    let chain = Arc::clone(&state.chain);
    let runtime = Arc::clone(&state.runtime);
    tokio::task::spawn_blocking(move || decode_directory(&dir, &chain, &runtime))
        .await
        .map_err(|e| format!("join: {e}"))?
}

async fn handle_index(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let entries = match load_entries(&state).await {
        Ok(es) => es,
        Err(e) => return Ok(Html(render_error_page(&e))),
    };
    Ok(Html(render_html(&entries)))
}

/// Serve the decoded payload for a single file by its rel-path. Lets each
/// entry have its own bookmarkable / shareable URL — e.g.
/// `/token_2022/transfer_checked.json` returns just that file's payload as
/// JSON. Wraps no envelope around it so browsers and `curl` see the raw
/// `SignablePayload` (or the verbatim file content for `.json` passthrough).
async fn handle_payload(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let entries = load_entries(&state)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let entry = entries
        .iter()
        .find(|e| e.rel_path == path)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("not found: {path}\n")))?;
    match &entry.result {
        Ok(value) => Ok(Json(value.clone())),
        Err(err) => Err((StatusCode::UNPROCESSABLE_ENTITY, format!("{err}\n"))),
    }
}

async fn handle_file(
    State(state): State<AppState>,
    Query(q): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let entries = load_entries(&state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let entry = entries
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

/// Percent-encode the segments that need it so a rel path becomes a URL.
/// Segment separators (`/`) are preserved. Common safe filename characters
/// (alphanumerics, `-`, `_`, `.`) are left alone; everything else is
/// escaped. Sufficient for filesystem rel paths; not a general-purpose
/// URL encoder.
fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
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

const STYLE: &str = "body{font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;max-width:1100px;margin:1.5em auto;padding:0 1em;color:#222}\
h1{font-size:1.2em;margin-bottom:1em}\
details{margin:0.4em 0;border-left:3px solid #ccc;padding:0.4em 0.8em;background:#fafafa}\
details[open]{background:#fff;border-left-color:#456}\
summary{font-family:ui-monospace,Menlo,Consolas,monospace;cursor:pointer;font-weight:600}\
summary.err{color:#b00}\
summary .path{word-break:break-all}\
summary a.open,summary button.copy{font-weight:400;color:#456;text-decoration:none;margin-left:0.5em;font-size:0.85em;display:inline-block;padding:0.25em 0.5em;font-family:inherit}\
summary button.copy{background:#eef;border:1px solid #cce;border-radius:3px;cursor:pointer}\
summary button.copy:hover{background:#dde}\
summary button.copy.copied{background:#dfd;border-color:#9c9}\
summary a.open:hover{text-decoration:underline}\
pre{background:#f5f5f5;padding:0.8em;overflow:auto;font-size:12px;border-radius:3px;margin-top:0.6em;white-space:pre}\
footer{margin-top:1.5em;color:#888;font-size:0.85em}\
@media (max-width:600px){body{margin:0.5em auto;padding:0 0.5em}h1{font-size:1.05em}}";

const COPY_SCRIPT: &str = "document.addEventListener('click',function(e){\
var btn=e.target.closest('button.copy');\
if(!btn)return;\
e.preventDefault();e.stopPropagation();\
var pre=btn.closest('details').querySelector('pre');\
if(!pre||!navigator.clipboard)return;\
navigator.clipboard.writeText(pre.textContent).then(function(){\
var orig=btn.textContent;\
btn.textContent='copied!';btn.classList.add('copied');\
setTimeout(function(){btn.textContent=orig;btn.classList.remove('copied');},1200);\
});\
});";

fn render_html(entries: &[DecodedEntry]) -> String {
    use std::fmt::Write as _;

    let ok = entries.iter().filter(|e| e.result.is_ok()).count();
    let err = entries.len() - ok;

    let mut body = String::new();
    let _ = write!(
        body,
        "<h1>parser_cli &mdash; {} entries ({ok} ok, {err} error)</h1>",
        entries.len()
    );

    if entries.is_empty() {
        body.push_str("<p>No files found in directory.</p>");
    }

    for entry in entries {
        let escaped_path = html_escape(&entry.rel_path);
        let url_path = url_encode_path(&entry.rel_path);
        match &entry.result {
            Ok(value) => {
                let json = serde_json::to_string_pretty(value)
                    .unwrap_or_else(|e| format!("(serialization error: {e})"));
                let _ = write!(
                    body,
                    "<details><summary><span class=path>{escaped_path}</span> <a class=open href=\"/{url_path}\">[json]</a><button class=copy type=button>copy</button></summary><pre>{}</pre></details>",
                    html_escape(&json),
                );
            }
            Err(err) => {
                let _ = write!(
                    body,
                    "<details><summary class=err><span class=path>{escaped_path}</span> &mdash; error <a class=open href=\"/{url_path}\">[json]</a><button class=copy type=button>copy</button></summary><pre>{}</pre></details>",
                    html_escape(err),
                );
            }
        }
    }

    body.push_str("<footer>Refresh to re-decode from disk. <code>.json</code> files are served as-is; everything else is decoded through the chain registry.</footer>");

    format!(
        "<!DOCTYPE html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>parser_cli serve</title><style>{STYLE}</style></head><body>{body}<script>{COPY_SCRIPT}</script></body></html>"
    )
}

fn render_error_page(msg: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>parser_cli serve</title><style>{STYLE}</style></head><body><h1>parser_cli &mdash; error</h1><pre>{}</pre><footer>Refresh once the underlying issue is fixed.</footer></body></html>",
        html_escape(msg)
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        let plugins = ChainArgs::default().build_plugins();
        prepare_runtime("ethereum", Some("ETHEREUM_MAINNET".to_string()), &plugins).unwrap()
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
    fn json_files_passthrough_as_is() {
        let dir = temp_dir("json_passthrough");
        let payload = serde_json::json!({"hello": "world", "n": 42});
        fs::write(dir.join("expected.json"), payload.to_string()).unwrap();

        let entries = decode_directory(&dir, "ethereum", &make_runtime()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rel_path, "expected.json");
        let value = entries[0].result.as_ref().expect("json should parse");
        assert_eq!(value, &payload);
    }

    #[test]
    fn malformed_json_errors_with_invalid_json_prefix() {
        let dir = temp_dir("json_bad");
        fs::write(dir.join("bad.json"), "{not json}").unwrap();

        let entries = decode_directory(&dir, "ethereum", &make_runtime()).unwrap();
        assert_eq!(entries.len(), 1);
        let err = entries[0].result.as_ref().unwrap_err();
        assert!(err.starts_with("invalid json:"), "got: {err}");
    }

    #[test]
    fn mixed_hex_and_json_directory_decodes_both() {
        let dir = temp_dir("mixed_hex_json");
        fs::write(dir.join("a.hex"), VALID_HEX).unwrap();
        fs::write(dir.join("b.json"), r#"{"sentinel":"value"}"#).unwrap();

        let entries = decode_directory(&dir, "ethereum", &make_runtime()).unwrap();
        assert_eq!(entries.len(), 2);
        // Both succeed, but via different paths.
        let hex_value = entries[0].result.as_ref().expect("hex should decode");
        assert_eq!(hex_value["Title"], "Ethereum Transaction");
        let json_value = entries[1].result.as_ref().expect("json should parse");
        assert_eq!(json_value["sentinel"], "value");
    }

    #[test]
    fn url_encode_path_preserves_separators_and_safe_chars() {
        assert_eq!(
            url_encode_path("token_2022/transfer_checked.json"),
            "token_2022/transfer_checked.json"
        );
        assert_eq!(url_encode_path("a b/c.hex"), "a%20b/c.hex");
        assert_eq!(url_encode_path("dir/file?weird"), "dir/file%3Fweird");
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
        assert!(html.contains("Refresh to re-decode"));
        // Each entry exposes a standalone-link to its rel-path.
        assert!(html.contains("href=\"/good.hex\""), "got: {html}");
        assert!(html.contains("href=\"/bad.hex\""), "got: {html}");
        // Mobile-friendly: viewport meta tag present.
        assert!(
            html.contains("name=viewport") && html.contains("width=device-width"),
            "got: {html}"
        );
        // Each entry has a copy button + the inline script that wires it up.
        let copy_buttons = html.matches("<button class=copy").count();
        assert_eq!(copy_buttons, entries.len(), "got: {html}");
        assert!(html.contains("navigator.clipboard"), "got: {html}");
    }

    #[test]
    fn render_error_page_is_mobile_friendly() {
        let html = render_error_page("boom");
        assert!(html.contains("name=viewport") && html.contains("width=device-width"));
        assert!(html.contains("boom"));
    }
}
