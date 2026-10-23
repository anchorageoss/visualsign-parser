#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Replays real mainnet transactions through the parser.
//!
//! `tests/fixtures/mainnet/mainnet_txs.json` holds six confirmed mainnet
//! transactions (one per script type, from block 972688) with the raw
//! previous transactions they spend and the block explorer's own view of each
//! (fee, output addresses and values, prevout addresses and values); see
//! `gen_mainnet_fixtures.py`. Each test rebuilds an unsigned PSBT from the raw
//! transactions and checks the parser's rendering against the explorer, which
//! is an independent source. Every txid is recomputed from the raw hex first.

mod common;

use base64::Engine as _;
use bitcoin::consensus::encode::deserialize_hex;
use bitcoin::psbt::Psbt;
use bitcoin::{Network, ScriptBuf, Transaction, Witness};
use common::{Rendered, RenderedFee, render_no_panic};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct ExplorerInput {
    txid: String,
    vout: u32,
    address: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    value_sat: u64,
}

#[derive(Deserialize)]
struct ExplorerOutput {
    address: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    script_hex: String,
    value_sat: u64,
}

#[derive(Deserialize)]
struct Explorer {
    fee_sat: u64,
    inputs: Vec<ExplorerInput>,
    outputs: Vec<ExplorerOutput>,
}

#[derive(Deserialize)]
struct Fixture {
    label: String,
    txid: String,
    tx_hex: String,
    prev_txs: BTreeMap<String, String>,
    explorer: Explorer,
}

#[derive(Deserialize)]
struct Doc {
    txs: Vec<Fixture>,
}

fn load() -> Vec<Fixture> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mainnet/mainnet_txs.json"
    );
    let doc: Doc = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    doc.txs
}

fn tx(hex: &str, txid: &str) -> Transaction {
    let tx: Transaction = deserialize_hex(hex).unwrap();
    assert_eq!(tx.compute_txid().to_string(), txid, "fixture txid mismatch");
    tx
}

/// How much UTXO data the rebuilt PSBT carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Utxos {
    /// `non_witness_utxo` on every input (plus `witness_utxo` on segwit inputs),
    /// what Bitcoin Core produces today.
    Full,
    /// `witness_utxo` only on segwit inputs (legacy inputs still need the
    /// previous transaction). The pre-CVE-2020-14199 shape.
    WitnessOnly,
}

fn build_psbt(f: &Fixture, utxos: Utxos) -> String {
    let mut unsigned = tx(&f.tx_hex, &f.txid);
    for input in &mut unsigned.input {
        input.script_sig = ScriptBuf::new();
        input.witness = Witness::new();
    }
    let mut psbt = Psbt::from_unsigned_tx(unsigned.clone()).unwrap();
    assert_eq!(unsigned.input.len(), f.explorer.inputs.len(), "{}", f.label);
    for (i, txin) in unsigned.input.iter().enumerate() {
        let prev_id = txin.previous_output.txid.to_string();
        let e = &f.explorer.inputs[i];
        assert_eq!(
            (prev_id.as_str(), txin.previous_output.vout),
            (e.txid.as_str(), e.vout)
        );
        let prev = tx(&f.prev_txs[&prev_id], &prev_id);
        let out = prev.output[txin.previous_output.vout as usize].clone();
        // The only P2SH prevouts in the fixture set are P2SH-P2WPKH.
        let segwit = out.script_pubkey.is_witness_program()
            || (out.script_pubkey.is_p2sh() && f.label == "p2sh-p2wpkh");
        if segwit {
            psbt.inputs[i].witness_utxo = Some(out);
        }
        if utxos == Utxos::Full || psbt.inputs[i].witness_utxo.is_none() {
            psbt.inputs[i].non_witness_utxo = Some(prev);
        }
    }
    base64::engine::general_purpose::STANDARD.encode(psbt.serialize())
}

