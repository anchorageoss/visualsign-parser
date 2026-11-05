//! One-time cost of decoding every compiled-in IDL: what a process pays once
//! to build the preset and built-in IDL records.
//!
//! The per-request cost of the parse path, with and without the intermediate
//! output, is measured by `benches/solana_stages.rs`; this bench keeps its own
//! group (`solana/idl_load`) so the two never share Criterion result
//! directories.
//!
//! Run:
//!   cargo bench -p visualsign-solana --bench idl_cache

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// Every compiled-in IDL as JSON: the preset IDLs this crate ships, exactly as
/// the parse path loads them, plus `solana_parser`'s built-in program IDLs.
fn compiled_in_idl_jsons() -> Vec<&'static str> {
    let mut jsons: Vec<&'static str> = visualsign_solana::compiled_in_preset_idl_jsons().collect();
    jsons.extend(
        solana_parser::construct_custom_idl_records_map()
            .expect("built-in IDL records")
            .into_values()
            .filter_map(|record| record.program_type.map(|t| t.idl_json())),
    );
    jsons
}

/// Decoding every compiled-in IDL from JSON. A preset that fails to decode is
/// dropped here as the parse path drops it, rather than failing the bench.
fn bench_idl_load(c: &mut Criterion) {
    let jsons = compiled_in_idl_jsons();
    c.bench_function("solana/idl_load/all", |b| {
        b.iter(|| {
            for json in &jsons {
                black_box(solana_parser::decode_idl_data(black_box(json)).ok());
            }
        });
    });
}

criterion_group!(benches, bench_idl_load);
criterion_main!(benches);
