//! The `replay` subcommand: decode recent successful mainnet transactions
//! of one program through `parser_cli` built before and after an IDL
//! change, and render one markdown section per transaction.
//!
//! The RPC surface ([`ReplayRpc`]) and the parser runs ([`ParserRunner`])
//! are traits, so the tests replay with a mocked RPC and fake parsers --
//! no network, no binary. The real implementations are [`HttpRpc`] and
//! [`BinaryParserRunner`].
//!
//! Security notes:
//!
//! * The RPC URL carries the API key (`SOLANA_RPC_URL` is built from
//!   `HELIUS_API_KEY` in the workflow): it is never printed, and every
//!   error leaving [`HttpRpc`] is redacted against it.
//! * Text derived from transactions (instruction titles, condensed rows,
//!   parse errors) is untrusted: it only ever appears inside fenced code
//!   blocks, with backtick runs neutralized (see [`crate::sanitize`]).

use anyhow::{Context, Result, anyhow, bail};
use solana_idl_fetch::json::{self, Json};
use solana_idl_fetch::solana::parse_program_id;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::sanitize;

/// The network parser_cli decodes against.
pub const NETWORK: &str = "SOLANA_MAINNET";

/// The env var the RPC URL is read from. It carries the API key and is
/// never printed.
pub const RPC_URL_ENV: &str = "SOLANA_RPC_URL";

/// What one replay run did; also the `replay` subcommand's stdout JSON.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayCounts {
    pub replayed: usize,
    pub unchanged: usize,
    pub changed: usize,
    pub errors: usize,
    /// Version 1 (SIMD-0385) transactions: the parser does not support
    /// them yet (issue #662), so they fail on both sides; counted
    /// separately from other parse errors.
    pub v1_failures: usize,
}

/// One getSignaturesForAddress entry, as far as replay cares.
#[derive(Clone, Debug)]
pub struct TxStatus {
    pub signature: String,
    pub failed: bool,
}

/// One getTransaction reply.
#[derive(Clone, Debug)]
pub struct TxReply {
    /// Transaction version as the RPC reports it ("legacy", "0", "1").
    pub version: String,
    /// The wire transaction, base64, ready for parser_cli's `-t`.
    pub tx_base64: String,
}

/// The RPC surface replay needs, behind a trait so tests need no network.
pub trait ReplayRpc {
    /// getSignaturesForAddress: the most recent `limit` signatures of
    /// `program_id`, newest first, with their success flag.
    fn signatures_for_address(&self, program_id: &str, limit: u64) -> Result<Vec<TxStatus>>;
    /// getTransaction (base64, maxSupportedTransactionVersion 1), or
    /// `None` when the transaction is no longer available.
    fn transaction(&self, signature: &str) -> Result<Option<TxReply>>;
}

/// Blocking HTTPS JSON-RPC client with a 60-second timeout per request.
/// The URL carries an API key and is never printed: every error leaving
/// this type is redacted against it ([`crate::sanitize::redact`]).
pub struct HttpRpc {
    url: String,
    client: reqwest::blocking::Client,
}

impl HttpRpc {
    pub fn new(url: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("failed to build the RPC client")?;
        Ok(Self {
            url: url.to_string(),
            client,
        })
    }

    /// From `SOLANA_RPC_URL`; an error if it is unset or empty. The error
    /// never echoes the URL.
    pub fn from_env() -> Result<Self> {
        let url = std::env::var(RPC_URL_ENV).map_err(|_| anyhow!("{RPC_URL_ENV} is not set"))?;
        if url.trim().is_empty() {
            bail!("{RPC_URL_ENV} is empty");
        }
        Self::new(&url)
    }

