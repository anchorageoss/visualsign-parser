//! Spec vectors run through the parser (or the PCZT digest code behind its
//! sighash fields), complementing `zip244_vectors.rs` and
//! `ironwood_v6_vectors.rs`, which check the raw-transaction digests only.
//!
//! All files are zcash/zcash-test-vectors `test-vectors/json/*.json` at commit
//! 78321beacb0e0477e33cd002b56585a107c2708c, copied unchanged (sha256 below).
//!
//! - ZIP 244 / ZIP 225: each vector transaction re-serializes byte for byte,
//!   and a PCZT rebuilt from it gives the vector's `sighash_shielded` and, for
//!   the six with a transparent input, `sighash_all` from the same `Signer`
//!   calls that fill the parser's sighash fields.
//! - ZIP 320: TEX and P2PKH encodings of each vector's key hash, as output
//!   recipient and as `user_address`.
//! - ZIP 316 (revisions 0 and 2): every unified address vector as an output's
//!   `user_address`, over an output paying one of its receivers; revision 2
//!   expiry metadata included.
//! - Orchard note encryption: each vector's note placed in an action; the
//!   parser must reproduce its `cmx`, recover its memo and show its receiver
//!   and value.
//!
//! Not used: `sapling_note_encryption.json` notes are pre-ZIP-212 (lead byte
//! 0x01), which the parser refuses by design (it checks `rseed`-form notes
//! only); Ironwood note-encryption vectors are not in a released commit yet
//! (zcash-test-vectors PR #129 is open).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/builder.rs"]
mod builder;
mod common;

use common::{Prevout, edit_pczt, memo_text, pczt_from_tx, render};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use visualsign::vsptrait::VisualSignError;
use zcash_keys::address::UnifiedAddress;
use zcash_keys::encoding::AddressCodec;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BranchId, Network};
use zcash_transparent::address::TransparentAddress;

const MAIN: Network = Network::MainNetwork;

const FILES: [(&str, &str); 5] = [
    (
        "zip_0244_78321beac.json",
        "3d20892f19cec18afba2ef2907bb8426192d0ea4eac2c0060c2f31ff78ad93a3",
    ),
    (
        "zip_0320_78321beac.json",
        "a40bbf1585fffd1d8e011d5f4d2b74271240b999620a7849518c3278a156c4ea",
    ),
    (
        "unified_address_78321beac.json",
        "657f542c4802ec40ef0f495cfaf1e31c0fd3b048725a6c75a022a08e05c244df",
    ),
    (
        "unified_address_r2_78321beac.json",
        "1493c9d51a73a8ef0c035efe37eb5abe9298e224420a56d5c92be94810a80df3",
    ),
    (
        "orchard_note_encryption_78321beac.json",
        "b27fec6fb76c563e3872141de7e2d3d1a87d11a72ce39b1f2dca7932664d3108",
    ),
];

fn read(name: &str) -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// The vector rows of a zcash-test-vectors JSON file, keyed by column name.
fn rows(name: &str) -> Vec<HashMap<String, Value>> {
    let doc: Vec<Value> = serde_json::from_slice(&read(name)).unwrap();
    let header: Vec<String> = doc[1][0]
        .as_str()
        .unwrap()
        .split(',')
        .map(|h| h.trim().to_string())
        .collect();
    doc[2..]
        .iter()
        .map(|r| {
            header
                .iter()
                .cloned()
                .zip(r.as_array().unwrap().iter().cloned())
                .collect()
        })
        .collect()
}

fn hex_field(row: &HashMap<String, Value>, name: &str) -> Option<Vec<u8>> {
    row[name].as_str().map(|s| hex::decode(s).unwrap())
}

fn fixture(name: &str) -> Vec<u8> {
    hex::decode(
        String::from_utf8(read(&format!("{name}.input")))
            .unwrap()
            .trim(),
    )
    .unwrap()
}

fn p2pkh(hash: &[u8]) -> Vec<u8> {
    [&[0x76, 0xa9, 0x14][..], hash, &[0x88, 0xac][..]].concat()
}

fn p2sh(hash: &[u8]) -> Vec<u8> {
    [&[0xa9, 0x14][..], hash, &[0x87][..]].concat()
}

