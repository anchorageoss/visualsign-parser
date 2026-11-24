//! Provenance records: the byte format, the derived record path, the
//! writer's validation rules, and a parser-side validator shared with the
//! tests that pin the committed records.
//!
//! Every IDL compiled into a Solana preset must carry a record next to it:
//! `<file>.json` is accompanied by `<file>.provenance.json`, which says
//! where the IDL came from. The format is pinned byte for byte to the
//! Python script that wrote the committed records (`json.dumps(record,
//! indent=2) + "\n"`, keys in the order below): the visualsign-solana test
//! `preset_idl_provenance_records_match_the_idls` checks those records,
//! and this crate's tests check that this crate can parse every one of
//! them.

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::json::{self, Json};

/// The `tool` field written into 'onchain' records.
///
/// The value is the path of the Python script that originally wrote the
/// committed records. It must not change now that this crate replaces that
/// script: every existing 'onchain' record carries it, and the
/// visualsign-solana test asserts it exactly.
pub const PROVENANCE_TOOL: &str = "scripts/fetch-solana-idl.py";

/// Length of a full git SHA, as accepted by `--commit`.
pub const FULL_SHA_LEN: usize = 40;

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Current UTC time as `%Y-%m-%dT%H:%M:%SZ`, like the Python script.
pub fn utc_now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Is `text` a full 40-character lowercase hex SHA?
pub fn is_full_sha(text: &str) -> bool {
    text.len() == FULL_SHA_LEN
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Sibling provenance path: `<file>.json` -> `<file>.provenance.json`.
pub fn record_path_for(idl: &Path) -> Result<PathBuf> {
    if idl.extension() != Some(std::ffi::OsStr::new("json")) {
        bail!("--idl must be a .json file: {}", idl.display());
    }
    Ok(idl.with_extension("provenance.json"))
}

/// Render a record: JSON, 2-space indent, trailing newline, keys in the
/// canonical order (common fields first, then per-source fields).
fn serialize(fields: Vec<(&str, Json)>) -> String {
    format!("{}\n", json::pretty(&Json::object(fields)))
}

/// 'repo' record for an IDL copied from a public repository at a known
/// commit.
pub fn repo_record(
    program_id: &str,
    idl_sha256: &str,
    recorded_at: &str,
    repo: &str,
    commit: &str,
    path: &str,
) -> String {
    serialize(vec![
        ("program_id", Json::str(program_id)),
        ("idl_sha256", Json::str(idl_sha256)),
        ("source", Json::str("repo")),
        ("recorded_at", Json::str(recorded_at)),
        ("repo", Json::str(repo)),
        ("commit", Json::str(commit)),
        ("path", Json::str(path)),
    ])
}

/// 'issuer' record for an IDL supplied out of band by a named party.
pub fn issuer_record(
    program_id: &str,
    idl_sha256: &str,
    recorded_at: &str,
    supplied_by: &str,
    notes: &str,
) -> String {
    serialize(vec![
        ("program_id", Json::str(program_id)),
        ("idl_sha256", Json::str(idl_sha256)),
        ("source", Json::str("issuer")),
        ("recorded_at", Json::str(recorded_at)),
        ("supplied_by", Json::str(supplied_by)),
        ("notes", Json::str(notes)),
    ])
}

/// 'unknown' record for an IDL whose origin cannot be established.
pub fn unknown_record(
    program_id: &str,
    idl_sha256: &str,
    recorded_at: &str,
    notes: &str,
) -> String {
    serialize(vec![
        ("program_id", Json::str(program_id)),
        ("idl_sha256", Json::str(idl_sha256)),
        ("source", Json::str("unknown")),
        ("recorded_at", Json::str(recorded_at)),
        ("notes", Json::str(notes)),
    ])
}

/// 'onchain' record for an IDL just fetched, over the exact bytes written.
pub fn onchain_record(
    program_id: &str,
    idl_sha256: &str,
    recorded_at: &str,
    idl_account: &str,
    rpc_slot: u64,
    program_last_deployed_slot: Option<u64>,
) -> String {
    serialize(vec![
        ("program_id", Json::str(program_id)),
        ("idl_sha256", Json::str(idl_sha256)),
        ("source", Json::str("onchain")),
        ("recorded_at", Json::str(recorded_at)),
        ("idl_account", Json::str(idl_account)),
        ("rpc_slot", Json::Int(rpc_slot as i64)),
        (
            "program_last_deployed_slot",
            match program_last_deployed_slot {
                Some(slot) => Json::Int(slot as i64),
                None => Json::Null,
            },
        ),
        ("tool", Json::str(PROVENANCE_TOOL)),
    ])
}

/// Is `text` a UTC ISO-8601 timestamp (`%Y-%m-%dT%H:%M:%SZ`, or the
/// `+00:00` offset form the visualsign-solana test also accepts)?
pub fn is_utc_timestamp(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text)
        .is_ok_and(|parsed| parsed.offset().local_minus_utc() == 0)
}

