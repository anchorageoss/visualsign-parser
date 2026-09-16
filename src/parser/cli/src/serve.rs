//! `serve` subcommand: scans a directory of raw-transaction files, decodes
//! every file on each request, and serves a small local web UI for browsing
//! the results. `.json` files are passed through as-is; other files are
//! decoded as raw transactions through the chain registry.
//!
//! The server binds `127.0.0.1` by default: the feature is local triage and
//! carries no auth and no TLS. `--host` overrides the bind address for the case
//! where the client is a separate device, such as a phone stepping through
//! `/next`. Any non-loopback address serves the directory to every host that
//! can route to the port, which is why it is an argument and not the default.
//!
//! Re-decoding happens on every HTTP request rather than once at startup,
//! so editing a fixture and refreshing the browser is enough to see the
//! new state — no server restart needed.
//!
//! `/next` walks the directory one payload per call, so a client that can be
//! given a URL but not content steps through every entry by refetching one
//! address instead of being handed each link in turn. It prepends a
//! `diagnostic` field naming the file and position, since a payload otherwise
//! carries nothing that says which file produced it.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderName, HeaderValue, StatusCode, header},
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

    /// Address to bind. Loopback unless overridden; `0.0.0.0` reaches every
    /// interface, which is what another device on the network needs.
    #[arg(
        long,
        value_name = "IP",
        default_value = "127.0.0.1",
        help = "Address to bind - 0.0.0.0 to accept connections from other hosts"
    )]
    pub host: IpAddr,

    /// TCP port to bind.
    #[arg(long, default_value_t = 47474, help = "Port to bind")]
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
    /// Call counter behind `/next`, taken modulo the decodable-entry count at
    /// request time. One cursor per server rather than per client, so two
    /// viewers stepping at once interleave.
    cursor: Arc<AtomicUsize>,
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

#[derive(Deserialize)]
struct NextQuery {
    /// `?bare=true` returns the payload exactly as the other routes do, with
    /// no step diagnostic prepended.
    #[serde(default)]
    bare: bool,
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
/// serves a small local web UI on `host:port`. Each request triggers
/// a fresh re-walk and re-decode of the directory — refresh the browser to
/// see edits.
pub fn run(args: &ServeArgs) -> Result<(), String> {
    let plugins = args.chains.build_plugins();
    let runtime = prepare_runtime(&args.chain, args.network.clone(), &plugins, false)?;

    validate_dir(&args.dir)?;

    eprintln!("Watching {} (re-decoded per request)", args.dir.display());

    let state = AppState {
        dir: Arc::new(args.dir.clone()),
        chain: Arc::new(args.chain.clone()),
        runtime: Arc::new(runtime),
        cursor: Arc::new(AtomicUsize::new(0)),
    };
    // `/next` and `/reset` are literal segments, which the router matches
    // ahead of the `/{*path}` wildcard. Files named `next` or `reset` are
    // therefore unreachable by path and have to be fetched through
    // `/api/file?path=`.
    let app = Router::new()
        .route("/", get(handle_index))
        .route("/api/file", get(handle_file))
        .route("/next", get(handle_next))
        .route("/reset", get(handle_reset))
        .route("/{*path}", get(handle_payload))
        .with_state(state);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| format!("build tokio runtime: {e}"))?;

    rt.block_on(async move {
        let addr = SocketAddr::new(args.host, args.port);
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
    // `convert_transaction` returns a `ConversionResult` (payload + optional
    // intermediate_output); only the inner `SignablePayload` is `Serialize`, so
    // serialize that. The `serve` UI is interactive triage and never opts into
    // `include_intermediate_output`, so the intermediate blob is `None` here
    // anyway.
    serde_json::to_value(&payload.payload).map_err(|e| format!("serialize: {e}"))
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

/// A JSON body that a cache must not keep.
///
/// `/next` and `/reset` are GETs that move the cursor. A cached `/next` would
/// hand back the same payload however many times it was refetched, which
/// defeats the route's only purpose; a cached `/reset` would return its
/// acknowledgement without the store ever happening.
type NoStoreJson = ([(HeaderName, HeaderValue); 1], Json<serde_json::Value>);

fn no_store(value: serde_json::Value) -> NoStoreJson {
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(value),
    )
}

/// Serve one payload per call, advancing a shared cursor and wrapping at the
/// end, so the same URL yields the whole directory over successive fetches.
///
/// Only entries that decoded are in the rotation: an undecodable file would
/// have nothing to return, and 422 mid-walk would stall a client that has no
/// way to skip. `/reset` puts the cursor back to the first entry.
///
/// One fetch is one step, so a client that requests twice per view — a
/// prefetch, a retry, a conditional request it does not cache — advances
/// twice. The step diagnostic names the file, so a skip is visible rather
/// than silent.
async fn handle_next(
    State(state): State<AppState>,
    Query(q): Query<NextQuery>,
) -> Result<NoStoreJson, (StatusCode, String)> {
    let entries = load_entries(&state)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let tick = state.cursor.fetch_add(1, Ordering::Relaxed);
    let step = pick_step(&entries, tick).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "no decodable payloads in directory\n".to_string(),
        )
    })?;
    if q.bare {
        return Ok(no_store(step.payload.clone()));
    }
    Ok(no_store(with_step_diagnostic(
        step.payload,
        step.position,
        step.count,
        step.rel_path,
    )))
}

