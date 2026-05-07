//! End-to-end policy-expressiveness tests using Google's CEL.
//!
//! These tests *do not* simulate Turnkey's policy engine — that engine is
//! server-side. Instead, they assert that the structured `SolanaIntermediateOutput`
//! carries enough information to express each of Turnkey's documented Solana
//! policy patterns. We use [`cel-interpreter`](https://crates.io/crates/cel-interpreter),
//! a Rust implementation of [Google CEL](https://github.com/google/cel-spec)
//! (the spec Turnkey says their evaluator is based on).
//!
//! ## Macro / property aliases
//!
//! Turnkey's docs surface a couple of CEL aliases that the standard CEL
//! grammar doesn't ship with. The expressions below use canonical CEL:
//!
//! | Turnkey docs            | Canonical CEL          |
//! |-------------------------|------------------------|
//! | `xs.any(x, p)`          | `xs.exists(x, p)`      |
//! | `xs.count`              | `size(xs)`             |
//!
//! Same semantics; if you need byte-identical Turnkey syntax for a fixture
//! capture, register `any` as a thin alias on top of `exists` in the
//! evaluator (out of scope here).
//!
//! ## Combining rules
//!
//! A CEL expression is *one* boolean. Combine rules within a single
//! expression using the standard grammar:
//!
//! - `&&` (AND, short-circuit), `||` (OR, short-circuit), `!` (NOT)
//! - ternary `cond ? a : b`
//! - comparisons `==`, `!=`, `<`, `<=`, `>`, `>=`, `in`
//! - list comprehensions `.exists`, `.all`, `.exists_one`, `.filter`
//! - `size(list_or_string_or_map)`, `x in list`
//!
//! Across multiple `--policy` CLI flags we apply an implicit **AND** —
//! every rule must PASS for the process to exit zero. There is no `||`
//! between flags; if you want OR semantics, write one expression with
//! `||`. This matches the wallet/policy mental model of "every rule
//! must hold", but it is a CLI convention, not part of CEL.
//!
//! ## Out of scope: engine-level policy structure
//!
//! Real policy engines (Turnkey's included) layer additional structure on
//! top of CEL that this PoC does not model:
//!
//! - **Effect**: each rule is `EFFECT_ALLOW` or `EFFECT_DENY`, and the
//!   engine combines them with documented precedence (typically: explicit
//!   `DENY` wins; absence of a matching `ALLOW` is treated as deny).
//! - **Consensus**: a separate CEL expression evaluated against an
//!   approvers / users root determines *who* must sign off (e.g.
//!   `approvers.count >= 2 && approvers.any(u, u.tags.contains('admin'))`).
//!
//! These tests assert only that the structured intermediate output is
//! expressive enough to encode the *condition* half of those rules —
//! not that we faithfully simulate the surrounding engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cel_interpreter::{Context, Program};
use serde_json::json;
use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
use visualsign_solana::intermediate::SolanaIntermediateOutput;
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

/// Jupiter swap legacy transaction (1 native SOL transfer of 1_000_000 lamports
/// from 6DSxAQ2H... to AEdS5zTy...). Reused from `lib.rs`'s charset test.
const JUPITER_SWAP_B64: &str = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAkSTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ664CMz4dTWd4gwDq6aKU/sqHgTzleVA7bTCOy59kSOO+0EPkGS7bWuT/2yiCuaADtj/v6d+KwyTj46OQM2MjIq6hTqzVdwLTW8t+UsWMrwHEvc/r814OmVR9yLVQZujbWvpTh0XSNlF7uoIvuHyKD/16mBElrNa/eT8vB1KVUaN8IoaTvZbN4b7iiv8Q8cl5bDecNqCXzTS1Xmsmh5b2UVZniTbtX0AYG5QKiSDC10m0caM6frmEVukpjEWOk7F/0OzFKL0A0HdMWTIMuQj4xBuP3csLyGzVO/MXtPu6woNViO2O9ocxd1YSDcIwhrzHY3a9ewvycRH5q662TcQqdxD6AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKkOA2hfjpCQU+RYEhxm9adq7cdwaqEcgviqlSqPK3h5qVJNNVq4xx0JIWWE9kFLvpQK5lvS5UCde3W3QfWYLIxYjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+Fm0P/on9df2SnTAmx8pWHneSwmrNt/J3VFLMhqns4zl6Mb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hXuFhKBWRymmouYdcNxL6PjM1Bkcio0R+AtqA/P3C3jAFDwYABgALCQwBAQkCAAYMAgAAAEBCDwAAAAAADAEGAREKFQwABgUKEQoQCg0MAAQGAwUHCAECDiTlF8uXeuOtKgEAAAARAWQAAUBCDwAAAAAAtEADAAAAAAAyAAAMAwYAAAEJ";

