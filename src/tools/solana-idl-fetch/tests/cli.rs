//! End-to-end tests for the record modes (repo / issuer / unknown) and
//! their validation, run against the real binary exactly the way the
//! Python tests ran the script. Everything here runs without network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use solana_idl_fetch::json::{self, Json};
use solana_idl_fetch::record::sha256_hex;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// The System program id: valid base58, decodes to 32 zero bytes.
const PROGRAM_ID: &str = "11111111111111111111111111111111";
const IDL_JSON: &str = "{\"address\": \"11111111111111111111111111111111\", \"instructions\": [{\"name\": \"noop\"}]}\n";

fn run_tool(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_solana-idl-fetch"))
        .args(args)
        .output()
        .expect("failed to run the solana-idl-fetch binary")
}

struct Fixture {
    _tmp: TempDir,
    idl: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let idl = tmp.path().join("some_program.json");
        std::fs::write(&idl, IDL_JSON).expect("write the idl fixture");
        Fixture { _tmp: tmp, idl }
    }

    fn record(&self) -> PathBuf {
        self.idl
            .parent()
            .unwrap()
            .join("some_program.provenance.json")
    }

    fn idl_arg(&self) -> String {
        self.idl.to_string_lossy().into_owned()
    }
}

/// The fields every record shares, plus the canonical 2-space form and
/// the per-source key order -- the Python tests' `assert_common_fields`.
fn assert_common_fields(record: &Json, source: &str, raw: &str) {
    let expected_order: &[&str] = match source {
        "onchain" => &[
            "program_id",
            "idl_sha256",
            "source",
            "recorded_at",
            "idl_account",
            "rpc_slot",
            "program_last_deployed_slot",
            "tool",
        ],
        "repo" => &[
            "program_id",
            "idl_sha256",
            "source",
            "recorded_at",
            "repo",
            "commit",
            "path",
        ],
        "issuer" => &[
            "program_id",
            "idl_sha256",
            "source",
            "recorded_at",
            "supplied_by",
            "notes",
        ],
        "unknown" => &["program_id", "idl_sha256", "source", "recorded_at", "notes"],
        other => panic!("unexpected source {other:?}"),
    };
    let order: Vec<&str> = match record {
        Json::Object(fields) => fields.iter().map(|(key, _)| key.as_str()).collect(),
        other => panic!("record is not an object: {other:?}"),
    };
    assert_eq!(order, expected_order);
    assert_eq!(record.get("source").unwrap().as_str().unwrap(), source);
    assert_eq!(
        record.get("program_id").unwrap().as_str().unwrap(),
        PROGRAM_ID
    );
    assert_eq!(
        record.get("idl_sha256").unwrap().as_str().unwrap(),
        sha256_hex(IDL_JSON.as_bytes())
    );
    let recorded_at = record.get("recorded_at").unwrap().as_str().unwrap();
    assert!(
        solana_idl_fetch::record::is_utc_timestamp(recorded_at),
        "recorded_at {recorded_at:?} is not a UTC timestamp"
    );
    // Canonical form: 2-space indent, trailing newline, no reordering.
    assert_eq!(raw, format!("{}\n", json::pretty(record)));
}

#[test]
fn record_repo_writes_a_valid_record() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-repo",
        "https://github.com/example/example",
        "--commit",
        &"0".repeat(40),
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert!(result.status.success(), "{}", result.stderr_lossy());
    let raw = std::fs::read_to_string(fixture.record()).expect("record written");
    let record = json::parse(&raw).unwrap();
    assert_common_fields(&record, "repo", &raw);
    assert_eq!(
        record.get("repo").unwrap().as_str().unwrap(),
        "https://github.com/example/example"
    );
    assert_eq!(
        record.get("commit").unwrap().as_str().unwrap(),
        "0".repeat(40)
    );
    assert_eq!(
        record.get("path").unwrap().as_str().unwrap(),
        "idls/some_program.json"
    );
    // Success says where the record landed, on stderr.
    assert!(
        result
            .stderr
            .starts_with(format!("provenance record: {}", fixture.record().display()).as_bytes()),
        "{}",
        result.stderr_lossy()
    );
}

