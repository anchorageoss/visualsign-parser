//! Per-stage latency benchmarks for the parse path.
//!
//! Each stage between "raw transaction bytes arrive" and "a signature exists"
//! is measured in isolation so a regression can be attributed to a stage
//! rather than to the pipeline as a whole. The final group measures the
//! end-to-end `parse()` entry point the gRPC service actually calls.
//!
//! Stages, in pipeline order:
//!   1. `decode`                    raw string -> chain transaction type
//!   2. `convert`                   transaction -> SignablePayload
//!   3. `convert_with_intermediate` same, plus the intermediate representation
//!   4. `validate_charset`          defense-in-depth charset check
//!   5. `serialize_json`            SignablePayload -> JSON string
//!   6. `digest`                    borsh encode + SHA-256 of the signed preimage
//!   7. `sign_p256`                 P-256 signature over the 32-byte digest
//!
//! Plus `registry_construction`, which `parse()` performs per request today.
//!
//! Run:
//!   cargo bench -p parser_app --bench parse_stages
//!   cargo bench -p parser_app --bench parse_stages -- decode   # one stage
//!
//! Profile (statistics off, runs the closure for N seconds under a profiler):
//!   cargo bench -p parser_app --bench parse_stages -- --profile-time 10
//!   perf record -g target/release/deps/parse_stages-<hash> --bench --profile-time 10
//!   samply record target/release/deps/parse_stages-<hash> --bench --profile-time 10

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use generated::parser::{Chain as ProtoChain, ParseRequest, ParsedTransactionPayload};
use parser_app::{registry::create_registry, routes::parse::parse};
use qos_crypto::sha_256;
use qos_p256::P256Pair;
use std::{fs, hint::black_box, path::PathBuf};
use visualsign::{
    registry::Chain as RegistryChain,
    vsptrait::{Transaction, VisualSignConverter, VisualSignOptions},
};
use visualsign_ethereum::{EthereumTransactionWrapper, EthereumVisualSignConverter};
use visualsign_solana::{
    SolanaTransactionWrapper, SolanaVisualSignConverter,
    utils::create_transaction_with_empty_signatures,
};

/// A known System Program transfer (1_000_000_000 lamports), base64 message.
/// Solana is the chain that actually emits `intermediate_output`, so it is the
/// one where the convert / convert_with_intermediate delta is meaningful.
const SOLANA_TRANSFER_MESSAGE: &str = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";

/// Fixtures spanning the interesting cost range: a bare transfer, a typical
/// EIP-1559 transaction, and a Uniswap v3 swap that exercises the protocol
/// decoders. The spread between the first and last is the cost of decoder
/// coverage, which is the number worth watching.
const FIXTURES: &[(&str, &str)] = &[
    ("legacy_transfer", "legacy.input"),
    ("eip1559", "1559.input"),
    ("uniswap_v3_swap", "uniswap-v3swap.input"),
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../chain_parsers/visualsign-ethereum/tests/fixtures")
}

fn load(file: &str) -> String {
    let path = fixture_dir().join(file);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
        .trim()
        .to_string()
}

fn options(include_intermediate_output: bool) -> VisualSignOptions {
    VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: None,
        developer_config: None,
        include_intermediate_output,
    }
}

/// Stage 1: raw string -> chain transaction type.
fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    for (name, file) in FIXTURES {
        let payload = load(file);
        group.bench_with_input(BenchmarkId::from_parameter(name), &payload, |b, p| {
            b.iter(|| black_box(EthereumTransactionWrapper::from_string(black_box(p)).unwrap()));
        });
    }
    group.finish();
}