const SENDER: &str = "6DSxAQ2HdBLGYwa3AQf6hXXjNZ762p761ANxBDqrao5P";
const RECIPIENT_ATA: &str = "AEdS5zTyeygvEbnsi5oszJLfu8mRwPmSFPyuPT1tDxMR";
const JUPITER_PROGRAM: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// Parse the fixture, extract intermediate_output bytes, borsh-decode.
fn fixture_intermediate_output() -> SolanaIntermediateOutput {
    let wrapper = SolanaTransactionWrapper::from_string(JUPITER_SWAP_B64).expect("fixture parses");
    let result = SolanaVisualSignConverter
        .to_visual_sign_payload(wrapper, VisualSignOptions::default())
        .expect("conversion succeeds");
    let bytes = result
        .intermediate_output
        .expect("Solana converter populates intermediate_output");
    borsh::from_slice::<SolanaIntermediateOutput>(&bytes).expect("borsh round-trips")
}

/// Wrap the borsh-friendly intermediate output as the JSON value Turnkey's
/// policy DSL expects (`solana.tx`).
fn cel_root_value(output: &SolanaIntermediateOutput) -> serde_json::Value {
    let tx = serialize_intermediate_to_json(output);
    json!({ "tx": tx })
}

/// Mirror the CLI's `serialize_solana_intermediate` helper. Kept self-contained
/// here so the test crate doesn't pull in `parser_cli`.
fn serialize_intermediate_to_json(output: &SolanaIntermediateOutput) -> serde_json::Value {
    let instructions: Vec<_> = output
        .instructions
        .iter()
        .map(|i| {
            let parsed = i.parsed_instruction_data.as_ref().map(|p| {
                let args: serde_json::Value = serde_json::from_str(&p.program_call_args_json)
                    .unwrap_or_else(|_| json!(p.program_call_args_json));
                json!({
                    "instruction_name": p.instruction_name,
                    "discriminator": p.discriminator,
                    "named_accounts": p.named_accounts,
                    "program_call_args": args,
                    "idl_source": p.idl_source,
                    "idl_hash": p.idl_hash,
                })
            });
            json!({
                "program_key": i.program_key,
                "accounts": i.accounts.iter().map(|a| json!({
                    "account_key": a.account_key,
                    "signer": a.signer,
                    "writable": a.writable,
                })).collect::<Vec<_>>(),
                "instruction_data_hex": i.instruction_data_hex,
                "address_table_lookups": i.address_table_lookups.iter().map(|lk| json!({
                    "address_table_key": lk.address_table_key,
                    "index": lk.index,
                    "writable": lk.writable,
                })).collect::<Vec<_>>(),
                "parsed_instruction_data": parsed,
            })
        })
        .collect();

    json!({
        "account_keys": output.account_keys,
        "program_keys": output.program_keys,
        "instructions": instructions,
        "transfers": output.transfers.iter().map(|t| json!({
            "from": t.from, "to": t.to, "amount": t.amount,
        })).collect::<Vec<_>>(),
        "spl_transfers": output.spl_transfers.iter().map(|t| json!({
            "from": t.from, "to": t.to, "amount": t.amount, "owner": t.owner,
            "signers": t.signers, "token_mint": t.token_mint,
            "decimals": t.decimals, "fee": t.fee,
        })).collect::<Vec<_>>(),
        "recent_blockhash": output.recent_blockhash,
        "address_table_lookups": output.address_table_lookups.iter().map(|lk| json!({
            "address_table_key": lk.address_table_key,
            "writable_indexes": lk.writable_indexes,
            "readonly_indexes": lk.readonly_indexes,
        })).collect::<Vec<_>>(),
    })
}