fn string_field<'a>(record: &'a Json, field: &str) -> Result<&'a str, String> {
    record
        .get(field)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("record has no string {field:?}"))
}

/// Validate a parsed provenance record: the writer's rules plus the shape
/// the visualsign-solana test checks. Extra fields (hand-added notes on an
/// 'onchain' record, for example) are allowed.
pub fn validate_record(record: &Json) -> Result<(), String> {
    let Json::Object(_) = record else {
        return Err("record is not a JSON object".to_string());
    };
    let program_id = string_field(record, "program_id")?;
    if Pubkey::from_str(program_id).is_err() {
        return Err(format!("invalid program id {program_id:?}"));
    }
    let digest = string_field(record, "idl_sha256")?;
    if digest.len() != 64
        || !digest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err("idl_sha256 must be a 64-character lowercase hex sha256".to_string());
    }
    let recorded_at = string_field(record, "recorded_at")?;
    if !is_utc_timestamp(recorded_at) {
        return Err(format!(
            "recorded_at {recorded_at:?} is not a UTC ISO-8601 timestamp"
        ));
    }
    match string_field(record, "source")? {
        "onchain" => {
            let idl_account = string_field(record, "idl_account")?;
            if idl_account.is_empty() {
                return Err("empty idl_account".to_string());
            }
            let rpc_slot = record
                .get("rpc_slot")
                .and_then(Json::as_u64)
                .ok_or("record has no positive integer rpc_slot")?;
            if rpc_slot == 0 {
                return Err("rpc_slot must be the slot of the getAccountInfo reply".to_string());
            }
            let last_deployed = record
                .get("program_last_deployed_slot")
                .ok_or("record has no program_last_deployed_slot")?;
            if !(last_deployed.is_null() || last_deployed.as_u64().is_some()) {
                return Err(
                    "program_last_deployed_slot must be an integer slot or null".to_string()
                );
            }
            if string_field(record, "tool")? != PROVENANCE_TOOL {
                return Err(format!("tool must name the fetch tool: {PROVENANCE_TOOL}"));
            }
            Ok(())
        }
        "repo" => {
            let repo = string_field(record, "repo")?;
            if !repo.starts_with("https://") {
                return Err(format!("repo {repo:?} must be an https URL"));
            }
            let commit = string_field(record, "commit")?;
            if !is_full_sha(commit) {
                return Err(format!(
                    "commit {commit:?} must be a full 40-character lowercase hex SHA"
                ));
            }
            if string_field(record, "path")?.is_empty() {
                return Err("empty path".to_string());
            }
            Ok(())
        }
        "issuer" => {
            if string_field(record, "supplied_by")?.is_empty() {
                return Err("empty supplied_by".to_string());
            }
            if record.get("notes").and_then(Json::as_str).is_none() {
                return Err("record has no string notes".to_string());
            }
            Ok(())
        }
        "unknown" => {
            if string_field(record, "notes")?.is_empty() {
                return Err(
                    "unknown source must say in notes why the origin can't be established"
                        .to_string(),
                );
            }
            Ok(())
        }
        source => Err(format!(
            "unknown source {source:?} (expected onchain, repo, issuer or unknown)"
        )),
    }
}