    /// One JSON-RPC call. Errors are redacted against the URL first: the
    /// URL carries the API key and reqwest embeds it in transport errors.
    fn call(&self, method: &str, params: Json) -> Result<Json> {
        let request = Json::object(vec![
            ("jsonrpc", Json::str("2.0")),
            ("id", Json::Int(1)),
            ("method", Json::str(method)),
            ("params", params),
        ]);
        let send = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json::canonical(&request))
            .send()
            .and_then(|response| response.text())
            .context("RPC request failed")
            .map_err(|err| self.scrub(err));
        let reply_text = send?;
        let reply = json::parse(&reply_text).map_err(anyhow::Error::msg)?;
        if let Some(error) = reply.get("error") {
            return Err(self.scrub(anyhow!("RPC error: {}", json::canonical(error))));
        }
        let result = reply
            .get("result")
            .ok_or_else(|| self.scrub(anyhow!("RPC reply has no 'result'")));
        Ok(result?.clone())
    }

    /// Redact the URL out of any error, so the API key can never leak
    /// through a transport or decode failure.
    fn scrub(&self, err: anyhow::Error) -> anyhow::Error {
        anyhow!("{}", sanitize::redact(&format!("{err:#}"), &self.url))
    }
}

impl ReplayRpc for HttpRpc {
    fn signatures_for_address(&self, program_id: &str, limit: u64) -> Result<Vec<TxStatus>> {
        let params = Json::Array(vec![
            Json::str(program_id),
            Json::object(vec![(
                "limit",
                Json::Int(i64::try_from(limit).context("signature limit too large")?),
            )]),
        ]);
        let result = self.call("getSignaturesForAddress", params)?;
        let entries = result
            .as_array()
            .ok_or_else(|| self.scrub(anyhow!("getSignaturesForAddress reply is not an array")))?;
        let mut statuses: Vec<TxStatus> = Vec::new();
        for entry in entries {
            let signature = entry
                .get("signature")
                .and_then(Json::as_str)
                .ok_or_else(|| self.scrub(anyhow!("signature entry has no string 'signature'")))?
                .to_string();
            // `err` is null for a successful transaction; absent is treated
            // as success, the way the RPC has always reported it.
            let failed = entry.get("err").is_some_and(|err| !err.is_null());
            statuses.push(TxStatus { signature, failed });
        }
        Ok(statuses)
    }

    fn transaction(&self, signature: &str) -> Result<Option<TxReply>> {
        let params = Json::Array(vec![
            Json::str(signature),
            Json::object(vec![
                ("encoding", Json::str("base64")),
                ("maxSupportedTransactionVersion", Json::Int(1)),
                ("commitment", Json::str("finalized")),
            ]),
        ]);
        let result = self.call("getTransaction", params)?;
        if result.is_null() {
            return Ok(None);
        }
        let version = tx_version(result.get("version"));
        let tx_base64 = result
            .get("transaction")
            .and_then(|tx| tx.at(0))
            .and_then(Json::as_str)
            .ok_or_else(|| {
                self.scrub(anyhow!(
                    "getTransaction reply has no base64 'transaction[0]'"
                ))
            })?
            .to_string();
        Ok(Some(TxReply { version, tx_base64 }))
    }
}

/// Render getTransaction's `version` the way replay reports it.
/// maxSupportedTransactionVersion 1 makes the RPC report it either as
/// the JSON string `"legacy"` or as the JSON number `0` or `1` (absent
/// on very old transactions): `legacy`, `0`, `1` respectively, anything
/// absent or unexpected `unknown`. Numbers must never default to
/// `legacy` -- a version 1 transaction reported as `legacy` would hide
/// SIMD-0385 failures (issue #662) as ordinary parse errors.
pub fn tx_version(version: Option<&Json>) -> String {
    match version {
        Some(Json::Str(text)) if text == "legacy" => "legacy".to_string(),
        Some(Json::Int(0)) => "0".to_string(),
        Some(Json::Int(1)) => "1".to_string(),
        _ => "unknown".to_string(),
    }
}

/// A parse failure on both sides is a version 1 (SIMD-0385) failure
/// (issue #662) when the RPC reports the transaction as version 1, or
/// when either parser build's error names SIMD-0385 -- parser_cli's v1
/// rejections all do, whatever shape they surface in (transaction
/// format v1, message format v1, over the packet limit, truncated in a
/// packet).
fn is_v1_failure(version: &str, before_err: &str, after_err: &str) -> bool {
    version == "1" || before_err.contains("SIMD-0385") || after_err.contains("SIMD-0385")
}

