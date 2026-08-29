//! Per-stage latency benchmarks for the Solana parse path.
//!
//! Solana is the chain that emits `intermediate_output`, and it currently
//! produces it by decoding the transaction a SECOND time: the human-readable
//! `SignablePayload` is built by `instructions::decode_instructions`, while the
//! intermediate blob is built by `extract_solana_intermediate_output` ->
//! `solana_parser::parse_transaction_with_idls` over the re-serialized message.
//!
//! The delta between the `convert` and `convert_with_intermediate` groups is
//! the cost of that re-parse. It is the baseline to beat once the structured
//! decode becomes the single source of truth and the `SignablePayload` is
//! derived from it (see the `build_intermediate_bytes` doc comment in
//! `core/visualsign.rs`).
//!
//! Inputs are the real protocol fixtures under `tests/fixtures/`, each of which
//! captures one instruction (program id, accounts, base58 data) lifted from a
//! mainnet transaction. Each is wrapped in a single-instruction transaction,
//! preserving the fixture's account metas.
//!
//! Run:
//!   cargo bench -p visualsign-solana --bench solana_stages
//!   cargo bench -p visualsign-solana --bench solana_stages -- jupiter
//!
//! Profile:
//!   cargo bench -p visualsign-solana --bench solana_stages -- --profile-time 10
//!   samply record target/release/deps/solana_stages-<hash> --bench --profile-time 10

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine;
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use serde::Deserialize;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
};
use std::{fs, hint::black_box, path::PathBuf, str::FromStr};
use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
use visualsign_solana::{
    SolanaTransactionWrapper, SolanaVisualSignConverter, build_solana_intermediate_output,
};

/// Fixtures to benchmark, as (group label, relative path under tests/fixtures).
/// Chosen to span the cost range: a small SPL token op, several Token-2022
/// operations, an Orca liquidity call, and a 25-account Jupiter route.
const FIXTURES: &[(&str, &str)] = &[
    ("spl_mint_to", "spl_token/mint_to_example.json"),
    ("t22_transfer_checked", "token_2022/transfer_checked.json"),
    ("t22_burn_checked", "token_2022/burn_checked.json"),
    ("t22_set_authority", "token_2022/set_authority.json"),
    (
        "orca_increase_liquidity",
        "orca_whirlpool/increase_liquidity_by_token_amounts_v2.json",
    ),
    ("jupiter_route", "jupiter_swap/sample_route.json"),
];

#[derive(Deserialize)]
struct Fixture {
    instruction_data: String,
    program_id: String,
    accounts: Vec<FixtureAccount>,
}

#[derive(Deserialize)]
struct FixtureAccount {
    pubkey: String,
    signer: bool,
    writable: bool,
}

/// Build the base64 unsigned transaction a fixture's instruction represents.
///
/// Mirrors `create_transaction_with_empty_signatures`: a compact-array length
/// of 0 signatures followed by the serialized message.
fn transaction_from_fixture(rel_path: &str) -> String {
    fixture_inputs(rel_path).0
}

/// The hex-encoded serialized message for a fixture, the input the structured
/// decode actually consumes.
fn message_hex_from_fixture(rel_path: &str) -> String {
    fixture_inputs(rel_path).1
}