/// Put the `/next` cursor back to the first entry. A GET mutates here because
/// the client this serves can be handed a URL and nothing else.
async fn handle_reset(State(state): State<AppState>) -> NoStoreJson {
    state.cursor.store(0, Ordering::Relaxed);
    no_store(serde_json::json!({ "reset": true, "next_step": 1 }))
}

/// One position in the walk. Carries the payload rather than the entry, so a
/// caller never has to re-check that this entry decoded.
struct StepPick<'a> {
    /// 1-based, for display.
    position: usize,
    /// How many entries are in the rotation.
    count: usize,
    rel_path: &'a str,
    payload: &'a serde_json::Value,
}

/// The entry `/next` serves on call number `tick`. Counts and indexes only
/// decodable entries, in the sorted order the index page lists. `None` when the
/// directory holds nothing that decodes.
fn pick_step(entries: &[DecodedEntry], tick: usize) -> Option<StepPick<'_>> {
    let decodable: Vec<(&str, &serde_json::Value)> = entries
        .iter()
        .filter_map(|e| e.result.as_ref().ok().map(|v| (e.rel_path.as_str(), v)))
        .collect();
    let count = decodable.len();
    if count == 0 {
        return None;
    }
    let idx = tick % count;
    decodable.get(idx).map(|(rel_path, payload)| StepPick {
        position: idx + 1,
        count,
        rel_path,
        payload,
    })
}