#[test]
fn short_commit_is_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-repo",
        "https://github.com/example/example",
        "--commit",
        &"0".repeat(39),
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--commit"));
    assert!(!fixture.record().exists());
}

#[test]
fn uppercase_commit_is_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-repo",
        "https://github.com/example/example",
        "--commit",
        &format!("{}A", "0".repeat(39)),
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--commit"));
    assert!(!fixture.record().exists());
}

#[test]
fn http_url_is_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-repo",
        "http://github.com/example/example",
        "--commit",
        &"0".repeat(40),
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("https"));
    assert!(!fixture.record().exists());
}

#[test]
fn missing_commit_is_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-repo",
        "https://github.com/example/example",
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
}

#[test]
fn record_issuer_writes_a_valid_record() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-issuer",
        "Example Labs support",
        "--notes",
        "attached to an email thread",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert!(result.status.success(), "{}", result.stderr_lossy());
    let raw = std::fs::read_to_string(fixture.record()).expect("record written");
    let record = json::parse(&raw).unwrap();
    assert_common_fields(&record, "issuer", &raw);
    assert_eq!(
        record.get("supplied_by").unwrap().as_str().unwrap(),
        "Example Labs support"
    );
    assert_eq!(
        record.get("notes").unwrap().as_str().unwrap(),
        "attached to an email thread"
    );
}

#[test]
fn record_unknown_writes_a_valid_record() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-unknown",
        "origin predates the provenance policy",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert!(result.status.success(), "{}", result.stderr_lossy());
    let raw = std::fs::read_to_string(fixture.record()).expect("record written");
    let record = json::parse(&raw).unwrap();
    assert_common_fields(&record, "unknown", &raw);
    assert_eq!(
        record.get("notes").unwrap().as_str().unwrap(),
        "origin predates the provenance policy"
    );
}

#[test]
fn empty_unknown_notes_are_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-unknown",
        "",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
}

#[test]
fn multiple_record_modes_are_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-unknown",
        "x",
        "--record-issuer",
        "y",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
}

#[test]
fn missing_idl_is_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("nope.json");
    let result = run_tool(&[
        "--record-unknown",
        "x",
        "--idl",
        &missing.to_string_lossy(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("not found"),
        "{}",
        result.stderr_lossy()
    );
}

#[test]
fn bad_program_id_is_rejected() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        "--record-unknown",
        "x",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        "not-base58!",
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
}

#[test]
fn positional_program_id_is_rejected_in_record_mode() {
    let fixture = Fixture::new();
    let result = run_tool(&[
        PROGRAM_ID,
        "--record-unknown",
        "x",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("positional argument"),
        "{}",
        result.stderr_lossy()
    );
    assert!(!fixture.record().exists());
}

#[test]
fn fetch_flags_are_rejected_in_record_mode() {
    let fixture = Fixture::new();
    for flag in ["--out", "--compare", "--provenance"] {
        let result = run_tool(&[
            "--record-unknown",
            "x",
            "--idl",
            &fixture.idl_arg(),
            "--program-id",
            PROGRAM_ID,
            flag,
            "some/path.json",
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{flag} should have been rejected: {}",
            result.stderr_lossy()
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(flag),
            "{}",
            result.stderr_lossy()
        );
        assert!(!fixture.record().exists());
    }
}

#[test]
fn commit_path_and_notes_flags_need_their_modes() {
    let fixture = Fixture::new();
    // --commit / --path without --record-repo.
    let result = run_tool(&[
        "--record-unknown",
        "x",
        "--commit",
        &"0".repeat(40),
        "--path",
        "idls/some_program.json",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
    // --notes without --record-issuer.
    let result = run_tool(&[
        "--record-unknown",
        "x",
        "--notes",
        "n",
        "--idl",
        &fixture.idl_arg(),
        "--program-id",
        PROGRAM_ID,
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.record().exists());
}

#[test]
fn missing_program_id_without_record_mode_exits_2() {
    // argparse treated this as a usage error: exit code 2.
    let result = run_tool(&[]);
    assert_eq!(result.status.code(), Some(2));
}

/// Lossy stderr, for assertion messages.
trait OutputExt {
    fn stderr_lossy(&self) -> String;
}

impl OutputExt for Output {
    fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}
