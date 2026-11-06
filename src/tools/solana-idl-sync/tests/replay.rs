//! End-to-end tests for the `replay` path: a mocked RPC, fake parser
//! runners, and the markdown they must produce. No network, no binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use solana_idl_fetch::json::{self, Json};
use solana_idl_sync::replay::{
    self, HttpRpc, ParserRunner, ReplayCounts, ReplayRpc, TxReply, TxStatus,
};
use solana_idl_sync::sanitize;
use std::collections::BTreeMap;

/// A preset program id (from the committed presets, so the test stays
/// honest about what a real one looks like).
const PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

/// Mocked RPC: fixed signature list, one reply per signature.
struct MockRpc {
    statuses: Vec<TxStatus>,
    transactions: BTreeMap<String, TxReply>,
}

impl MockRpc {
    fn new() -> Self {
        MockRpc {
            statuses: Vec::new(),
            transactions: BTreeMap::new(),
        }
    }

    fn put_signature(&mut self, sig: &str, failed: bool) {
        self.statuses.push(TxStatus {
            signature: sig.to_string(),
            failed,
        });
    }

    fn put_transaction(&mut self, sig: &str, version: &str, tx_base64: &str) {
        self.transactions.insert(
            sig.to_string(),
            TxReply {
                version: version.to_string(),
                tx_base64: tx_base64.to_string(),
            },
        );
    }
}

impl ReplayRpc for MockRpc {
    fn signatures_for_address(
        &self,
        _program_id: &str,
        limit: u64,
    ) -> anyhow::Result<Vec<TxStatus>> {
        Ok(self
            .statuses
            .iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .cloned()
            .collect())
    }

    fn transaction(&self, signature: &str) -> anyhow::Result<Option<TxReply>> {
        Ok(self.transactions.get(signature).cloned())
    }
}

/// A fake parser build: every transaction decodes to the same payload, or
/// fails with a fixed error.
struct FakeRunner {
    payload: Option<String>,
}

impl FakeRunner {
    fn ok(payload: String) -> Self {
        FakeRunner {
            payload: Some(payload),
        }
    }

    fn failing() -> Self {
        FakeRunner { payload: None }
    }
}

impl ParserRunner for FakeRunner {
    fn decode(&self, _tx_base64: &str) -> Result<String, String> {
        match &self.payload {
            Some(payload) => Ok(payload.clone()),
            None => Err("unsupported transaction layout".to_string()),
        }
    }
}

