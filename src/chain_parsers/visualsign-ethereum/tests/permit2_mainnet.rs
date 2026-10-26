#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Real mainnet Permit2 calldata, checked against an independent ABI decode.
//!
//! Each `json-permit2-*` fixture in `tests/fixtures` carries the `to` and
//! calldata of a real Ethereum mainnet transaction (hash and block in
//! [`PERMIT2_FIXTURES`]). The JSON form has no gas, fee or nonce, so the
//! snapshots show the parser's defaults for those. The snapshot test in
//! `lib_test.rs` pins the whole rendering; this test checks the property that
//! matters for #346 regardless of layout: every amount the calldata carries is
//! shown in full (scaled by the built-in registry's decimals), and a `permit` shows
//! "Unlimited Amount" if and only if the amount is `type(uint160).max`.
//!
//! `validate_real_calls_from_env` (ignored) runs the same check over a JSONL file
//! of transactions (`P2_REAL_TXS`, one `{"tx","block","to","input"}` per line)
//! and writes a TSV report to `P2_OUT`. It was used to validate #608 against
//! mainnet traffic and is kept for re-running that survey.

mod common;

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use alloy_primitives::{Address, U160};
use serde_json::Value;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::registry::ContractRegistry;
use visualsign_ethereum::transaction_string_to_visual_sign;

use common::{DecodedAmount, builtin_token, collect_strings, decode_reference, expected_amount};

/// `(fixture name, tx hash, block, expected outcome)`. Hashes are Ethereum
/// mainnet; the `.input` files carry the transaction's `to` and calldata.
const PERMIT2_FIXTURES: &[(&str, &str, u64, Outcome)] = &[
    (
        "json-permit2-approve-u160-max",
        "0xd3a3b345538493d89edae39edcca994489524fb5a5b78700912d3b6c1c55522d",
        26_272_888,
        Outcome::Match,
    ),
    (
        "json-permit2-approve-2pow159-usdt",
        "0x695c2310d2260cb35b6bb802d77faed5f148e0b5f7c64759438f574b8a2e8fa7",
        26_205_434,
        Outcome::Match,
    ),
    (
        "json-permit2-approve-2pow159-weth",
        "0x821cdceca1cb515df8d777a39580f330127861ab74b2e8ce1b70bb48067bcb01",
        26_207_514,
        Outcome::Match,
    ),
    (
        "json-permit2-approve-zero",
        "0x014521d66a738fc071a34610912ad3b8be4892a032798ed5154f41a02cd8ef57",
        26_272_444,
        Outcome::Match,
    ),
    (
        "json-permit2-permit-single-u160-max",
        "0xa5f9291f79240c6afe8ad83a5e7f946bf53b722da79da76dfb99adf10e539de7",
        26_272_918,
        Outcome::Match,
    ),
    (
        "json-permit2-transfer-from",
        "0x2187e7f16a8014454c45aa732788ae1560770c3146ed5f9a93ad57f296af7c7d",
        26_270_136,
        Outcome::Match,
    ),
    (
        "json-permit2-ur-permit-u160-max",
        "0x04260d8a7b2c2c5ba84f76cbed3e967955d95346d635f38ac0fe23f35e0207cd",
        26_269_542,
        Outcome::Match,
    ),
    (
        "json-permit2-ur-permit-weth",
        "0x4615f6666ba83b4560eb0db9c704e5c80c62318265cad23337c1cf1bfc72f86c",
        26_272_796,
        Outcome::Match,
    ),
];

/// Real mainnet calldata the parser renders wrongly today.
struct KnownMisrender {
    fixture: &'static str,
    tx: &'static str,
    block: u64,
    /// The GitHub issue tracking the bug.
    issue: u32,
    what: &'static str,
    /// The wrong line the parser shows today, pinned by
    /// `known_misrenders_still_render_as_today` so a fix or behaviour change
    /// is noticed.
    current: &'static str,
}

