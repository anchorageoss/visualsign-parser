#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Replays the official PSBT test vectors (BIP-174, BIP-370, BIP-371) through
//! the parser.
//!
//! `tests/fixtures/vectors/bip_vectors.json` is extracted by
//! `gen_bip_vectors.py` from the BIP text at the commit and sha256 recorded in
//! the file. Every invalid vector must be rejected, every valid v0 vector must
//! parse, and for valid vectors the rendered outputs, input amounts and fee must
//! match what rust-bitcoin's own PSBT accessors report. PSBTv2 (BIP-370) is not
//! supported by the parser, so all of its vectors must be refused, never
//! misread as v0.

mod common;

use base64::Engine as _;
use bitcoin::psbt::Psbt;
use bitcoin::{Address, Network};
use common::{RenderedFee, render_no_panic};
use serde::Deserialize;
use std::collections::BTreeMap;

const BIPS_COMMIT: &str = "c0c5aea41e6a334d34db4ca87ae53bbfd7f268d1";

/// BIP-370 invalid vectors the parser currently accepts: v0 PSBTs that carry
/// PSBTv2-only keys. BIP-370 says these must be rejected; rust-bitcoin 0.32
/// keeps the keys as unknown entries and the parser renders the PSBT.
/// Finding on #577; `v0_psbt_with_v2_only_keys_is_rejected` pins the fix.
const KNOWN_ACCEPTED_INVALID: &[&str] = &[
    "PSBTv0 but with PSBT_GLOBAL_TX_VERSION.",
    "PSBTv0 but with PSBT_GLOBAL_FALLBACK_LOCKTIME.",
    "PSBTv0 but with PSBT_GLOBAL_INPUT_COUNT.",
    "PSBTv0 but with PSBT_GLOBAL_OUTPUT_COUNT.",
    "PSBTv0 but with PSBT_GLOBAL_TX_MODIFIABLE.",
    "PSBTv0 but with PSBT_IN_PREVIOUS_TXID.",
    "PSBTv0 but with PSBT_IN_OUTPUT_INDEX.",
    "PSBTv0 but with PSBT_IN_SEQUENCE.",
    "PSBTv0 but with PSBT_IN_REQUIRED_TIME_LOCKTIME.",
    "PSBTv0 but with PSBT_IN_REQUIRED_HEIGHT_LOCKTIME.",
    "PSBTv0 but with PSBT_OUT_AMOUNT.",
    "PSBTv0 but with PSBT_OUT_SCRIPT.",
];

/// Valid vectors the parser currently rejects. BIP-174's "PSBT with 0 inputs"
/// is refused as "outputs exceed inputs" instead of rendering with an unknown
/// fee. Finding on #577; `zero_input_psbt_parses` pins the fix.
const KNOWN_REJECTED_VALID: &[&str] = &["PSBT with 0 inputs"];