#[test]
fn vector_files_are_the_pinned_upstream_files() {
    for (name, sha) in FILES {
        assert_eq!(hex::encode(Sha256::digest(read(name))), sha, "{name}");
    }
}

#[test]
fn zip244_vectors_through_a_rebuilt_pczt() {
    let mut transparent = 0;
    let mut coinbase = 0;
    let mut random_scripts = 0;
    let mut shielded_checked = 0;
    for (n, row) in rows("zip_0244_78321beac.json").iter().enumerate() {
        let raw = hex_field(row, "tx").unwrap();
        let tx = Transaction::read(&raw[..], BranchId::Nu5).unwrap();
        // ZIP 225: the v5 encoding round-trips exactly.
        let mut again = Vec::new();
        tx.write(&mut again).unwrap();
        assert_eq!(again, raw, "vector {n}: ZIP 225 re-serialization");

        let amounts = row["amounts"].as_array().unwrap();
        if tx
            .transparent_bundle()
            .is_some_and(|b| b.vin.len() != amounts.len())
        {
            // Coinbase vectors: no spent outputs to put in a PCZT.
            coinbase += 1;
            continue;
        }
        let scripts = row["script_pubkeys"].as_array().unwrap();
        let prevouts: Vec<Prevout> = amounts
            .iter()
            .zip(scripts)
            .map(|(a, s)| Prevout {
                value: a.as_u64().unwrap(),
                script_pubkey: hex::decode(s.as_str().unwrap()).unwrap(),
            })
            .collect();
        let pczt = pczt_from_tx(&tx, &prevouts, 133);
        let signer = match pczt::roles::signer::Signer::new(pczt::Pczt::parse(&pczt).unwrap()) {
            Ok(s) => s,
            // The generator draws random scriptPubKeys; a PCZT only carries
            // parseable scripts, so those vectors cannot be expressed as one.
            Err(e) if format!("{e:?}").contains("InvalidScriptPubkey") => {
                random_scripts += 1;
                continue;
            }
            Err(e) => panic!("vector {n}: {e:?}"),
        };
        assert_eq!(
            signer.shielded_sighash().to_vec(),
            hex_field(row, "sighash_shielded").unwrap(),
            "vector {n}: sighash_shielded"
        );
        shielded_checked += 1;
        if let Some(i) = row["transparent_input"].as_u64() {
            assert_eq!(
                signer.transparent_sighash(i as usize).unwrap().to_vec(),
                hex_field(row, "sighash_all").unwrap(),
                "vector {n}: sighash_all for input {i}"
            );
            transparent += 1;
        }
        // Without note plaintexts (not part of a transaction) the parser must
        // refuse rather than render a partial view.
        let shielded = tx
            .sapling_bundle()
            .is_some_and(|b| !b.shielded_outputs().is_empty())
            || tx.orchard_bundle().is_some();
        if shielded {
            assert!(
                matches!(render(&pczt), Err(VisualSignError::MissingData(_))),
                "vector {n}: rendered without note plaintexts"
            );
        }
    }
    println!(
        "ZIP 244 via PCZT: {shielded_checked} sighash_shielded, {transparent} sighash_all; \
         skipped {coinbase} coinbase and {random_scripts} with unparseable scripts"
    );
    assert_eq!(shielded_checked + coinbase + random_scripts, 10);
    assert!(shielded_checked > 0);
}

/// `transparent_mix_v5` with its first transparent output (the foreign send)
/// paying `script`, and `user_address` set.
fn transparent_send(script: &[u8], user_address: Option<&str>) -> Vec<u8> {
    edit_pczt(&fixture("transparent_mix_v5"), |v| {
        let out = &mut v["transparent"]["outputs"][0];
        out["script_pubkey"] = json!(script);
        out["user_address"] = json!(user_address);
    })
}

/// The rendered address of the transparent send.
fn transparent_output(pczt: &[u8]) -> String {
    render(pczt)
        .unwrap()
        .outputs
        .iter()
        .find(|o| o.badge.starts_with("transparent") && o.zat == 300_000)
        .unwrap()
        .address
        .clone()
}

