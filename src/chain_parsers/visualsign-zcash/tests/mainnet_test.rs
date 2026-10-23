//! Replays confirmed mainnet transactions (`tests/fixtures/mainnet/`).
//!
//! An explorer sees only the public parts of a Zcash transaction, so this
//! layer checks only those: the txid, the ZIP-244 digests, transparent inputs
//! and outputs, per-pool value balances, the fee, and action/spend/output
//! counts. Shielded recipients, values and memos are encrypted on chain; they
//! are checked against construction and viewing-key decryption in
//! `ground_truth_test.rs` and `regtest.rs`, not here.
//!
//! For each transaction the test rebuilds the PCZT a coordinator would have
//! handed the signer (every effecting field, the spent outputs, no note
//! plaintexts) and then requires:
//!
//! - the raw transaction hashes to its pinned txid;
//! - the PCZT's effects carry the same counts and value balances as the node's
//!   decoding, and the transparent inputs and outputs match the explorer;
//! - the `Signer` digests behind the parser's sighash fields verify the
//!   transaction's own on-chain signatures (ECDSA for transparent inputs,
//!   RedPallas / RedJubjub spend-authorization and binding signatures), and
//!   the shielded digest equals the txid when there are no transparent inputs
//!   (ZIP 244);
//! - transparent-only transactions render with the explorer's recipients,
//!   amounts and fee; shielded ones are refused, since a PCZT without note
//!   plaintexts cannot be checked.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Prevout, pczt_from_tx, render, script_address};
use serde::Deserialize;
use std::fs;
use visualsign::vsptrait::VisualSignError;
use zcash_keys::encoding::AddressCodec;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, Network};

const MAIN: Network = Network::MainNetwork;

#[derive(Deserialize)]
struct Doc {
    txs: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    label: String,
    txid: String,
    block_height: u32,
    tx_hex: String,
    explorer: Explorer,
}

#[derive(Deserialize)]
struct Explorer {
    version: u32,
    fee_zat: Option<u64>,
    inputs: Vec<Coin>,
    outputs: Vec<Coin>,
    sapling: SaplingCounts,
    orchard: PoolCounts,
    ironwood: PoolCounts,
}

#[derive(Deserialize)]
struct Coin {
    address: Option<String>,
    script_hex: String,
    value_zat: u64,
}

#[derive(Deserialize)]
struct SaplingCounts {
    spends: usize,
    outputs: usize,
    value_balance: i64,
}

#[derive(Deserialize)]
struct PoolCounts {
    actions: usize,
    value_balance: i64,
}