/// Real mainnet calldata the parser renders wrongly. The inputs have no
/// `.expected` snapshot on purpose, so the wrong output is not blessed.
const KNOWN_MISRENDERS: &[KnownMisrender] = &[
    KnownMisrender {
        fixture: "json-permit2-permit-batch",
        tx: "0xc346205520a2b15ec1388301df5f4810fb624e133fd2a325d4c7d1710a8a0216",
        block: 26_205_925,
        issue: 610,
        what: "permit(PermitBatch) renders as a PermitSingle read from the calldata head",
        current: "Permit 0x00000000F80762A6FAc58536D5715BEa6233dd97 to spend 0 of 0x000000003ed3404098a6b6d7ed7d976971cce40b",
    },
    KnownMisrender {
        fixture: "json-permit2-transfer-from-batch",
        tx: "0xfb60c23358afb59cdc49595851f4d06185ce696f3daa09cbb9c737ed5364994f",
        block: 26_270_629,
        issue: 610,
        what: "batch transferFrom renders as a PermitSingle",
        current: "Permit 0x0000000000000000000000000000000000000000 to spend 0 of 0x0000000000000000000000000000000000000000",
    },
    KnownMisrender {
        fixture: "json-permit2-lockdown-2",
        tx: "0x1b69d2285d610343af62f574cf58aa36cb96385d826d5fac935fdb4e71abe711",
        block: 26_219_155,
        issue: 610,
        what: "lockdown with two pairs renders as a PermitSingle",
        current: "Permit 0x00000000A0B86991C6218b36C1d19D4A2E9Eb0ce to spend 0 of 0x0000000000000000000000000000000000000000",
    },
    KnownMisrender {
        fixture: "json-permit2-permit-transfer-from",
        tx: "0x4d6889e8697372de79386e974f2cbba12325d23715b979f4d96cb853d8033a32",
        block: 26_242_480,
        issue: 610,
        what: "SignatureTransfer permitTransferFrom renders as an Unlimited PermitSingle",
        current: "Permit 0x0000000040C27eB70216fE6e353D6cC618d7b74c to spend Unlimited Amount of 0x00000000dac17f958d2ee523a2206206994597c1",
    },
    KnownMisrender {
        fixture: "json-permit2-ur-permit-allow-revert",
        tx: "0x56df5056cd7e804e992861328fa032cab7b0d85cd7dd246910000625f23c4a78",
        block: 26_270_407,
        issue: 611,
        what: "PERMIT2_PERMIT with FLAG_ALLOW_REVERT (0x8a) is dropped from the command list",
        current: "Uniswap Universal Router Execute: 1 commands ([V4Swap]), deadline 2026-10-25 15:21:44 UTC",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Every decoded amount is rendered correctly.
    Match,
}

/// What the parser showed for one transaction, next to the independent decode.
struct Report {
    function: String,
    amounts: Vec<DecodedAmount>,
    expected: Vec<String>,
    preview: Vec<String>,
    problems: Vec<String>,
    unsupported: bool,
    /// `to` is a contract the parser's registry knows (Permit2 or a
    /// registered Universal Router).
    routed: bool,
}

fn options() -> VisualSignOptions {
    VisualSignOptions {
        include_intermediate_output: false,
        decode_transfers: true,
        transaction_name: None,
        metadata: None,
        developer_config: None,
    }
}

/// Decimals for `token` as the parser's built-in mainnet registry has them,
/// cross-checked against the registry itself.
fn registry_decimals(registry: &ContractRegistry, token: Address) -> Option<u8> {
    let builtin = builtin_token(token);
    assert_eq!(
        registry.get_token_symbol(1, token).as_deref(),
        builtin.map(|(s, _)| s),
        "built-in mainnet registry changed: update BUILTIN_MAINNET_TOKENS ({token})"
    );
    builtin.map(|(_, d)| d)
}

fn check(registry: &ContractRegistry, to: Address, data: &[u8]) -> Report {
    let input = serde_json::json!({
        "type": "transaction",
        "to": to.to_string(),
        "chainId": "0x1",
        "data": format!("0x{}", hex::encode(data)),
    })
    .to_string();
    let payload = transaction_string_to_visual_sign(&input, options())
        .unwrap_or_else(|e| panic!("parser rejected {to} calldata: {e:?}"));
    let json: Value = serde_json::from_str(&payload.to_json().unwrap()).unwrap();
    let mut strings = vec![];
    collect_strings(&json, &mut strings);

    let (function, amounts) = decode_reference(to, data);
    let mut problems = vec![];
    let routed = registry.get_contract_type(1, to).is_some();
    let mut expected = vec![];
    let mut unsupported = false;
    let mut any_permit_max = false;

    for a in &amounts {
        let decimals = registry_decimals(registry, a.token);
        let shown = expected_amount(a.amount, decimals);
        // Symbol as the parser shows it: the registry's, or the token's Debug
        // form for unknown tokens.
        let symbol = builtin_token(a.token)
            .map(|(sym, _)| sym.to_string())
            .unwrap_or_else(|| format!("{:?}", a.token));
        // The whole rendered line, so spender, token and parties are checked
        // along with the amount. `prefix` lines continue with the expiry.
        let (needle, prefix_only) = match (a.path, a.spender, a.from_to) {
            ("permit" | "UR PERMIT2_PERMIT", Some(spender), _) => {
                let shown = if a.amount == U160::MAX {
                    any_permit_max = true;
                    "Unlimited Amount".to_string()
                } else {
                    shown
                };
                let token = a.token.to_string().to_lowercase();
                (
                    format!("Permit {spender} to spend {shown} of {token}"),
                    false,
                )
            }
            ("approve", Some(spender), _) => (
                format!("Approve {spender} {shown} {symbol} to spend {symbol} (expires: "),
                true,
            ),
            ("transferFrom", _, Some((from, to))) => (
                format!("Transfer {shown} {symbol} from {from} to {to}"),
                false,
            ),
            _ => {
                unsupported = true;
                expected.push(format!("{}: {shown} (not decoded by the parser)", a.path));
                continue;
            }
        };
        expected.push(format!("{}: {needle}", a.path));
        let found = strings.iter().any(|s| {
            if prefix_only {
                s.starts_with(&needle)
            } else {
                *s == needle
            }
        });
        if !found {
            problems.push(format!("{}: expected {needle:?}", a.path));
        }
    }

    let unlimited_shown = strings.iter().any(|s| s.contains("Unlimited"));
    if unlimited_shown != any_permit_max {
        problems.push(format!(
            "Unlimited shown={unlimited_shown} but a permit amount is U160::MAX={any_permit_max}"
        ));
    }

    // A `permit` rendering must describe a PermitSingle the calldata carries.
    // The parser's inline-PermitSingle fallback can otherwise read garbage out
    // of any other Permit2 call that is at least 192 bytes long.
    let decoded_permit = amounts
        .iter()
        .any(|a| matches!(a.path, "permit" | "UR PERMIT2_PERMIT"));
    if !decoded_permit
        && strings
            .iter()
            .any(|s| s.starts_with("Permit ") && s.contains("to spend"))
    {
        problems.push("rendered a Permit2 Permit the calldata does not contain".into());
    }
    // Nothing the parser decodes to check (lockdown, nonce invalidation,
    // SignatureTransfer, or no Permit2 amounts at all).
    if amounts.is_empty() {
        unsupported = true;
    }

    let mut preview: Vec<String> = strings
        .into_iter()
        .filter(|s| {
            s.starts_with("Approve ")
                || s.starts_with("Permit ")
                || s.starts_with("Transfer ")
                || s.starts_with("Permit2")
                || s.starts_with("Uniswap Universal Router Execute:")
                || s.starts_with("Input:")
                || s.starts_with("Failed to decode")
                || s.starts_with("Error:")
        })
        .map(|s| s.chars().take(240).collect())
        .collect();
    preview.dedup();

    Report {
        function,
        amounts,
        expected,
        preview,
        problems,
        unsupported,
        routed,
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn read_fixture(name: &str) -> (Address, Vec<u8>) {
    let raw = fs::read_to_string(fixture_path(&format!("{name}.input")))
        .unwrap_or_else(|e| panic!("{name}.input: {e}"));
    let v: Value = serde_json::from_str(&raw).unwrap();
    let to: Address = v["to"].as_str().unwrap().parse().unwrap();
    let data = hex::decode(v["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
    (to, data)
}

#[test]
fn permit2_mainnet_fixtures_render_decoded_amounts() {
    let (registry, _) = ContractRegistry::with_default_protocols();
    for (name, tx, block, outcome) in PERMIT2_FIXTURES {
        let (to, data) = read_fixture(name);
        let report = check(&registry, to, &data);
        assert!(
            !report.amounts.is_empty(),
            "{name} ({tx} @ {block}): reference decode found no Permit2 amounts"
        );
        match outcome {
            Outcome::Match => assert!(
                report.problems.is_empty() && !report.unsupported,
                "{name} ({tx} @ {block}) {}: {:?}\npreview: {:#?}",
                report.function,
                report.problems,
                report.preview
            ),
        }
    }
}

/// The calls for `issue` render what the reference decode says.
fn assert_known_misrenders_fixed(issue: u32) {
    let (registry, _) = ContractRegistry::with_default_protocols();
    let mut failures = vec![];
    for m in KNOWN_MISRENDERS.iter().filter(|m| m.issue == issue) {
        let (to, data) = read_fixture(m.fixture);
        let report = check(&registry, to, &data);
        if !report.problems.is_empty() {
            failures.push(format!(
                "{} ({} @ {}): {}\n  {:?}\n  preview: {:?}",
                m.fixture, m.tx, m.block, m.what, report.problems, report.preview
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "known bug #610: the inline-PermitSingle fallback misrenders PermitBatch, lockdown, SignatureTransfer and batch transferFrom"]
fn known_misrenders_610_render_correctly() {
    assert_known_misrenders_fixed(610);
}

#[test]
#[ignore = "known bug #611: Universal Router commands with FLAG_ALLOW_REVERT are dropped from the display"]
fn known_misrenders_611_render_correctly() {
    assert_known_misrenders_fixed(611);
}

/// Pins today's wrong output for the known misrenders, so CI notices when a
/// fix or any other behaviour change lands. When this fails because a bug
/// was fixed, delete the entry's `current` line here and un-ignore the
/// matching `known_misrenders_*_render_correctly` test.
// TODO(#610): remove the #610 entries once the inline-permit fallback is fixed.
// TODO(#611): remove the #611 entry once allow-revert commands are shown.
#[test]
fn known_misrenders_still_render_as_today() {
    let (registry, _) = ContractRegistry::with_default_protocols();
    for m in KNOWN_MISRENDERS {
        let (to, data) = read_fixture(m.fixture);
        let report = check(&registry, to, &data);
        assert!(
            !report.problems.is_empty(),
            "{} ({}): #{} looks fixed; un-ignore known_misrenders_{}_render_correctly",
            m.fixture,
            m.tx,
            m.issue,
            m.issue
        );
        assert!(
            report.preview.iter().any(|line| line == m.current),
            "{} ({}): #{} output changed\n  want: {}\n  preview: {:?}",
            m.fixture,
            m.tx,
            m.issue,
            m.current,
            report.preview
        );
    }
}

/// Survey mode: `P2_REAL_TXS=txs.jsonl P2_OUT=report.tsv cargo test -p
/// visualsign-ethereum --test permit2_mainnet -- --ignored`.
#[test]
#[ignore = "survey tool: needs P2_REAL_TXS and P2_OUT"]
fn validate_real_calls_from_env() {
    let src = std::env::var("P2_REAL_TXS").expect("P2_REAL_TXS");
    let dst = std::env::var("P2_OUT").expect("P2_OUT");
    let (registry, _) = ContractRegistry::with_default_protocols();
    let mut out = fs::File::create(&dst).unwrap();
    writeln!(
        out,
        "tx\tblock\tto\tfunction\tamounts_hex\texpected\tpreview\tverdict\tproblems"
    )
    .unwrap();
    for line in fs::read_to_string(&src).unwrap().lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        let to: Address = v["to"].as_str().unwrap().parse().unwrap();
        let data = hex::decode(v["input"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let report = check(&registry, to, &data);
        let verdict = match (
            report.routed,
            report.problems.is_empty(),
            report.unsupported,
        ) {
            (false, _, _) => "NOT_ROUTED",
            (true, true, false) => "MATCH",
            (true, true, true) => "UNSUPPORTED",
            (true, false, _) => "MISMATCH",
        };
        let amounts: Vec<String> = report
            .amounts
            .iter()
            .map(|a| format!("{:#x}", a.amount))
            .collect();
        writeln!(
            out,
            "{}\t{}\t{to}\t{}\t{}\t{}\t{}\t{verdict}\t{}",
            v["tx"].as_str().unwrap_or(""),
            v["block"],
            report.function,
            amounts.join(","),
            report.expected.join(" | "),
            report.preview.join(" | "),
            report.problems.join(" | "),
        )
        .unwrap();
    }
}