/// Copy `payload` with a `diagnostic` field prepended naming the file and its
/// position in the walk.
///
/// A `SignablePayload` carries no field identifying its source, so a viewer
/// handed a bare payload cannot say which file it came from. `diagnostic` is
/// the field type for a note about a payload rather than content within it,
/// which is what this is, and clients already render it.
///
/// A value with no `Fields` array — a file that is valid JSON but not a
/// payload — is returned untouched, since there is nowhere to put the field.
fn with_step_diagnostic(
    payload: &serde_json::Value,
    position: usize,
    count: usize,
    rel_path: &str,
) -> serde_json::Value {
    let message = format!("step {position} of {count}: {rel_path}");
    let field = serde_json::json!({
        "Diagnostic": {
            "Domain": "parser_cli-serve",
            "Level": "ok",
            "Message": message,
            "Rule": "serve::step",
        },
        "FallbackText": format!("ok: {message}"),
        "Label": "step",
        "Type": "diagnostic",
    });

    let mut out = payload.clone();
    match out.get_mut("Fields").and_then(|f| f.as_array_mut()) {
        Some(fields) => {
            fields.insert(0, field);
            out
        }
        None => out,
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

/// Root paths the literal routes take, which the wildcard therefore never
/// sees. A nested `sub/next` is unaffected: only the exact path collides.
const RESERVED_ROOT_PATHS: [&str; 2] = ["next", "reset"];

/// The index link for one entry.
///
/// A root file named for one of the walk routes cannot be reached at
/// `/{rel_path}`, because the literal route matches first. Linking it through
/// `/api/file` keeps every entry in the index reachable. That route answers
/// with the `{path, ok, payload}` envelope rather than a bare payload, which
/// is the visible difference for those two names.
fn entry_href(rel_path: &str) -> String {
    let encoded = url_encode_path(rel_path);
    if RESERVED_ROOT_PATHS.contains(&rel_path) {
        // No slash to escape: only a root name can collide, and a root name
        // has none.
        format!("/api/file?path={encoded}")
    } else {
        format!("/{encoded}")
    }
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
        let href = entry_href(&entry.rel_path);
        match &entry.result {
            Ok(value) => {
                let json = serde_json::to_string_pretty(value)
                    .unwrap_or_else(|e| format!("(serialization error: {e})"));
                let _ = write!(
                    body,
                    "<details><summary><span class=path>{escaped_path}</span> <a class=open href=\"{href}\">[json]</a><button class=copy type=button>copy</button></summary><pre>{}</pre></details>",
                    html_escape(&json),
                );
            }
            Err(err) => {
                let _ = write!(
                    body,
                    "<details><summary class=err><span class=path>{escaped_path}</span> &mdash; error <a class=open href=\"{href}\">[json]</a><button class=copy type=button>copy</button></summary><pre>{}</pre></details>",
                    html_escape(err),
                );
            }
        }
    }

    body.push_str("<footer>Refresh to re-decode from disk. <code>.json</code> files are served as-is; everything else is decoded through the chain registry.<br><a href=\"/next\">/next</a> serves one payload per call and wraps at the end, so a client that takes a URL steps through every entry by refetching it; the payload carries a <code>step</code> diagnostic naming the file. <a href=\"/reset\">/reset</a> returns to the first.</footer>");

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
        prepare_runtime(
            "ethereum",
            Some("ETHEREUM_MAINNET".to_string()),
            &plugins,
            false,
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

    /// Build entries directly rather than through a temp directory: `pick_step`
    /// cares only about decode success and sort order.
    fn entry(rel_path: &str, ok: bool) -> DecodedEntry {
        DecodedEntry {
            rel_path: rel_path.to_string(),
            result: if ok {
                Ok(serde_json::json!({ "Fields": [], "Title": rel_path }))
            } else {
                Err("nope".to_string())
            },
        }
    }

    #[test]
    fn pick_step_walks_in_order_then_wraps() {
        let entries = vec![entry("01.json", true), entry("02.json", true)];
        let walk: Vec<(usize, usize, String)> = (0..5)
            .map(|tick| {
                let s = pick_step(&entries, tick).unwrap();
                (s.position, s.count, s.rel_path.to_string())
            })
            .collect();
        assert_eq!(
            walk,
            vec![
                (1, 2, "01.json".to_string()),
                (2, 2, "02.json".to_string()),
                (1, 2, "01.json".to_string()),
                (2, 2, "02.json".to_string()),
                (1, 2, "01.json".to_string()),
            ]
        );
    }

    #[test]
    fn pick_step_skips_undecodable_and_counts_only_the_rest() {
        let entries = vec![
            entry("01.json", true),
            entry("02.hex", false),
            entry("03.json", true),
        ];
        let first = pick_step(&entries, 0).unwrap();
        assert_eq!(
            (first.position, first.count, first.rel_path),
            (1, 2, "01.json")
        );
        let second = pick_step(&entries, 1).unwrap();
        assert_eq!(
            (second.position, second.count, second.rel_path),
            (2, 2, "03.json")
        );
        // Wrapping is over the decodable count, so the broken file never lands.
        assert_eq!(pick_step(&entries, 2).unwrap().rel_path, "01.json");
    }

    #[test]
    fn pick_step_carries_the_decoded_payload() {
        let entries = vec![entry("01.json", true)];
        let picked = pick_step(&entries, 0).unwrap();
        assert_eq!(picked.payload["Title"], "01.json");
    }

    #[test]
    fn pick_step_is_none_when_nothing_decodes() {
        assert!(pick_step(&[], 0).is_none());
        assert!(pick_step(&[entry("01.hex", false)], 0).is_none());
    }

    #[test]
    fn step_diagnostic_leads_the_fields_and_names_the_file() {
        let payload = serde_json::json!({
            "Fields": [{ "Label": "Network", "Type": "text_v2" }],
            "Title": "NEAR Intent",
        });
        let out = with_step_diagnostic(&payload, 3, 14, "03-near-register.json");
        let fields = out["Fields"].as_array().unwrap();
        assert_eq!(fields.len(), 2, "the original field is kept");
        assert_eq!(fields[0]["Type"], "diagnostic");
        assert_eq!(fields[0]["Label"], "step");
        assert_eq!(
            fields[0]["Diagnostic"]["Message"],
            "step 3 of 14: 03-near-register.json"
        );
        assert_eq!(
            fields[0]["Diagnostic"]["Rule"], "serve::step",
            "namespaced so it cannot collide with a chain parser's rule"
        );
        assert_eq!(
            fields[0]["FallbackText"],
            "ok: step 3 of 14: 03-near-register.json"
        );
        // Everything else is untouched.
        assert_eq!(out["Title"], "NEAR Intent");
        assert_eq!(fields[1]["Label"], "Network");
    }

    #[test]
    fn step_diagnostic_leaves_a_value_with_no_fields_array_alone() {
        // A file that is valid JSON but not a payload: there is nowhere to put
        // the diagnostic, and dropping it beats inventing a shape.
        let list = serde_json::json!([{ "input": "0xdeadbeef" }]);
        assert_eq!(with_step_diagnostic(&list, 1, 1, "inputs.json"), list);
        let no_fields = serde_json::json!({ "Title": "no fields here" });
        assert_eq!(
            with_step_diagnostic(&no_fields, 1, 1, "odd.json"),
            no_fields
        );
    }

    /// State for driving a handler directly. The routes are thin, but the
    /// wiring they do -- parsing `?bare`, sharing one cursor, refusing an
    /// empty rotation, setting `no-store` -- is not covered by testing the
    /// functions underneath them.
    fn app_state(dir: &Path) -> AppState {
        AppState {
            dir: Arc::new(dir.to_path_buf()),
            chain: Arc::new("ethereum".to_string()),
            runtime: Arc::new(make_runtime()),
            cursor: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn two_payload_dir(label: &str) -> PathBuf {
        let dir = temp_dir(label);
        fs::write(
            dir.join("a.json"),
            r#"{"Fields":[{"Label":"L","Type":"text_v2"}],"Title":"A"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("b.json"),
            r#"{"Fields":[{"Label":"L","Type":"text_v2"}],"Title":"B"}"#,
        )
        .unwrap();
        dir
    }

    async fn next(state: &AppState, bare: bool) -> serde_json::Value {
        let (headers, Json(value)) = handle_next(State(state.clone()), Query(NextQuery { bare }))
            .await
            .expect("a payload");
        // Both walk routes move the cursor, so neither may be cached.
        assert_eq!(headers[0].0, header::CACHE_CONTROL);
        assert_eq!(headers[0].1, HeaderValue::from_static("no-store"));
        value
    }

    #[tokio::test]
    async fn next_walks_in_order_and_wraps() {
        let state = app_state(&two_payload_dir("next_walk"));
        let titles: Vec<String> = {
            let mut got = Vec::new();
            for _ in 0..3 {
                got.push(next(&state, false).await["Title"].to_string());
            }
            got
        };
        assert_eq!(titles, vec!["\"A\"", "\"B\"", "\"A\""]);
    }

    #[tokio::test]
    async fn next_prepends_the_step_diagnostic_unless_bare() {
        let state = app_state(&two_payload_dir("next_bare"));

        let decorated = next(&state, false).await;
        let first = &decorated["Fields"][0];
        assert_eq!(first["Type"], "diagnostic");
        assert_eq!(first["Diagnostic"]["Message"], "step 1 of 2: a.json");

        // ?bare=true is the same entry's neighbour, undecorated.
        let bare = next(&state, true).await;
        assert_eq!(bare["Title"], "B");
        assert_eq!(
            bare["Fields"].as_array().unwrap().len(),
            1,
            "no step field was added: {bare}"
        );
    }

    #[tokio::test]
    async fn reset_returns_the_cursor_to_the_first_entry() {
        let state = app_state(&two_payload_dir("next_reset"));
        next(&state, false).await;
        next(&state, false).await;

        let (headers, Json(ack)) = handle_reset(State(state.clone())).await;
        assert_eq!(headers[0].1, HeaderValue::from_static("no-store"));
        assert_eq!(ack, serde_json::json!({"reset": true, "next_step": 1}));

        assert_eq!(
            next(&state, false).await["Fields"][0]["Diagnostic"]["Message"],
            "step 1 of 2: a.json"
        );
    }

    #[tokio::test]
    async fn next_is_not_found_when_nothing_decodes() {
        let dir = temp_dir("next_empty");
        fs::write(dir.join("junk.hex"), "not a transaction").unwrap();
        let state = app_state(&dir);

        let err = handle_next(State(state), Query(NextQuery { bare: false }))
            .await
            .expect_err("nothing to serve");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        assert!(err.1.contains("no decodable payloads"), "got: {}", err.1);
    }

    #[test]
    fn a_file_named_for_a_walk_route_links_through_api_file() {
        // The literal routes match before the wildcard, so a root file with
        // one of those names is unreachable at /{name} and the index has to
        // link it elsewhere.
        assert_eq!(entry_href("next"), "/api/file?path=next");
        assert_eq!(entry_href("reset"), "/api/file?path=reset");
        assert_eq!(
            entry_href("sub/next"),
            "/sub/next",
            "only a root name collides"
        );
        assert_eq!(entry_href("payload.json"), "/payload.json");
    }

    #[test]
    fn index_page_advertises_the_walk_routes() {
        let entries = vec![entry("01.json", true)];
        let html = render_html(&entries);
        assert!(html.contains("href=\"/next\""), "got: {html}");
        assert!(html.contains("href=\"/reset\""), "got: {html}");
    }
}