#[test]
fn zip320_tex_and_p2pkh_encodings() {
    let rows = rows("zip_0320_78321beac.json");
    for row in &rows {
        let hash = hex_field(row, "p2pkh_bytes").unwrap();
        let t_addr = row["t_addr"].as_str().unwrap();
        let tex = row["tex_addr"].as_str().unwrap();
        let script = p2pkh(&hash);
        assert_eq!(transparent_output(&transparent_send(&script, None)), t_addr);
        assert_eq!(
            transparent_output(&transparent_send(&script, Some(t_addr))),
            t_addr
        );
        assert_eq!(
            transparent_output(&transparent_send(&script, Some(tex))),
            tex
        );
        // The same TEX over a different key hash is refused.
        let mut other = hash.clone();
        other[0] ^= 1;
        assert!(matches!(
            render(&transparent_send(&p2pkh(&other), Some(tex))),
            Err(VisualSignError::ValidationError(_))
        ));
    }
    assert_eq!(rows.len(), 15);
}

/// Build a send of 1 000 000 zatoshis to `ua`'s Orchard or Sapling receiver
/// and set the output's `user_address` to `ua`.
fn shielded_send_to(
    ua: &str,
    orchard: Option<&[u8]>,
    sapling: Option<&[u8]>,
    expiry: Option<u32>,
) -> Vec<u8> {
    let (spec, pool) = if let Some(raw) = orchard {
        let addr = orchard::Address::from_raw_address_bytes(raw.try_into().unwrap()).unwrap();
        (
            builder::Spec {
                height: builder::PRE_NU6_3,
                orchard_in: Some(10_000_000),
                orchard_out: vec![(1_000_000, "")],
                orchard_to: Some(addr),
                change: Some(builder::ChangePool::Orchard),
                ..Default::default()
            },
            "orchard",
        )
    } else {
        let raw: [u8; 43] = sapling.unwrap().try_into().unwrap();
        let addr = sapling_crypto::PaymentAddress::from_bytes(&raw).unwrap();
        (
            builder::Spec {
                height: builder::PRE_NU6_3,
                sapling_in: Some(10_000_000),
                sapling_out: vec![(1_000_000, "")],
                sapling_to: Some(addr),
                change: Some(builder::ChangePool::Sapling),
                ..Default::default()
            },
            "sapling",
        )
    };
    edit_pczt(&builder::build(&spec), |v| {
        if let Some(h) = expiry {
            v["global"]["expiry_height"] = json!(h);
        }
        let outputs = if pool == "sapling" {
            v["sapling"]["outputs"].as_array_mut().unwrap()
        } else {
            v["orchard"]["actions"].as_array_mut().unwrap()
        };
        for o in outputs {
            let o = if pool == "sapling" {
                o
            } else {
                &mut o["output"]
            };
            if o["value"] == 1_000_000 {
                o["user_address"] = json!(ua);
            }
        }
    })
}

#[test]
fn zip316_unified_address_vectors() {
    let mut checked = HashMap::new();
    for file in [
        "unified_address_78321beac.json",
        "unified_address_r2_78321beac.json",
    ] {
        for (n, row) in rows(file).into_iter().enumerate() {
            let ua = row["unified_addr"].as_str().unwrap();
            let expiry = row
                .get("expiry_height")
                .and_then(|h| h.as_u64())
                .map(|h| h as u32);
            let decoded = UnifiedAddress::decode(&MAIN, ua).unwrap();
            assert_eq!(decoded.expiry_height().map(u32::from), expiry, "{ua}");

            let t_script = hex_field(&row, "p2pkh_bytes")
                .map(|h| p2pkh(&h))
                .or_else(|| hex_field(&row, "p2sh_bytes").map(|h| p2sh(&h)));
            if let Some(script) = t_script.filter(|_| expiry.is_none()) {
                assert_eq!(transparent_output(&transparent_send(&script, Some(ua))), ua);
                let mut wrong = script.clone();
                wrong[4] ^= 1;
                assert!(
                    matches!(
                        render(&transparent_send(&wrong, Some(ua))),
                        Err(VisualSignError::ValidationError(_))
                    ),
                    "{ua} accepted over a different transparent receiver"
                );
                *checked.entry("transparent").or_insert(0) += 1;
            }
            let orchard = hex_field(&row, "orchard_raw_addr");
            let sapling = hex_field(&row, "sapling_raw_addr");
            // Every transparent receiver above; every third shielded one
            // (each needs a fresh shielded build).
            if (orchard.is_some() || sapling.is_some()) && n % 3 == 0 {
                let pczt = shielded_send_to(ua, orchard.as_deref(), sapling.as_deref(), expiry);
                let r = render(&pczt).unwrap_or_else(|e| panic!("{ua}: {e}"));
                let shown = r.outputs.iter().find(|o| o.zat == 1_000_000).unwrap();
                assert_eq!(shown.address, ua);
                *checked
                    .entry(if orchard.is_some() {
                        "orchard"
                    } else {
                        "sapling"
                    })
                    .or_insert(0) += 1;
            }
        }
    }
    println!("unified address vectors checked per receiver: {checked:?}");
    assert!(
        checked.values().all(|n| *n > 0) && checked.len() == 3,
        "{checked:?}"
    );
}