/// One parser_cli build, behind a trait so tests need no binary.
pub trait ParserRunner {
    /// Decode one base64 wire transaction into the CLI's JSON payload.
    /// The `Err` text is the failure shown in the report.
    fn decode(&self, tx_base64: &str) -> std::result::Result<String, String>;
}

/// Runs a parser_cli binary with
/// `decode --chain solana --network SOLANA_MAINNET -o json -t <base64 tx>`.
///
/// parser_cli writes `visualsign-parser-cli.log` into its working
/// directory, so every decode runs in a throwaway temp directory.
pub struct BinaryParserRunner {
    binary: PathBuf,
}

impl BinaryParserRunner {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// The binary to spawn: an absolute path as given, a relative one
    /// resolved against the current directory (a bare name would go
    /// through `$PATH` search instead, which is not what `--before
    /// before-parser_cli` from the workflow means).
    fn command_path(&self) -> PathBuf {
        if self.binary.is_absolute() {
            return self.binary.clone();
        }
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&self.binary),
            Err(_) => Path::new(".").join(&self.binary),
        }
    }
}

impl ParserRunner for BinaryParserRunner {
    fn decode(&self, tx_base64: &str) -> std::result::Result<String, String> {
        let dir = tempfile::tempdir()
            .map_err(|e| format!("failed to create a temp working dir for parser_cli: {e}"))?;
        let output = Command::new(self.command_path())
            .current_dir(dir.path())
            .args([
                "decode",
                "--chain",
                "solana",
                "--network",
                NETWORK,
                "-o",
                "json",
                "-t",
                tx_base64,
            ])
            .output()
            .map_err(|e| format!("failed to run {}: {e}", self.binary.display()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "parser_cli exited with {}: {}",
                output.status,
                stderr.trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|e| format!("parser_cli stdout is not UTF-8: {e}"))
    }
}

/// Replay up to `limit` recent successful mainnet transactions of
/// `program_id` through both parser builds, and render the markdown
/// section (plus the counts, which also reach stdout as JSON).
pub fn replay_section(
    rpc: &dyn ReplayRpc,
    program_id: &str,
    before: &dyn ParserRunner,
    after: &dyn ParserRunner,
    limit: usize,
) -> Result<(String, ReplayCounts)> {
    parse_program_id(program_id).context("invalid --program-id")?;
    if limit == 0 {
        bail!("--limit must be at least 1");
    }
    // One page, generous enough to skip failed transactions and still
    // reach `limit` successful ones.
    let signature_limit = (limit * 5).clamp(25, 100) as u64;
    let statuses = rpc.signatures_for_address(program_id, signature_limit)?;
    let successful: Vec<&TxStatus> = statuses
        .iter()
        .filter(|status| !status.failed)
        .take(limit)
        .collect();

    let mut counts = ReplayCounts::default();
    let mut body = String::new();
    for status in &successful {
        counts.replayed += 1;
        let Some(tx) = rpc.transaction(&status.signature)? else {
            counts.errors += 1;
            body.push_str(&render_transaction_unavailable(&status.signature));
            continue;
        };
        let before_side = run_side(before, &tx.tx_base64, program_id);
        let after_side = run_side(after, &tx.tx_base64, program_id);
        match (&before_side, &after_side) {
            (Side::View(before_view), Side::View(after_view)) => {
                if before_view == after_view {
                    counts.unchanged += 1;
                    body.push_str(&render_unchanged(&status.signature, &tx.version));
                } else {
                    counts.changed += 1;
                    body.push_str(&render_changed(
                        &status.signature,
                        &tx.version,
                        before_view,
                        after_view,
                    ));
                }
            }
            (Side::Error(before_err), Side::Error(after_err)) => {
                if is_v1_failure(&tx.version, before_err, after_err) {
                    // Version 1 (SIMD-0385) transactions fail on both
                    // sides today: the parser does not support them yet
                    // (issue #662). Report and count them as such rather
                    // than as ordinary parse errors.
                    counts.v1_failures += 1;
                    body.push_str(&render_v1_failure(&status.signature, &tx.version));
                } else {
                    counts.errors += 1;
                    body.push_str(&render_parse_error_both(
                        &status.signature,
                        &tx.version,
                        before_err,
                        after_err,
                    ));
                }
            }
            (before_side, after_side) => {
                // One side parsed and the other did not: still a
                // difference to show, counted with the errors.
                counts.errors += 1;
                body.push_str(&render_changed(
                    &status.signature,
                    &tx.version,
                    &before_side.render_text(),
                    &after_side.render_text(),
                ));
            }
        }
    }

    let mut section = String::new();
    if successful.is_empty() {
        section.push_str(&format!(
            "No successful mainnet transactions of `{program_id}` among the last \
             {signature_limit} signatures; nothing to replay.\n"
        ));
    } else {
        section.push_str(&format!(
            "Replay of up to {limit} recent successful mainnet transactions of \
             `{program_id}`: {replayed} replayed, {unchanged} unchanged, {changed} changed, \
             {errors} parse errors, {v1_failures} v1 (SIMD-0385) parse failures (issue #662).\n",
            replayed = counts.replayed,
            unchanged = counts.unchanged,
            changed = counts.changed,
            errors = counts.errors,
            v1_failures = counts.v1_failures,
        ));
    }
    if !body.is_empty() {
        section.push('\n');
        section.push_str(&body);
    }
    Ok((section, counts))
}

/// One side (before or after) of a replayed transaction.
enum Side {
    View(String),
    Error(String),
}

impl Side {
    /// How this side lands in a before/after block.
    fn render_text(&self) -> String {
        match self {
            Side::View(view) => view.clone(),
            Side::Error(err) => format!("parse error: {err}"),
        }
    }
}

fn run_side(runner: &dyn ParserRunner, tx_base64: &str, program_id: &str) -> Side {
    match runner.decode(tx_base64) {
        Err(err) => Side::Error(err),
        Ok(payload) => match program_view(&payload, program_id) {
            Ok(view) => Side::View(view),
            Err(err) => Side::Error(format!("parser_cli output is not the expected JSON: {err}")),
        },
    }
}

/// The `replay` subcommand's stdout JSON for the counts.
pub fn counts_json(counts: &ReplayCounts) -> String {
    json::canonical(&Json::object(vec![
        ("replayed", Json::Int(counts.replayed as i64)),
        ("unchanged", Json::Int(counts.unchanged as i64)),
        ("changed", Json::Int(counts.changed as i64)),
        ("errors", Json::Int(counts.errors as i64)),
        ("v1_failures", Json::Int(counts.v1_failures as i64)),
    ]))
}

/// Run one replay over RPC + binaries and write `replay-<program_id>.md`
/// into `out_dir`; the counts go to stdout as JSON.
pub fn replay_cli(
    program_id: &str,
    before: &Path,
    after: &Path,
    limit: usize,
    out_dir: &Path,
) -> Result<()> {
    let rpc = HttpRpc::from_env()?;
    let before = BinaryParserRunner::new(before);
    let after = BinaryParserRunner::new(after);
    let (section, counts) = replay_section(&rpc, program_id, &before, &after, limit)?;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let path = out_dir.join(format!("replay-{program_id}.md"));
    std::fs::write(&path, section.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("{}", counts_json(&counts));
    Ok(())
}

/// The program's instructions as parser_cli rendered them: one line per
/// instruction (label + preview title), each condensed row indented below.
fn program_view(payload_text: &str, program_id: &str) -> Result<String> {
    let payload = json::parse(payload_text).map_err(anyhow::Error::msg)?;
    let fields = payload
        .get("Fields")
        .and_then(Json::as_array)
        .ok_or_else(|| anyhow!("no 'Fields' array"))?;
    let prefix = format!("Program ID: {program_id}");
    let mut out = String::new();
    for field in fields {
        if field.get("Type").and_then(Json::as_str) != Some("preview_layout") {
            continue;
        }
        let fallback = field
            .get("FallbackText")
            .and_then(Json::as_str)
            .unwrap_or("");
        if !fallback.starts_with(&prefix) {
            continue;
        }
        let label = field.get("Label").and_then(Json::as_str).unwrap_or("");
        let title = field
            .get("PreviewLayout")
            .and_then(|layout| layout.get("Title"))
            .and_then(|title| title.get("Text"))
            .and_then(Json::as_str)
            .unwrap_or("");
        if !out.is_empty() {
            out.push('\n');
        }
        if title.is_empty() {
            out.push_str(label);
        } else {
            out.push_str(&format!("{label}: {title}"));
        }
        if let Some(rows) = condensed_rows(field) {
            for row in rows {
                out.push_str(&format!("\n  {row}"));
            }
        }
    }
    Ok(out)
}

/// `PreviewLayout.Condensed.Fields[]` as `Label: value` rows (value alone
/// when the label is empty).
fn condensed_rows(field: &Json) -> Option<Vec<String>> {
    let rows = field
        .get("PreviewLayout")?
        .get("Condensed")?
        .get("Fields")?
        .as_array()?;
    Some(
        rows.iter()
            .map(|row| {
                let label = row.get("Label").and_then(Json::as_str).unwrap_or("");
                let value = field_value(row);
                if label.is_empty() {
                    value
                } else {
                    format!("{label}: {value}")
                }
            })
            .collect(),
    )
}

/// The payload value of one condensed row, whatever its field type.
fn field_value(field: &Json) -> String {
    let get = |outer: &str, inner: &str| -> Option<String> {
        field
            .get(outer)
            .and_then(|value| value.get(inner))
            .and_then(Json::as_str)
            .map(str::to_string)
    };
    match field.get("Type").and_then(Json::as_str).unwrap_or("") {
        "text" => get("Text", "Text"),
        "text_v2" => get("TextV2", "Text"),
        "address" => get("Address", "Address"),
        "address_v2" => get("AddressV2", "Address"),
        "amount" => get("Amount", "Amount"),
        "amount_v2" => get("AmountV2", "Amount"),
        "number" => get("Number", "Number"),
        _ => field
            .get("FallbackText")
            .and_then(Json::as_str)
            .map(str::to_string),
    }
    .unwrap_or_default()
}

/// A base58 signature (or any text) safe for a markdown heading: valid
/// base58 links to solscan, anything else is shown neutralized without a
/// link, so nothing untrusted can reach the link target.
fn signature_heading(signature: &str) -> String {
    if is_base58(signature) {
        format!("#### [`{signature}`](https://solscan.io/tx/{signature})")
    } else {
        format!("#### `{}`", sanitize::neutralize_backticks(signature))
    }
}

fn is_base58(text: &str) -> bool {
    (1..=88).contains(&text.len())
        && text
            .bytes()
            .all(|b| matches!(b, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

fn version_line(version: &str) -> String {
    format!("version: `{}`", sanitize::neutralize_backticks(version))
}

fn render_unchanged(signature: &str, version: &str) -> String {
    format!(
        "{}\n\n{}\n\nunchanged\n\n",
        signature_heading(signature),
        version_line(version)
    )
}

fn render_changed(signature: &str, version: &str, before: &str, after: &str) -> String {
    format!(
        "{}\n\n{}\n\nbefore:\n\n{}\n\nafter:\n\n{}\n\n",
        signature_heading(signature),
        version_line(version),
        sanitize::fenced(before),
        sanitize::fenced(after)
    )
}

fn render_parse_error_both(
    signature: &str,
    version: &str,
    before_err: &str,
    after_err: &str,
) -> String {
    let detail = format!("before: {before_err}\nafter: {after_err}");
    format!(
        "{}\n\n{}\n\nparse error (before and after):\n\n{}\n\n",
        signature_heading(signature),
        version_line(version),
        sanitize::fenced(&detail)
    )
}

fn render_v1_failure(signature: &str, version: &str) -> String {
    format!(
        "{}\n\n{}\n\nv1 (SIMD-0385) parse failure: version 1 transactions are not \
         supported by the parser yet (tracked in issue #662); this transaction \
         fails to decode identically before and after.\n\n",
        signature_heading(signature),
        version_line(version)
    )
}

fn render_transaction_unavailable(signature: &str) -> String {
    format!(
        "{}\n\ntransaction no longer available via RPC; not replayed\n\n",
        signature_heading(signature)
    )
}