/// Returns `(base64 unsigned transaction, hex-encoded message)`.
fn fixture_inputs(rel_path: &str) -> (String, String) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel_path);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    let fixture: Fixture = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse fixture {}: {e}", path.display()));

    let program_id = Pubkey::from_str(&fixture.program_id).expect("program id");
    let accounts: Vec<AccountMeta> = fixture
        .accounts
        .iter()
        .map(|a| AccountMeta {
            pubkey: Pubkey::from_str(&a.pubkey).expect("account pubkey"),
            is_signer: a.signer,
            is_writable: a.writable,
        })
        .collect();

    // Instruction data in these fixtures is base58, as returned by JSON RPC.
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("base58 instruction data");

    // The fee payer must be a signer; reuse the fixture's first signer when it
    // has one so the account set stays faithful to the captured instruction.
    let fee_payer = fixture
        .accounts
        .iter()
        .find(|a| a.signer)
        .map(|a| Pubkey::from_str(&a.pubkey).expect("signer pubkey"))
        .unwrap_or_else(Pubkey::new_unique);

    let instruction = Instruction::new_with_bytes(program_id, &data, accounts);
    let message = Message::new(&[instruction], Some(&fee_payer));

    let serialized = message.serialize();
    let mut bytes = vec![0u8]; // zero signatures
    bytes.extend_from_slice(&serialized);
    (
        base64::engine::general_purpose::STANDARD.encode(bytes),
        hex::encode(&serialized),
    )
}

fn options(include_intermediate_output: bool) -> VisualSignOptions {
    VisualSignOptions {
        decode_transfers: true,
        transaction_name: Some("Solana Transaction".to_string()),
        include_intermediate_output,
        ..VisualSignOptions::default()
    }
}

/// Stage 1: base64 transaction -> `SolanaTransactionWrapper`.
fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("solana/decode");
    for (name, path) in FIXTURES {
        let tx = transaction_from_fixture(path);
        group.bench_with_input(BenchmarkId::from_parameter(name), &tx, |b, tx| {
            b.iter(|| black_box(SolanaTransactionWrapper::from_string(black_box(tx)).unwrap()));
        });
    }
    group.finish();
}

/// Stages 2 and 3. The delta between the two groups is the re-parse cost that
/// making the intermediate representation the single source of truth removes.
fn bench_convert(c: &mut Criterion) {
    for (group_name, with_intermediate) in [
        ("solana/convert", false),
        ("solana/convert_with_intermediate", true),
    ] {
        let mut group = c.benchmark_group(group_name);
        for (name, path) in FIXTURES {
            let tx = transaction_from_fixture(path);
            let wrapper = SolanaTransactionWrapper::from_string(&tx).unwrap();

            // Skip fixtures this converter cannot handle rather than panicking
            // mid-run; some captures exist to assert error paths.
            if SolanaVisualSignConverter
                .to_visual_sign_payload(wrapper.clone(), options(with_intermediate))
                .is_err()
            {
                eprintln!("skipping {name}: conversion returns an error for this fixture");
                continue;
            }

            group.bench_with_input(BenchmarkId::from_parameter(name), &wrapper, |b, w| {
                // `to_visual_sign_payload` consumes the wrapper, so clone in
                // untimed setup rather than inside the measured closure.
                b.iter_batched(
                    || w.clone(),
                    |w| {
                        black_box(
                            SolanaVisualSignConverter
                                .to_visual_sign_payload(w, options(with_intermediate))
                                .unwrap(),
                        )
                    },
                    BatchSize::SmallInput,
                );
            });
        }
        group.finish();
    }
}

/// The first half of the pipeline on its own: the single structured decode, up
/// to and including `intermediate_output` creation, with no `SignablePayload`
/// rendering.
///
/// Compare against `solana/convert_with_intermediate` to see how much of the
/// current cost is rendering versus decoding, and against `solana/convert` to
/// see what a caller that needs only policy metadata would pay.
fn bench_intermediate_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("solana/intermediate_only");
    for (name, path) in FIXTURES {
        let message_hex = message_hex_from_fixture(path);
        if build_solana_intermediate_output(&message_hex, &options(true)).is_err() {
            eprintln!("skipping {name}: intermediate extraction returns an error");
            continue;
        }
        group.bench_with_input(BenchmarkId::from_parameter(name), &message_hex, |b, hex| {
            b.iter(|| {
                black_box(build_solana_intermediate_output(black_box(hex), &options(true)).unwrap())
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_decode,
    bench_intermediate_only,
    bench_convert
);
criterion_main!(benches);