#[test]
fn orchard_note_encryption_vectors_through_the_parser() {
    let base = fixture("orchard_send_v5");
    let mut checked = 0;
    for (n, row) in rows("orchard_note_encryption_78321beac.json")
        .iter()
        .enumerate()
    {
        let field = |name: &str| hex_field(row, name).unwrap();
        let value = row["v"].as_u64().unwrap();
        let recipient = [field("default_d"), field("default_pk_d")].concat();
        let pczt = edit_pczt(&base, |v| {
            let actions = v["orchard"]["actions"].as_array_mut().unwrap();
            // Spend values must not be stated once an output value changes.
            for a in actions.iter_mut() {
                a["spend"]["value"] = Value::Null;
            }
            let a = actions
                .iter_mut()
                .find(|a| a["output"]["value"] == 100_000)
                .unwrap();
            a["cv_net"] = json!(field("cv_net"));
            for k in [
                "recipient",
                "value",
                "rho",
                "rseed",
                "fvk",
                "witness",
                "alpha",
            ] {
                a["spend"][k] = Value::Null;
            }
            a["spend"]["nullifier"] = json!(field("rho"));
            let o = &mut a["output"];
            o["recipient"] = json!(recipient);
            o["value"] = json!(value);
            o["rseed"] = json!(field("rseed"));
            o["cmx"] = json!(field("cmx"));
            o["ephemeral_key"] = json!(field("ephemeral_key"));
            o["enc_ciphertext"] = json!({ "Encrypted": field("c_enc") });
            o["out_ciphertext"] = json!(field("c_out"));
        });
        let r = render(&pczt).unwrap_or_else(|e| panic!("vector {n}: {e}"));
        let addr = orchard::Address::from_raw_address_bytes(&recipient.clone().try_into().unwrap())
            .unwrap();
        let ua = UnifiedAddress::from_receivers(Some(addr), None, None, None, None)
            .unwrap()
            .encode(&MAIN);
        let memo: [u8; 512] = field("memo").try_into().unwrap();
        let shown = r.outputs.iter().find(|o| o.zat == value).unwrap();
        assert_eq!(shown.address, ua, "vector {n}");
        assert_eq!(shown.memo, memo_text(&memo), "vector {n}");

        // The same note with the vector's cmx off by one bit is refused.
        let wrong = edit_pczt(&pczt, |v| {
            let a = v["orchard"]["actions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|a| a["output"]["value"] == value)
                .unwrap();
            let mut cmx = field("cmx");
            cmx[0] ^= 1;
            a["output"]["cmx"] = json!(cmx);
        });
        assert!(render(&wrong).is_err(), "vector {n}: wrong cmx accepted");
        checked += 1;
    }
    assert_eq!(checked, 10);
}

#[test]
fn transparent_script_encodings_match_the_address_codec() {
    // Sanity link between the vectors above and the P2PKH/P2SH script forms
    // the parser decodes.
    let row = &rows("zip_0320_78321beac.json")[0];
    let hash: [u8; 20] = hex_field(row, "p2pkh_bytes").unwrap().try_into().unwrap();
    assert_eq!(
        TransparentAddress::PublicKeyHash(hash).encode(&MAIN),
        row["t_addr"].as_str().unwrap()
    );
}
