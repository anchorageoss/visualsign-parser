//! Cost of the compiled-in IDLs on the Solana parse path.
//!
//! `solana/convert` is the default signing path (`include_intermediate_output:
//! false`); `solana/convert_with_intermediate` adds the structured decode,
//! which resolves an IDL for every instruction. `solana/idl_load` is the
//! one-time cost of decoding every compiled-in IDL.
//!
//! Inputs are real protocol fixtures under `tests/fixtures/`, each one
//! instruction lifted from a mainnet transaction and wrapped in a
//! single-instruction transaction.
//!
//! Run:
//!   cargo bench -p visualsign-solana --bench idl_cache
//!   cargo bench -p visualsign-solana --bench idl_cache -- jupiter

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
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

/// (group label, path under tests/fixtures): a small SPL op, a Token-2022 op,
/// an Orca liquidity call and a 25-account Jupiter route.
const FIXTURES: &[(&str, &str)] = &[
    ("spl_mint_to", "spl_token/mint_to_example.json"),
    ("t22_burn_checked", "token_2022/burn_checked.json"),
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

/// The base64 unsigned transaction (zero signatures + message) for a fixture.
fn transaction_from_fixture(rel_path: &str) -> String {
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
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("base58 instruction data");
    let fee_payer = fixture
        .accounts
        .iter()
        .find(|a| a.signer)
        .map(|a| Pubkey::from_str(&a.pubkey).expect("signer pubkey"))
        .unwrap_or_else(Pubkey::new_unique);

    let message = Message::new(
        &[Instruction::new_with_bytes(program_id, &data, accounts)],
        Some(&fee_payer),
    );
    let mut bytes = vec![0u8];
    bytes.extend_from_slice(&message.serialize());
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn options(include_intermediate_output: bool) -> VisualSignOptions {
    VisualSignOptions {
        decode_transfers: true,
        transaction_name: Some("Solana Transaction".to_string()),
        include_intermediate_output,
        ..VisualSignOptions::default()
    }
}

fn bench_convert(c: &mut Criterion) {
    for (group_name, with_intermediate) in [
        ("solana/convert", false),
        ("solana/convert_with_intermediate", true),
    ] {
        let mut group = c.benchmark_group(group_name);
        for (name, path) in FIXTURES {
            let wrapper =
                SolanaTransactionWrapper::from_string(&transaction_from_fixture(path)).unwrap();
            if SolanaVisualSignConverter
                .to_visual_sign_payload(wrapper.clone(), options(with_intermediate))
                .is_err()
            {
                eprintln!("skipping {name}: conversion returns an error for this fixture");
                continue;
            }
            group.bench_with_input(BenchmarkId::from_parameter(name), &wrapper, |b, w| {
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

/// Every compiled-in IDL as JSON: the preset IDLs shipped in this crate plus
/// `solana_parser`'s built-in program IDLs.
fn compiled_in_idl_jsons() -> Vec<String> {
    let presets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/presets");
    let mut jsons: Vec<String> = fs::read_dir(&presets)
        .unwrap()
        .flat_map(|dir| fs::read_dir(dir.unwrap().path()).into_iter().flatten())
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    jsons.extend(
        solana_parser::construct_custom_idl_records_map()
            .unwrap()
            .into_values()
            .filter_map(|record| record.program_type.map(|t| t.idl_json().to_string())),
    );
    jsons
}

/// Decoding every compiled-in IDL from JSON: what a process pays once.
fn bench_idl_load(c: &mut Criterion) {
    let jsons = compiled_in_idl_jsons();
    c.bench_function("solana/idl_load/all", |b| {
        b.iter(|| {
            for json in &jsons {
                black_box(solana_parser::decode_idl_data(black_box(json)).unwrap());
            }
        });
    });
}

criterion_group!(benches, bench_idl_load, bench_convert);
criterion_main!(benches);