fn load() -> Vec<Fixture> {
    let path = format!(
        "{}/tests/fixtures/mainnet/mainnet_txs.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str::<Doc>(&fs::read_to_string(path).unwrap())
        .unwrap()
        .txs
}

fn tx(f: &Fixture) -> Transaction {
    let raw = hex::decode(&f.tx_hex).unwrap();
    let branch = BranchId::for_height(&MAIN, BlockHeight::from_u32(f.block_height));
    let tx = Transaction::read(&raw[..], branch).unwrap();
    // Display order is the reverse of the internal byte order.
    let mut txid = tx.txid().as_ref().to_vec();
    txid.reverse();
    assert_eq!(hex::encode(txid), f.txid, "{}: txid", f.label);
    tx
}

fn prevouts(f: &Fixture) -> Vec<Prevout> {
    f.explorer
        .inputs
        .iter()
        .map(|c| Prevout {
            value: c.value_zat,
            script_pubkey: hex::decode(&c.script_hex).unwrap(),
        })
        .collect()
}

/// (signature without its hash-type byte, hash type, public key) from a
/// P2PKH scriptSig.
fn p2pkh_script_sig(script: &[u8]) -> (Vec<u8>, u8, Vec<u8>) {
    let sig_len = script[0] as usize;
    let sig = &script[1..1 + sig_len];
    let pk_len = script[1 + sig_len] as usize;
    let pk = &script[2 + sig_len..2 + sig_len + pk_len];
    assert_eq!(2 + sig_len + pk_len, script.len(), "not a P2PKH scriptSig");
    (
        sig[..sig.len() - 1].to_vec(),
        sig[sig.len() - 1],
        pk.to_vec(),
    )
}

#[test]
fn fixture_covers_each_shape() {
    let txs = load();
    let labels: Vec<&str> = txs.iter().map(|f| f.label.as_str()).collect();
    for want in [
        "v5-transparent-1in-2out",
        "v5-transparent-5in-2out",
        "v6-transparent-1in-2out",
        "v5-sapling-z2z",
        "v5-sapling-t2z",
        "v5-sapling-z2t",
        "v5-orchard-z2z",
        "v5-orchard-t2z",
        "v5-orchard-z2t",
        "v6-ironwood-z2z",
    ] {
        assert!(labels.contains(&want), "missing {want}");
    }
    for f in &txs {
        assert_eq!(tx(f).version().header() & 0x7fff_ffff, f.explorer.version);
    }
}

#[test]
fn structure_matches_the_node_decoding() {
    for f in load() {
        let t = tx(&f);
        let pczt = pczt_from_tx(&t, &prevouts(&f), 133);
        let effects = pczt::Pczt::parse(&pczt).unwrap().into_effects().unwrap();
        let e = &f.explorer;
        let l = &f.label;

        let sapling = effects.sapling_bundle();
        assert_eq!(
            sapling.map_or(0, |b| b.shielded_spends().len()),
            e.sapling.spends,
            "{l}"
        );
        assert_eq!(
            sapling.map_or(0, |b| b.shielded_outputs().len()),
            e.sapling.outputs,
            "{l}"
        );
        assert_eq!(
            sapling.map_or(0, |b| i64::from(*b.value_balance())),
            e.sapling.value_balance,
            "{l}"
        );
        for (bundle, counts) in [
            (effects.orchard_bundle(), &e.orchard),
            (effects.ironwood_bundle(), &e.ironwood),
        ] {
            assert_eq!(
                bundle.map_or(0, |b| b.actions().len()),
                counts.actions,
                "{l}"
            );
            assert_eq!(
                bundle.map_or(0, |b| i64::from(*b.value_balance())),
                counts.value_balance,
                "{l}"
            );
        }

        let t_out: Vec<(Option<String>, u64)> = effects
            .transparent_bundle()
            .map(|b| {
                b.vout
                    .iter()
                    .map(|o| {
                        (
                            script_address(&o.script_pubkey().0.0).map(|a| a.encode(&MAIN)),
                            u64::from(o.value()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let explorer_out: Vec<(Option<String>, u64)> = e
            .outputs
            .iter()
            .map(|o| (o.address.clone(), o.value_zat))
            .collect();
        assert_eq!(t_out, explorer_out, "{l}: transparent outputs");

        // Public fee: transparent in - transparent out + every value balance.
        let t_in: i64 = e.inputs.iter().map(|c| c.value_zat as i64).sum();
        let t_out: i64 = e.outputs.iter().map(|c| c.value_zat as i64).sum();
        let fee = t_in - t_out
            + e.sapling.value_balance
            + e.orchard.value_balance
            + e.ironwood.value_balance;
        assert!(fee > 0, "{l}: fee {fee}");
        if let Some(explorer_fee) = e.fee_zat {
            assert_eq!(fee as u64, explorer_fee, "{l}: fee");
        }
    }
}

#[test]
fn signer_digests_verify_the_on_chain_signatures() {
    for f in load() {
        let t = tx(&f);
        let l = &f.label;
        let pczt = pczt_from_tx(&t, &prevouts(&f), 133);
        let signer = pczt::roles::signer::Signer::new(pczt::Pczt::parse(&pczt).unwrap()).unwrap();
        let shielded = signer.shielded_sighash();

        let mut verified = 0;
        if let Some(b) = t.transparent_bundle() {
            for (i, input) in b.vin.iter().enumerate() {
                let (der, hash_type, pk) = p2pkh_script_sig(&input.script_sig().0.0);
                assert_eq!(hash_type, 1, "{l}: input {i} is not SIGHASH_ALL");
                let digest = signer.transparent_sighash(i).unwrap();
                let mut sig = secp256k1::ecdsa::Signature::from_der(&der).unwrap();
                sig.normalize_s();
                let pk = secp256k1::PublicKey::from_slice(&pk).unwrap();
                secp256k1::ecdsa::verify(&sig, secp256k1::Message::from_digest(digest), &pk)
                    .unwrap_or_else(|e| panic!("{l}: input {i} signature: {e}"));
                verified += 1;
            }
        }
        if t.transparent_bundle().is_none_or(|b| b.vin.is_empty()) {
            // ZIP 244: with no transparent inputs the shielded digest is the txid.
            assert_eq!(
                shielded.as_slice(),
                t.txid().as_ref(),
                "{l}: sighash != txid"
            );
        }
        if let Some(b) = t.sapling_bundle() {
            for s in b.shielded_spends() {
                s.rk().verify(&shielded, s.spend_auth_sig()).unwrap();
                verified += 1;
            }
            let spends: sapling_crypto::value::CommitmentSum =
                b.shielded_spends().iter().map(|s| s.cv()).sum();
            let outputs: sapling_crypto::value::CommitmentSum =
                b.shielded_outputs().iter().map(|o| o.cv()).sum();
            let bvk = (spends - outputs).into_bvk(i64::from(*b.value_balance()));
            bvk.verify(&shielded, &b.authorization().binding_sig)
                .unwrap_or_else(|e| panic!("{l}: Sapling binding signature: {e:?}"));
            verified += 1;
        }
        for bundle in [t.orchard_bundle(), t.ironwood_bundle()]
            .into_iter()
            .flatten()
        {
            for a in bundle.actions() {
                a.rk().verify(&shielded, a.authorization()).unwrap();
                verified += 1;
            }
            bundle
                .binding_validating_key()
                .verify(&shielded, bundle.authorization().binding_signature())
                .unwrap_or_else(|e| panic!("{l}: binding signature: {e:?}"));
            verified += 1;
        }
        assert!(verified > 0, "{l}");
    }
}

#[test]
fn transparent_transactions_render_as_the_explorer_shows_them() {
    let mut rendered = 0;
    for f in load() {
        let t = tx(&f);
        let l = &f.label;
        let pczt = pczt_from_tx(&t, &prevouts(&f), 133);
        let e = &f.explorer;
        let Some(fee) = e.fee_zat else {
            // Shielded: the PCZT has no note plaintexts, so nothing can be
            // checked and the parser must say so instead of rendering.
            assert!(
                matches!(render(&pczt), Err(VisualSignError::MissingData(_))),
                "{l}: shielded PCZT without note plaintexts was rendered"
            );
            continue;
        };
        let r = render(&pczt).unwrap_or_else(|err| panic!("{l}: {err}"));
        assert_eq!(r.network, "Zcash mainnet");
        assert_eq!(r.fee, fee, "{l}: fee");
        let outputs: Vec<(String, u64)> = r
            .outputs
            .iter()
            .map(|o| (o.address.clone(), o.zat))
            .collect();
        let explorer: Vec<(String, u64)> = e
            .outputs
            .iter()
            .map(|o| (o.address.clone().unwrap(), o.value_zat))
            .collect();
        assert_eq!(outputs, explorer, "{l}: outputs");
        let inputs: Vec<(String, u64)> = e
            .inputs
            .iter()
            .map(|c| {
                (
                    format!("{} (P2PKH)", c.address.clone().unwrap()),
                    c.value_zat,
                )
            })
            .collect();
        assert_eq!(r.transparent_inputs, inputs, "{l}: inputs");
        let signer = pczt::roles::signer::Signer::new(pczt::Pczt::parse(&pczt).unwrap()).unwrap();
        for (i, shown) in r.transparent_sighashes.iter().enumerate() {
            assert_eq!(
                *shown,
                hex::encode(signer.transparent_sighash(i).unwrap()),
                "{l}"
            );
        }
        rendered += 1;
    }
    assert_eq!(rendered, 3);
}