/// Read and validate the provenance record at `path`.
pub fn read_record(path: &Path) -> Result<Json> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let record = json::parse(&text).map_err(anyhow::Error::msg)?;
    validate_record(&record).map_err(anyhow::Error::msg)?;
    Ok(record)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn record_sits_next_to_the_idl() {
        assert_eq!(
            record_path_for(Path::new("presets/jupiter_swap/jupiter_agg_v6.json")).unwrap(),
            PathBuf::from("presets/jupiter_swap/jupiter_agg_v6.provenance.json")
        );
        // The stem keeps any dotted prefix; only the final .json changes.
        assert_eq!(
            record_path_for(Path::new("dir/some.program.json")).unwrap(),
            PathBuf::from("dir/some.program.provenance.json")
        );
    }

    #[test]
    fn non_json_idl_is_rejected() {
        assert!(record_path_for(Path::new("idl.txt")).is_err());
        assert!(record_path_for(Path::new("idl")).is_err());
    }

    #[test]
    fn repo_record_is_byte_identical_to_the_python_format() {
        let record = repo_record(
            "11111111111111111111111111111111",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "2026-11-06T22:08:06Z",
            "https://github.com/example/example",
            "0000000000000000000000000000000000000000",
            "idls/some_program.json",
        );
        assert_eq!(
            record,
            concat!(
                "{\n",
                "  \"program_id\": \"11111111111111111111111111111111\",\n",
                "  \"idl_sha256\": \"0000000000000000000000000000000000000000000000000000000000000000\",\n",
                "  \"source\": \"repo\",\n",
                "  \"recorded_at\": \"2026-11-06T22:08:06Z\",\n",
                "  \"repo\": \"https://github.com/example/example\",\n",
                "  \"commit\": \"0000000000000000000000000000000000000000\",\n",
                "  \"path\": \"idls/some_program.json\"\n",
                "}\n"
            )
        );
    }

    #[test]
    fn onchain_record_is_byte_identical_to_the_python_format() {
        let record = onchain_record(
            "LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF",
            "d57e19f41273f621b2033d4db2214190ffddc4fed9bfb2b6912aba473646ed94",
            "2026-11-06T22:08:06Z",
            "CEKPqaWFh121hEPxCd6vyuZRe2n3tsgQZZczuX8vmbnu",
            466_271_459,
            Some(446_205_635),
        );
        assert_eq!(
            record,
            concat!(
                "{\n",
                "  \"program_id\": \"LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF\",\n",
                "  \"idl_sha256\": \"d57e19f41273f621b2033d4db2214190ffddc4fed9bfb2b6912aba473646ed94\",\n",
                "  \"source\": \"onchain\",\n",
                "  \"recorded_at\": \"2026-11-06T22:08:06Z\",\n",
                "  \"idl_account\": \"CEKPqaWFh121hEPxCd6vyuZRe2n3tsgQZZczuX8vmbnu\",\n",
                "  \"rpc_slot\": 466271459,\n",
                "  \"program_last_deployed_slot\": 446205635,\n",
                "  \"tool\": \"scripts/fetch-solana-idl.py\"\n",
                "}\n"
            )
        );
        // A null deployment slot (non-upgradeable program or failed lookup)
        // renders as JSON null.
        let record = onchain_record(
            "LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF",
            "d57e19f41273f621b2033d4db2214190ffddc4fed9bfb2b6912aba473646ed94",
            "2026-11-06T22:08:06Z",
            "CEKPqaWFh121hEPxCd6vyuZRe2n3tsgQZZczuX8vmbnu",
            7,
            None,
        );
        assert!(record.contains("  \"program_last_deployed_slot\": null,\n"));
    }

    #[test]
    fn issuer_record_keeps_empty_notes_as_empty_string() {
        // --notes defaults to "" and lands in the record verbatim.
        let record = issuer_record(
            "11111111111111111111111111111111",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "2026-01-01T00:00:00Z",
            "Example Labs support",
            "",
        );
        assert!(record.contains("\"supplied_by\": \"Example Labs support\""));
        assert!(record.contains("\"notes\": \"\""));
    }

    #[test]
    fn unknown_record_is_byte_identical_to_the_python_format() {
        let record = unknown_record(
            "11111111111111111111111111111111",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "2026-11-06T22:07:43Z",
            "Origin not recorded; added in 817cb4b15bed (#309).",
        );
        assert_eq!(
            record,
            concat!(
                "{\n",
                "  \"program_id\": \"11111111111111111111111111111111\",\n",
                "  \"idl_sha256\": \"1111111111111111111111111111111111111111111111111111111111111111\",\n",
                "  \"source\": \"unknown\",\n",
                "  \"recorded_at\": \"2026-11-06T22:07:43Z\",\n",
                "  \"notes\": \"Origin not recorded; added in 817cb4b15bed (#309).\"\n",
                "}\n"
            )
        );
    }

    #[test]
    fn every_committed_preset_record_parses_and_validates() {
        // Format drift between this tool and the records already committed
        // under the Solana presets must fail CI: the drift test in
        // visualsign-solana parses them with serde_json, this one with the
        // tool's own parser and the writer's validation rules.
        let presets = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../chain_parsers/visualsign-solana/src/presets");
        let mut records = 0;
        let mut checked = Vec::new();
        walk_for_records(&presets, &mut checked).expect("presets directory walk");
        for path in &checked {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            let parsed = json::parse(&text)
                .unwrap_or_else(|e| panic!("record {} does not parse: {e}", path.display()));
            // serde_json must agree the file is JSON, and on every value.
            let serde_parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
                panic!("record {} is not serde_json JSON: {e}", path.display())
            });
            assert_json_agrees(&parsed, &serde_parsed, &path.display().to_string());
            validate_record(&parsed)
                .unwrap_or_else(|e| panic!("record {} fails validation: {e}", path.display()));
            // 2-space indent and a trailing newline, with no reordering
            // when re-rendered from the parsed value.
            assert_eq!(
                text,
                format!("{}\n", json::pretty(&parsed)),
                "record {} is not in the canonical 2-space form",
                path.display()
            );
            records += 1;
        }
        assert!(
            records >= 20,
            "expected every committed record to be checked"
        );
    }

    fn walk_for_records(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("failed to list {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("failed to list {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                walk_for_records(&path, out)?;
            } else if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".provenance.json")
            {
                out.push(path);
            }
        }
        Ok(())
    }

    /// The tool's parser and serde_json must agree on every value in the
    /// record, so a drift in either direction is caught.
    fn assert_json_agrees(parsed: &Json, serde_value: &serde_json::Value, where_: &str) {
        match (parsed, serde_value) {
            (Json::Null, serde_json::Value::Null) => {}
            (Json::Bool(a), serde_json::Value::Bool(b)) => assert_eq!(a, b, "{where_}"),
            (Json::Int(a), serde_json::Value::Number(b)) => {
                assert_eq!(b.as_i64(), Some(*a), "{where_}")
            }
            (Json::Str(a), serde_json::Value::String(b)) => assert_eq!(a, b, "{where_}"),
            (Json::Array(a), serde_json::Value::Array(b)) => {
                assert_eq!(a.len(), b.len(), "{where_}");
                for (x, y) in a.iter().zip(b) {
                    assert_json_agrees(x, y, where_);
                }
            }
            (Json::Object(a), serde_json::Value::Object(b)) => {
                assert_eq!(a.len(), b.len(), "{where_}");
                for (key, x) in a {
                    let y = b
                        .get(key)
                        .unwrap_or_else(|| panic!("{where_}: key {key:?}"));
                    assert_json_agrees(x, y, where_);
                }
            }
            _ => panic!("{where_}: value kinds disagree"),
        }
    }
}