fn assert_matches_explorer(f: &Fixture, r: &Rendered) {
    let ctx = format!("{} ({})", f.label, f.txid);
    let e = &f.explorer;
    assert_eq!(r.fee.sat(), Some(e.fee_sat), "{ctx}: fee");
    assert_eq!(r.inputs.len(), e.inputs.len(), "{ctx}: input count");
    for (i, (got, want)) in r.inputs.iter().zip(&e.inputs).enumerate() {
        assert_eq!(
            got.address, want.address,
            "{ctx}: input {i} address ({})",
            want.kind
        );
        assert_eq!(
            got.amount_sat,
            Some(want.value_sat),
            "{ctx}: input {i} amount"
        );
    }
    assert_eq!(r.outputs.len(), e.outputs.len(), "{ctx}: output count");
    for (i, (got, want)) in r.outputs.iter().zip(&e.outputs).enumerate() {
        assert_eq!(got.amount_sat, want.value_sat, "{ctx}: output {i} amount");
        assert_eq!(
            got.address, want.address,
            "{ctx}: output {i} address ({})",
            want.kind
        );
        if want.address.is_none() {
            assert_eq!(
                got.script_hex.as_deref(),
                Some(want.script_hex.as_str()),
                "{ctx}: output {i} script ({})",
                want.kind
            );
        }
        assert!(
            !got.claims_change,
            "{ctx}: output {i} claims change without derivation data"
        );
    }
}

#[test]
fn fixture_covers_each_script_type() {
    let fixtures = load();
    let mut input_types: Vec<&str> = fixtures
        .iter()
        .flat_map(|f| f.explorer.inputs.iter().map(|i| i.kind.as_str()))
        .collect();
    input_types.sort_unstable();
    input_types.dedup();
    for t in ["p2pkh", "p2sh", "v0_p2wpkh", "v0_p2wsh", "v1_p2tr"] {
        assert!(input_types.contains(&t), "no {t} input in fixtures");
    }
    let mut output_types: Vec<&str> = fixtures
        .iter()
        .flat_map(|f| f.explorer.outputs.iter().map(|o| o.kind.as_str()))
        .collect();
    output_types.sort_unstable();
    output_types.dedup();
    for t in [
        "p2pkh",
        "p2sh",
        "v0_p2wpkh",
        "v0_p2wsh",
        "v1_p2tr",
        "op_return",
    ] {
        assert!(output_types.contains(&t), "no {t} output in fixtures");
    }
}

/// With full previous transactions every amount is verified, so the fee is.
#[test]
fn mainnet_full_utxos_match_explorer_and_fee_is_verified() {
    for f in load() {
        let r = render_no_panic(&build_psbt(&f, Utxos::Full), Network::Bitcoin).unwrap();
        assert_matches_explorer(&f, &r);
        assert!(
            matches!(r.fee, RenderedFee::Verified(_)),
            "{}: fee not verified with full UTXOs: {:?}",
            f.label,
            r.fee
        );
        for (i, input) in r.inputs.iter().enumerate() {
            assert_eq!(
                input.source, "verified against previous transaction",
                "{} input {i}",
                f.label
            );
        }
    }
}

/// With witness_utxo only, amounts are claims: the fee must be flagged
/// unverified unless every input is taproot (BIP-341 commits to all amounts).
#[test]
fn mainnet_witness_utxo_only_flags_fee_unless_all_taproot() {
    for f in load() {
        let r = render_no_panic(&build_psbt(&f, Utxos::WitnessOnly), Network::Bitcoin).unwrap();
        assert_matches_explorer(&f, &r);
        let all_taproot = f.explorer.inputs.iter().all(|i| i.kind == "v1_p2tr");
        let all_legacy = f.explorer.inputs.iter().all(|i| i.kind == "p2pkh");
        if all_taproot || all_legacy {
            assert!(
                matches!(r.fee, RenderedFee::Verified(_)),
                "{}: {:?}",
                f.label,
                r.fee
            );
        } else {
            assert!(
                matches!(r.fee, RenderedFee::Unverified { .. }),
                "{}: claimed-only fee shown as verified: {:?}",
                f.label,
                r.fee
            );
        }
    }
}