/// Compile the policy expression and evaluate it against a CEL context that
/// has `solana` bound to the fixture's intermediate output.
fn evaluate(policy: &str, root: &serde_json::Value) -> bool {
    let program = Program::compile(policy).expect("policy compiles");
    let mut ctx = Context::default();
    let cel_value = cel_interpreter::to_value(root).expect("serialize to CEL value");
    ctx.add_variable_from_value("solana", cel_value);
    match program.execute(&ctx).expect("policy evaluates") {
        cel_interpreter::Value::Bool(b) => b,
        other => panic!("policy must return bool, got {other:?}"),
    }
}

// ── Policies adapted from Turnkey's Solana policy-engine announcement ───────

#[test]
fn allows_transactions_only_from_designated_sender() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    // PASS: every native SOL transfer originates from the expected sender.
    assert!(evaluate(
        &format!("solana.tx.transfers.all(t, t.from == '{SENDER}')"),
        &root,
    ));

    // DENY: a different sender — no native transfer matches, so the predicate fails.
    assert!(!evaluate(
        "solana.tx.transfers.all(t, t.from == 'NotTheSender11111111111111111111111111111')",
        &root,
    ));
}

#[test]
fn allows_only_transactions_with_exactly_one_transfer_to_recipient() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    // PASS: there's exactly one native transfer, and it goes to the expected ATA.
    assert!(evaluate(
        &format!(
            "size(solana.tx.transfers) == 1 && \
             solana.tx.transfers.all(t, t.to == '{RECIPIENT_ATA}')"
        ),
        &root,
    ));
}

#[test]
fn denies_transfers_to_blocked_address() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    let bad_addr = "BadAddr1111111111111111111111111111111111";

    // PASS (allowed): no native or SPL transfer touches the blocked address.
    assert!(evaluate(
        &format!(
            "!(solana.tx.transfers.exists(t, t.to == '{bad_addr}') || \
              solana.tx.spl_transfers.exists(t, t.to == '{bad_addr}'))"
        ),
        &root,
    ));
}

#[test]
fn restricts_to_known_program_set() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    // Whitelist of programs we expect this transaction to invoke.
    let allowed = format!(
        "solana.tx.program_keys.all(p, \
            p == '{JUPITER_PROGRAM}' || p == '{TOKEN_PROGRAM}' || \
            p == '{ATA_PROGRAM}'    || p == '{SYSTEM_PROGRAM}')"
    );
    assert!(evaluate(&allowed, &root));

    // DENY: drop the system program from the allowlist — every program_key
    // should still be allowed, but `11111111…` won't be → expression false.
    let too_strict = format!(
        "solana.tx.program_keys.all(p, \
            p == '{JUPITER_PROGRAM}' || p == '{TOKEN_PROGRAM}' || p == '{ATA_PROGRAM}')"
    );
    assert!(!evaluate(&too_strict, &root));
}

#[test]
fn forbids_address_table_lookups() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    // The fixture is a legacy transaction → no ALT lookups → policy passes.
    assert!(evaluate(
        "size(solana.tx.address_table_lookups) == 0",
        &root
    ));
}

#[test]
fn idl_aware_instruction_name_check() {
    let output = fixture_intermediate_output();
    let root = cel_root_value(&output);

    // Sanity: at least one instruction targets Jupiter and decodes to "route".
    let policy = format!(
        "solana.tx.instructions.exists(i, \
            i.program_key == '{JUPITER_PROGRAM}' && \
            i.parsed_instruction_data != null && \
            i.parsed_instruction_data.instruction_name == 'route')"
    );
    assert!(evaluate(&policy, &root));

    // Inverse: deny if any instruction is `closeUserAccount`. Should pass
    // (this fixture is a `route` swap, not an account close).
    let deny = format!(
        "!solana.tx.instructions.exists(i, \
            i.program_key == '{JUPITER_PROGRAM}' && \
            i.parsed_instruction_data != null && \
            i.parsed_instruction_data.instruction_name == 'closeUserAccount')"
    );
    assert!(evaluate(&deny, &root));
}