#[derive(Deserialize)]
struct Source {
    repo: String,
    commit: String,
    sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Vector {
    bip: String,
    status: String,
    case: String,
    psbt_base64: String,
}

#[derive(Deserialize)]
struct Doc {
    source: Source,
    vectors: Vec<Vector>,
}

fn load() -> Doc {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vectors/bip_vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn count(doc: &Doc, bip: &str, status: &str) -> usize {
    doc.vectors
        .iter()
        .filter(|v| v.bip == bip && v.status == status)
        .count()
}

#[test]
fn fixture_is_the_pinned_bip_text() {
    let doc = load();
    assert_eq!(doc.source.repo, "bitcoin/bips");
    assert_eq!(doc.source.commit, BIPS_COMMIT);
    assert_eq!(doc.source.sha256.len(), 3);
    // Counts as listed in the BIPs' "Test Vectors" sections. BIP-174's valid
    // set includes the PSBTs of its creator/updater/signer/combiner/finalizer
    // walkthrough; BIP-370 lists one valid PSBT twice.
    assert_eq!(count(&doc, "bip-0174", "invalid"), 20);
    assert_eq!(count(&doc, "bip-0174", "valid"), 25);
    assert_eq!(count(&doc, "bip-0371", "invalid"), 11);
    assert_eq!(count(&doc, "bip-0371", "valid"), 6);
    assert_eq!(count(&doc, "bip-0370", "invalid"), 24);
    assert_eq!(count(&doc, "bip-0370", "valid"), 23);
}

#[test]
fn invalid_vectors_are_rejected() {
    let doc = load();
    let mut accepted = Vec::new();
    let mut known = 0;
    for v in doc.vectors.iter().filter(|v| v.status == "invalid") {
        if render_no_panic(&v.psbt_base64, Network::Bitcoin).is_ok() {
            if v.bip == "bip-0370" && KNOWN_ACCEPTED_INVALID.contains(&v.case.as_str()) {
                known += 1;
            } else {
                accepted.push(format!("{}: {}", v.bip, v.case));
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid vectors accepted:\n{}",
        accepted.join("\n")
    );
    assert_eq!(
        known,
        KNOWN_ACCEPTED_INVALID.len(),
        "a known-accepted invalid vector is now rejected: remove it from KNOWN_ACCEPTED_INVALID"
    );
}

#[test]
#[ignore = "finding on #577: v0 PSBTs with PSBTv2-only keys are accepted (BIP-370 says invalid)"]
fn v0_psbt_with_v2_only_keys_is_rejected() {
    let doc = load();
    for v in doc
        .vectors
        .iter()
        .filter(|v| v.bip == "bip-0370" && KNOWN_ACCEPTED_INVALID.contains(&v.case.as_str()))
    {
        assert!(
            render_no_panic(&v.psbt_base64, Network::Bitcoin).is_err(),
            "{} accepted",
            v.case
        );
    }
}

#[test]
#[ignore = "finding on #577: BIP-174 'PSBT with 0 inputs' is rejected as 'outputs exceed inputs'"]
fn zero_input_psbt_parses() {
    let doc = load();
    let v = doc
        .vectors
        .iter()
        .find(|v| v.case == "PSBT with 0 inputs")
        .unwrap();
    let rendered = render_no_panic(&v.psbt_base64, Network::Bitcoin).unwrap();
    assert_eq!(rendered.fee, RenderedFee::Unknown);
}

/// Genuine PSBTv2 vectors (BIP-370 valid set): unsupported, so refused at decode.
#[test]
fn psbt_v2_is_refused() {
    let doc = load();
    for v in doc
        .vectors
        .iter()
        .filter(|v| v.bip == "bip-0370" && v.status == "valid")
    {
        let err = render_no_panic(&v.psbt_base64, Network::Bitcoin)
            .expect_err(&format!("PSBTv2 vector parsed: {}", v.case));
        assert!(
            err.contains("Failed to parse PSBT"),
            "{}: unexpected error {err}",
            v.case
        );
    }
}

/// Compare every valid v0 vector with rust-bitcoin's own view of the same bytes.
#[test]
fn valid_vectors_match_rust_bitcoin() {
    let doc = load();
    let mut checked_fee = 0;
    let mut rejected = Vec::new();
    let mut known = 0;
    for v in doc
        .vectors
        .iter()
        .filter(|v| v.status == "valid" && v.bip != "bip-0370")
    {
        let ctx = format!("{}: {}", v.bip, v.case);
        let rendered = match render_no_panic(&v.psbt_base64, Network::Bitcoin) {
            Ok(r) => r,
            Err(e) if KNOWN_REJECTED_VALID.contains(&v.case.as_str()) => {
                let _ = e;
                known += 1;
                continue;
            }
            Err(e) => {
                rejected.push(format!("{ctx}: {e}"));
                continue;
            }
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&v.psbt_base64)
            .unwrap();
        let psbt = Psbt::deserialize(&bytes).unwrap();
        let tx = &psbt.unsigned_tx;

        assert_eq!(rendered.outputs.len(), tx.output.len(), "{ctx}");
        for (i, (out, got)) in tx.output.iter().zip(&rendered.outputs).enumerate() {
            assert_eq!(got.amount_sat, out.value.to_sat(), "{ctx}: output {i}");
            let want = Address::from_script(&out.script_pubkey, Network::Bitcoin)
                .ok()
                .map(|a| a.to_string());
            assert_eq!(got.address, want, "{ctx}: output {i} address");
            if want.is_none() {
                assert_eq!(
                    got.script_hex.as_deref(),
                    Some(out.script_pubkey.to_hex_string().as_str()),
                    "{ctx}: output {i} script"
                );
            }
            let meta = &psbt.outputs[i];
            let derives = !meta.bip32_derivation.is_empty() || !meta.tap_key_origins.is_empty();
            assert_eq!(got.claims_change, derives, "{ctx}: output {i} change claim");
        }

        assert_eq!(rendered.inputs.len(), tx.input.len(), "{ctx}");
        for (i, got) in rendered.inputs.iter().enumerate() {
            let want = psbt.spend_utxo(i).ok().map(|o| o.value.to_sat());
            assert_eq!(got.amount_sat, want, "{ctx}: input {i} amount");
            let expected_source = if psbt.inputs[i].non_witness_utxo.is_some() {
                "verified against previous transaction"
            } else if psbt.inputs[i].witness_utxo.is_some() {
                "claimed by witness_utxo only"
            } else {
                "missing"
            };
            assert_eq!(got.source, expected_source, "{ctx}: input {i} source");
        }

        match psbt.fee() {
            Ok(fee) => {
                checked_fee += 1;
                assert_eq!(rendered.fee.sat(), Some(fee.to_sat()), "{ctx}: fee");
                // A fee built only from witness_utxo amounts must never be shown
                // as verified unless every input is taproot (BIP-341 commits to
                // all input amounts).
                let all_full = psbt.inputs.iter().all(|i| i.non_witness_utxo.is_some());
                let all_taproot = (0..tx.input.len())
                    .all(|i| psbt.spend_utxo(i).is_ok_and(|o| o.script_pubkey.is_p2tr()));
                if !all_full && !all_taproot {
                    assert!(
                        matches!(rendered.fee, RenderedFee::Unverified { .. }),
                        "{ctx}: claimed-only fee shown as verified"
                    );
                }
            }
            Err(_) => assert_eq!(rendered.fee, RenderedFee::Unknown, "{ctx}: fee"),
        }
    }
    assert!(
        rejected.is_empty(),
        "valid vectors rejected:\n{}",
        rejected.join("\n")
    );
    assert_eq!(
        known,
        KNOWN_REJECTED_VALID.len(),
        "a known-rejected valid vector now parses: remove it from KNOWN_REJECTED_VALID"
    );
    assert!(checked_fee > 0, "no vector had full UTXO data");
}