/// A parser_cli-shaped payload: one `preview_layout` field for
/// `PROGRAM_ID` with a title and condensed rows, plus a foreign program's
/// field that must not leak into the before/after view.
fn payload(title: &str, rows: &[(&str, &str)]) -> String {
    let row_fields: Vec<Json> = rows
        .iter()
        .map(|(label, text)| {
            Json::object(vec![
                ("Type", Json::str("text_v2")),
                ("Label", Json::str(label)),
                ("FallbackText", Json::str(text)),
                ("TextV2", Json::object(vec![("Text", Json::str(text))])),
            ])
        })
        .collect();
    let program_field = Json::object(vec![
        ("Type", Json::str("preview_layout")),
        ("Label", Json::str("Instruction 1")),
        (
            "FallbackText",
            Json::str(&format!("Program ID: {PROGRAM_ID}\nData: deadbeef")),
        ),
        (
            "PreviewLayout",
            Json::object(vec![
                ("Title", Json::object(vec![("Text", Json::str(title))])),
                (
                    "Condensed",
                    Json::object(vec![("Fields", Json::Array(row_fields))]),
                ),
            ]),
        ),
    ]);
    let foreign_field = Json::object(vec![
        ("Type", Json::str("preview_layout")),
        ("Label", Json::str("Instruction 2")),
        (
            "FallbackText",
            Json::str("Program ID: TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
        ),
        (
            "PreviewLayout",
            Json::object(vec![(
                "Title",
                Json::object(vec![("Text", Json::str("foreign title"))]),
            )]),
        ),
    ]);
    json::canonical(&Json::object(vec![
        ("Title", Json::str("Transaction")),
        ("Fields", Json::Array(vec![program_field, foreign_field])),
    ]))
}

fn run(
    rpc: &MockRpc,
    before: &FakeRunner,
    after: &FakeRunner,
    limit: usize,
) -> (String, ReplayCounts) {
    replay::replay_section(rpc, PROGRAM_ID, before, after, limit).unwrap()
}

fn one_tx(version: &str) -> MockRpc {
    let mut rpc = MockRpc::new();
    rpc.put_signature(
        "5Sig1111111111111111111111111111111111111111111111111111111",
        false,
    );
    rpc.put_transaction(
        "5Sig1111111111111111111111111111111111111111111111111111111",
        version,
        "QUJDRA==",
    );
    rpc
}

#[test]
fn identical_renderings_are_reported_as_unchanged() {
    let rpc = one_tx("v0");
    let payload = payload("Jupiter Swap", &[("Instruction", "Swap 1.5 USDC")]);
    let before = FakeRunner::ok(payload.clone());
    let after = FakeRunner::ok(payload);
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(
        counts,
        ReplayCounts {
            replayed: 1,
            unchanged: 1,
            changed: 0,
            errors: 0,
            v1_failures: 0,
        }
    );
    assert!(
        section.contains(
            "[`5Sig1111111111111111111111111111111111111111111111111111111`](https://solscan.io/tx/"
        ),
        "the signature must link to solscan: {section}"
    );
    assert!(section.contains("version: `v0`"));
    assert!(section.contains("unchanged"));
    assert!(!section.contains("before:"), "no diff expected: {section}");
}

#[test]
fn a_difference_shows_before_and_after_of_the_program_view_only() {
    let rpc = one_tx("legacy");
    let before = FakeRunner::ok(payload("Jupiter Swap", &[("Instruction", "Swap 1.5 USDC")]));
    let after = FakeRunner::ok(payload(
        "Jupiter Swap v2",
        &[("Instruction", "Swap 1.6 USDC")],
    ));
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(counts.changed, 1);
    assert!(section.contains(
        "before:\n\n```\nInstruction 1: Jupiter Swap\n  Instruction: Swap 1.5 USDC\n```"
    ));
    assert!(section.contains(
        "after:\n\n```\nInstruction 1: Jupiter Swap v2\n  Instruction: Swap 1.6 USDC\n```"
    ));
    // The other program's instruction is not part of this program's view.
    assert!(!section.contains("foreign title"));
}

#[test]
fn failed_signatures_are_skipped_and_limit_is_respected() {
    let mut rpc = MockRpc::new();
    rpc.put_signature(
        "Fsig1111111111111111111111111111111111111111111111111",
        true,
    );
    rpc.put_signature(
        "Fsig2111111111111111111111111111111111111111111111111",
        true,
    );
    for n in 1..=3u8 {
        let sig = format!("Sig{n}11111111111111111111111111111111111111111111111111111");
        rpc.put_signature(&sig, false);
        rpc.put_transaction(&sig, "v0", "QUJDRA==");
    }
    let runner = FakeRunner::ok(payload("Title", &[]));
    let (section, counts) = run(&rpc, &runner, &runner, 2);

    assert_eq!(
        counts,
        ReplayCounts {
            replayed: 2,
            unchanged: 2,
            changed: 0,
            errors: 0,
            v1_failures: 0,
        }
    );
    assert!(!section.contains("Fsig"));
}

#[test]
fn v1_transactions_are_counted_and_reported_as_simd_0385_failures() {
    let rpc = one_tx("1");
    let before = FakeRunner::failing();
    let after = FakeRunner::failing();
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(counts.v1_failures, 1);
    assert_eq!(counts.errors, 0);
    assert!(section.contains("v1 (SIMD-0385) parse failure"));
    assert!(section.contains("issue #662"));
    // Not reported as an ordinary parse error.
    assert!(!section.contains("parse error (before and after)"));
}

#[test]
fn ordinary_parse_failures_report_both_error_texts() {
    let rpc = one_tx("legacy");
    let before = FakeRunner::failing();
    let after = FakeRunner::failing();
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(counts.errors, 1);
    assert_eq!(counts.v1_failures, 0);
    assert!(section.contains("parse error (before and after)"));
    assert!(section.contains("before: unsupported transaction layout"));
    assert!(section.contains("after: unsupported transaction layout"));
}

#[test]
fn a_one_sided_parse_failure_is_a_reported_difference() {
    let rpc = one_tx("v0");
    let before = FakeRunner::failing();
    let after = FakeRunner::ok(payload("New title", &[]));
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(counts.errors, 1);
    assert!(section.contains("before:\n\n```\nparse error: unsupported transaction layout\n```"));
    assert!(section.contains("after:\n\n```\nInstruction 1: New title\n```"));
}

#[test]
fn an_unavailable_transaction_is_reported_and_counted() {
    let mut rpc = MockRpc::new();
    rpc.put_signature(
        "Gone1111111111111111111111111111111111111111111111111111",
        false,
    );
    // No getTransaction reply for it.
    let runner = FakeRunner::ok(payload("Title", &[]));
    let (section, counts) = run(&rpc, &runner, &runner, 5);

    assert_eq!(counts.errors, 1);
    assert!(section.contains("no longer available via RPC"));
}

#[test]
fn a_transaction_without_the_program_renders_no_rows_on_both_sides() {
    // The tx involves the program per getSignaturesForAddress, but the
    // payload carries no preview_layout for it (e.g. a memo-only ix).
    let rpc = one_tx("v0");
    let runner = FakeRunner::ok(json::canonical(&Json::object(vec![(
        "Fields",
        Json::Array(vec![]),
    )])));
    let (section, counts) = run(&rpc, &runner, &runner, 5);
    // Both sides are the same empty view: unchanged.
    assert_eq!(counts.unchanged, 1);
    assert!(section.contains("unchanged"));
}

#[test]
fn hostile_titles_cannot_close_the_before_after_fences() {
    let rpc = one_tx("v0");
    let before = FakeRunner::ok(payload("safe", &[("Row", "value ``` \nhere")]));
    let after = FakeRunner::ok(payload("safe 2", &[("Row", "value ``` \nhere")]));
    let (section, counts) = run(&rpc, &before, &after, 5);

    assert_eq!(counts.changed, 1);
    // The only fence delimiters are this tool's own: an even count, and
    // the hostile run never survives as backticks.
    assert_eq!(section.matches("```").count() % 2, 0);
    assert!(!section.contains("value ```"));
    assert!(section.contains(&sanitize::neutralize_backticks("value ``` \nhere")));
    assert_eq!(
        section.matches("```").count(),
        4,
        "two blocks: before and after"
    );
}

#[test]
fn no_successful_signatures_is_reported_not_an_error() {
    let mut rpc = MockRpc::new();
    rpc.put_signature(
        "Fsig1111111111111111111111111111111111111111111111111",
        true,
    );
    let runner = FakeRunner::ok(payload("Title", &[]));
    let (section, counts) = run(&rpc, &runner, &runner, 5);

    assert_eq!(counts, ReplayCounts::default());
    assert!(section.contains("No successful mainnet transactions"));
}

#[test]
fn a_zero_limit_is_rejected() {
    let rpc = one_tx("v0");
    let runner = FakeRunner::ok(payload("Title", &[]));
    assert!(replay::replay_section(&rpc, PROGRAM_ID, &runner, &runner, 0).is_err());
}

#[test]
fn an_invalid_program_id_is_rejected() {
    let rpc = one_tx("v0");
    let runner = FakeRunner::ok(payload("Title", &[]));
    let err = replay::replay_section(&rpc, "not a program id", &runner, &runner, 1)
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid --program-id"), "{err}");
}

#[test]
fn the_binary_runner_reports_a_missing_binary() {
    use solana_idl_sync::replay::BinaryParserRunner;
    let runner = BinaryParserRunner::new("/no/such/parser_cli");
    let err = runner.decode("QUJDRA==").unwrap_err();
    assert!(err.contains("failed to run"), "{err}");
}

#[test]
fn rpc_errors_never_leak_the_url_or_its_api_key() {
    // A refused localhost connection fails fast without any network, and
    // reqwest embeds the URL (with the key) in the transport error --
    // HttpRpc must redact it.
    let rpc = HttpRpc::new("https://127.0.0.1:9/?api-key=SECRET").unwrap();
    let err = rpc
        .transaction("Sig11111111111111111111111111111111111111111111111111111")
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(!text.contains("SECRET"), "leaked the api key: {text}");
    assert!(!text.contains("api-key"), "leaked the key name: {text}");
    assert!(!text.contains("127.0.0.1:9"), "leaked the url: {text}");
    assert!(
        text.contains("<redacted>") || text.contains("failed"),
        "{text}"
    );
}

#[test]
fn counts_json_is_the_single_line_stdout_shape() {
    let counts = ReplayCounts {
        replayed: 5,
        unchanged: 3,
        changed: 1,
        errors: 1,
        v1_failures: 1,
    };
    let text = replay::counts_json(&counts);
    assert_eq!(
        json::parse(&text).unwrap(),
        json::parse(r#"{"changed":1,"errors":1,"replayed":5,"unchanged":3,"v1_failures":1}"#)
            .unwrap()
    );
}