/// Stages 2 and 3: transaction -> SignablePayload, without and with the
/// intermediate representation. The delta between the two groups is the
/// marginal cost of producing `intermediate_output`.
fn bench_convert(c: &mut Criterion) {
    let converter = EthereumVisualSignConverter::new();

    for (group_name, with_intermediate) in [("convert", false), ("convert_with_intermediate", true)]
    {
        let mut group = c.benchmark_group(group_name);
        for (name, file) in FIXTURES {
            let tx = EthereumTransactionWrapper::from_string(&load(file)).unwrap();
            group.bench_with_input(BenchmarkId::from_parameter(name), &tx, |b, tx| {
                // `to_visual_sign_payload` consumes the transaction, so clone in
                // untimed setup rather than inside the measured closure.
                b.iter_batched(
                    || tx.clone(),
                    |tx| {
                        black_box(
                            converter
                                .to_visual_sign_payload(tx, options(with_intermediate))
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

/// Stages 4, 5 and 6: the post-conversion work on the signing path.
fn bench_post_conversion(c: &mut Criterion) {
    let converter = EthereumVisualSignConverter::new();

    // Precompute inputs once. Criterion hands out one benchmark group at a
    // time from `&mut Criterion`, so the groups below run in sequence.
    let cases: Vec<(&str, String, _)> = FIXTURES
        .iter()
        .map(|(name, file)| {
            let raw = load(file);
            let tx = EthereumTransactionWrapper::from_string(&raw).unwrap();
            let payload = converter
                .to_visual_sign_payload(tx, options(false))
                .unwrap()
                .payload;
            (*name, raw, payload)
        })
        .collect();

    let mut validate = c.benchmark_group("validate_charset");
    for (name, _, payload) in &cases {
        validate.bench_with_input(BenchmarkId::from_parameter(name), payload, |b, p| {
            b.iter(|| {
                p.validate_charset().unwrap();
            });
        });
    }
    validate.finish();

    let mut serialize = c.benchmark_group("serialize_json");
    for (name, _, payload) in &cases {
        serialize.bench_with_input(BenchmarkId::from_parameter(name), payload, |b, p| {
            b.iter(|| black_box(serde_json::to_string(black_box(p)).unwrap()));
        });
    }
    serialize.finish();

    // Mirrors the preimage `parse()` signs: the borsh encoding of the parsed
    // payload, hashed with SHA-256.
    let mut digest = c.benchmark_group("digest");
    for (name, raw, payload) in &cases {
        let json = serde_json::to_string(payload).unwrap();
        let parsed = ParsedTransactionPayload {
            parsed_payload: json.clone(),
            input_payload_digest: qos_hex::encode(&sha_256(raw.as_bytes())),
            metadata_digest: qos_hex::encode(&sha_256(&[])),
            signable_payload: json,
            intermediate_output: Vec::new(),
        };
        digest.bench_with_input(BenchmarkId::from_parameter(name), &parsed, |b, p| {
            b.iter(|| black_box(sha_256(&borsh::to_vec(black_box(p)).unwrap())));
        });
    }
    digest.finish();
}

/// Stage 7, plus the per-request registry construction `parse()` performs.
fn bench_crypto_and_setup(c: &mut Criterion) {
    let key = P256Pair::generate().expect("keygen");
    let digest = sha_256(b"parse-path benchmark digest");

    c.bench_function("sign_p256", |b| {
        b.iter(|| black_box(key.sign(black_box(&digest)).unwrap()));
    });

    c.bench_function("registry_construction", |b| {
        b.iter(|| black_box(create_registry()));
    });
}

/// End to end: the entry point the gRPC service calls per request.
fn bench_end_to_end(c: &mut Criterion) {
    let key = P256Pair::generate().expect("keygen");

    for (group_name, with_intermediate) in [("parse", false), ("parse_with_intermediate", true)] {
        let mut group = c.benchmark_group(group_name);
        for (name, file) in FIXTURES {
            let request = ParseRequest {
                include_intermediate_output: with_intermediate,
                unsigned_payload: load(file),
                chain: ProtoChain::Ethereum as i32,
                chain_metadata: None,
            };
            group.bench_with_input(BenchmarkId::from_parameter(name), &request, |b, req| {
                b.iter(|| black_box(parse(black_box(req), &key).unwrap()));
            });
        }
        group.finish();
    }
}

/// Sanity check that the registry path and the direct converter path agree, so
/// the isolated stages above are measuring the same work `parse()` does.
fn bench_registry_dispatch(c: &mut Criterion) {
    let registry = create_registry();
    let payload = load("uniswap-v3swap.input");

    c.bench_function("registry_dispatch/uniswap_v3_swap", |b| {
        b.iter(|| {
            black_box(
                registry
                    .convert_transaction(
                        &RegistryChain::Ethereum,
                        black_box(&payload),
                        options(false),
                    )
                    .unwrap(),
            )
        });
    });
}

/// Solana: the chain that emits `intermediate_output`.
///
/// `intermediate_output` is currently produced by a SECOND, independent decode
/// of the raw message (`extract_solana_intermediate_output` ->
/// `parse_transaction_with_idls`) rather than being the intermediate
/// representation the `SignablePayload` is built from. The delta between
/// `solana/convert` and `solana/convert_with_intermediate` is the cost of that
/// re-parse, and is the baseline to beat when the structured decode becomes the
/// single source of truth.
fn bench_solana_intermediate(c: &mut Criterion) {
    let tx = create_transaction_with_empty_signatures(SOLANA_TRANSFER_MESSAGE);
    let key = P256Pair::generate().expect("keygen");

    let mut group = c.benchmark_group("solana");

    group.bench_function("decode", |b| {
        b.iter(|| black_box(SolanaTransactionWrapper::from_string(black_box(&tx)).unwrap()));
    });

    let wrapper = SolanaTransactionWrapper::from_string(&tx).unwrap();
    for (name, with_intermediate) in [("convert", false), ("convert_with_intermediate", true)] {
        group.bench_function(name, |b| {
            b.iter_batched(
                || wrapper.clone(),
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

    for (name, with_intermediate) in [("parse", false), ("parse_with_intermediate", true)] {
        let request = ParseRequest {
            include_intermediate_output: with_intermediate,
            unsigned_payload: tx.clone(),
            chain: ProtoChain::Solana as i32,
            chain_metadata: None,
        };
        group.bench_function(name, |b| {
            b.iter(|| black_box(parse(black_box(&request), &key).unwrap()));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_decode,
    bench_convert,
    bench_post_conversion,
    bench_crypto_and_setup,
    bench_end_to_end,
    bench_registry_dispatch,
    bench_solana_intermediate,
);
criterion_main!(benches);
