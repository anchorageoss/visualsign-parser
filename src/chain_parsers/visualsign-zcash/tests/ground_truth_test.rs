//! Ground truth from construction and from viewing keys.
//!
//! Each PCZT here is built with deterministic keys through the librustzcash
//! roles a coordinator runs (`support/builder.rs`), so what it pays is known.
//! The parser's keyless view (recipients, values, memos, fee, padding) must
//! match both the construction spec and an independent path: trial decryption
//! of the PCZT's on-chain output fields (`cmx`/`cmu`, ephemeral key,
//! ciphertexts) with the recipients' incoming viewing keys and the sender's
//! outgoing viewing keys.
//!
//! The tamper tests then check the property a signer depends on: the parser
//! shows a recipient, value or memo only if it reproduces the output's note
//! commitment (which the ZIP-244 sighash commits to) and decrypts from the
//! output's own ciphertext.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/builder.rs"]
mod builder;
mod common;

use builder::{ChangePool, Spec};
use common::{Decrypted, Rendered, decrypt_tx, edit_pczt, memo_text, render, script_address};
use orchard::keys::Scope;
use sapling_crypto::keys::PreparedIncomingViewingKey;
use serde_json::{Value, json};
use std::fs;
use std::time::Instant;
use visualsign_zcash::MAX_SHIELDED_OUTPUTS;
use zcash_keys::address::UnifiedAddress;
use zcash_keys::encoding::AddressCodec;
use zcash_protocol::consensus::Network;
use zcash_protocol::memo::MemoBytes;
use zcash_transparent::address::TransparentAddress;

const MAIN: Network = Network::MainNetwork;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}.input", env!("CARGO_MANIFEST_DIR"));
    hex::decode(fs::read_to_string(path).unwrap().trim()).unwrap()
}

fn foreign_sapling() -> sapling_crypto::zip32::DiversifiableFullViewingKey {
    sapling_crypto::zip32::ExtendedSpendingKey::master(&[9; 32])
        .unwrap()
        .to_diversifiable_full_viewing_key()
}

/// Every output the test keys can read, keyed by pool and output index.
fn decrypt(pczt: &[u8]) -> Vec<Option<Decrypted>> {
    let effects = pczt::Pczt::parse(pczt).unwrap().into_effects().unwrap();
    let sender = builder::sender_orchard();
    let orchard_ivks = [
        sender.to_ivk(Scope::External),
        sender.to_ivk(Scope::Internal),
        builder::foreign_orchard().to_ivk(Scope::External),
    ];
    let orchard_ovks = [
        sender.to_ovk(Scope::External),
        sender.to_ovk(Scope::Internal),
    ];
    let s = builder::sender_sapling();
    let sapling_ivks = [
        PreparedIncomingViewingKey::new(&s.to_ivk(Scope::External)),
        PreparedIncomingViewingKey::new(&s.to_ivk(Scope::Internal)),
        PreparedIncomingViewingKey::new(&foreign_sapling().to_ivk(Scope::External)),
    ];
    let sapling_ovks = [s.to_ovk(Scope::External), s.to_ovk(Scope::Internal)];
    decrypt_tx(
        &effects,
        &orchard_ivks,
        &orchard_ovks,
        &sapling_ivks,
        &sapling_ovks,
    )
}

/// How the parser shows a shielded receiver when the PCZT has no
/// `user_address`: an Orchard-only UA, or a Sapling address.
fn shown_address(d: &Decrypted) -> String {
    if d.pool == "Sapling" {
        sapling_crypto::PaymentAddress::from_bytes(&d.recipient)
            .unwrap()
            .encode(&MAIN)
    } else {
        let addr = orchard::Address::from_raw_address_bytes(&d.recipient).unwrap();
        UnifiedAddress::from_receivers(Some(addr), None, None, None, None)
            .unwrap()
            .encode(&MAIN)
    }
}

/// (pool or badge, address, zatoshis, memo) for comparison as a multiset.
type Row = (String, String, u64, Option<String>);

fn sorted(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort();
    rows
}

/// What `spec` pays, from the construction alone.
fn spec_rows(spec: &Spec) -> Vec<Row> {
    let memo = |m: &str| {
        let mut bytes = *MemoBytes::empty().as_array();
        if !m.is_empty() {
            bytes = *MemoBytes::from_bytes(m.as_bytes()).unwrap().as_array();
        }
        memo_text(&bytes)
    };
    let orchard_to = spec
        .orchard_to
        .unwrap_or_else(builder::foreign_orchard_address);
    let orchard_ua = |a: orchard::Address| {
        UnifiedAddress::from_receivers(Some(a), None, None, None, None)
            .unwrap()
            .encode(&MAIN)
    };
    let sapling_to = spec
        .sapling_to
        .unwrap_or_else(builder::foreign_sapling_address)
        .encode(&MAIN);
    let mut rows = Vec::new();
    for (v, m) in &spec.orchard_out {
        rows.push(("Orchard".into(), orchard_ua(orchard_to), *v, memo(m)));
    }
    for (v, m) in &spec.ironwood_out {
        rows.push(("Ironwood".into(), orchard_ua(orchard_to), *v, memo(m)));
    }
    for (v, m) in &spec.sapling_out {
        rows.push(("Sapling".into(), sapling_to.clone(), *v, memo(m)));
    }
    for v in &spec.transparent_out {
        rows.push((
            "transparent".into(),
            builder::foreign_taddr().encode(&MAIN),
            *v,
            None,
        ));
    }
    let change = builder::change_for(spec);
    let internal = builder::sender_orchard().address_at(0u32, Scope::Internal);
    match spec.change {
        Some(ChangePool::Orchard) => {
            rows.push(("Orchard".into(), orchard_ua(internal), change, None))
        }
        Some(ChangePool::Ironwood) => {
            rows.push(("Ironwood".into(), orchard_ua(internal), change, None))
        }
        Some(ChangePool::Sapling) => rows.push((
            "Sapling".into(),
            builder::sender_sapling().change_address().1.encode(&MAIN),
            change,
            None,
        )),
        Some(ChangePool::Transparent) => rows.push((
            "transparent".into(),
            TransparentAddress::from_pubkey(&builder::sender_tkey().public_key()).encode(&MAIN),
            change,
            None,
        )),
        None => {}
    }
    rows
}

fn parser_rows(r: &Rendered) -> Vec<Row> {
    r.outputs
        .iter()
        .map(|o| {
            let pool = o.badge.split(' ').next().unwrap().to_string();
            (pool, o.address.clone(), o.zat, o.memo.clone())
        })
        .collect()
}

/// The decrypted nonzero-or-memo outputs, as the parser would list them, plus
/// the transparent outputs read from the PCZT's effects.
fn decrypted_rows(pczt: &[u8]) -> (Vec<Row>, usize) {
    let mut rows = Vec::new();
    let mut padding = 0;
    for d in decrypt(pczt) {
        match d {
            Some(d) if d.value > 0 || memo_text(&d.memo).is_some() => {
                rows.push((
                    d.pool.to_string(),
                    shown_address(&d),
                    d.value,
                    memo_text(&d.memo),
                ));
            }
            // Zero-value outputs no key reads, or with an empty memo, are padding.
            _ => padding += 1,
        }
    }
    let effects = pczt::Pczt::parse(pczt).unwrap().into_effects().unwrap();
    if let Some(t) = effects.transparent_bundle() {
        for o in &t.vout {
            let addr = script_address(&o.script_pubkey().0.0).unwrap();
            rows.push((
                "transparent".into(),
                addr.encode(&MAIN),
                u64::from(o.value()),
                None,
            ));
        }
    }
    (rows, padding)
}

/// Fresh builds on top of the committed fixtures: spends from two pools at
/// once, every shielded pool as a destination, and memos of each kind.
fn extra_specs() -> Vec<(&'static str, Spec)> {
    vec![
        (
            "orchard+sapling in, every pool out",
            Spec {
                height: builder::PRE_NU6_3,
                orchard_in: Some(30_000_000),
                sapling_in: Some(20_000_000),
                orchard_out: vec![(1_000_000, "first"), (0, "zero-value with memo")],
                sapling_out: vec![(2_000_000, "")],
                transparent_out: vec![3_000_000],
                change: Some(ChangePool::Orchard),
                ..Spec::default()
            },
        ),
        (
            "transparent in, ironwood and orchard out",
            Spec {
                height: builder::POST_NU6_3,
                transparent_in: Some(9_000_000),
                ironwood_out: vec![(4_000_000, "to ironwood")],
                change: Some(ChangePool::Transparent),
                ..Spec::default()
            },
        ),
    ]
}

#[test]
fn parser_view_matches_construction_and_viewing_key_decryption() {
    let committed = builder::fixtures()
        .into_iter()
        .map(|(name, spec)| (name, spec.clone(), fixture(name)));
    let fresh = extra_specs()
        .into_iter()
        .map(|(name, spec)| (name, spec.clone(), builder::build(&spec)));
    let mut checked = 0;
    for (name, spec, bytes) in committed.chain(fresh) {
        let r = render(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (decrypted, padding) = decrypted_rows(&bytes);
        let expected = sorted(spec_rows(&spec));
        assert_eq!(
            sorted(decrypted),
            expected,
            "{name}: viewing keys vs construction"
        );
        assert_eq!(
            sorted(parser_rows(&r)),
            expected,
            "{name}: parser vs construction"
        );
        assert_eq!(r.padding, padding, "{name}: padding count");
        assert_eq!(r.fee, builder::fee_for(&spec), "{name}: fee");
        assert_eq!(r.fee_check, "equals the ZIP-317 conventional fee", "{name}");
        checked += 1;
    }
    assert_eq!(checked, 6);
}

/// A named edit of one output in the v2 model.
type Edit<'a> = (&'static str, Box<dyn Fn(&mut Value) + 'a>);

/// The JSON path of a shielded output in the v2 model.
fn output<'a>(v: &'a mut Value, pool: &str, i: usize) -> &'a mut Value {
    if pool == "sapling" {
        &mut v["sapling"]["outputs"][i]
    } else {
        &mut v[pool]["actions"][i]["output"]
    }
}

fn outputs_len(v: &Value, pool: &str) -> usize {
    let key = if pool == "sapling" {
        "outputs"
    } else {
        "actions"
    };
    v[pool][key].as_array().unwrap().len()
}

/// Index of the first output paying `value`.
fn index_of(v: &Value, pool: &str, value: u64) -> usize {
    let mut v = v.clone();
    (0..outputs_len(&v, pool))
        .find(|i| output(&mut v, pool, *i)["value"] == value)
        .unwrap()
}

#[test]
fn tampered_shielded_outputs_are_refused() {
    // (fixture, pool key, the value of the send to tamper with)
    let cases = [
        ("orchard_send_v5", "orchard", 100_000u64),
        ("ironwood_send_v6", "ironwood", 1_000_000),
        ("sapling_send_v5", "sapling", 1_000_000),
    ];
    for (name, pool, value) in cases {
        let bytes = fixture(name);
        let model = serde_json::to_value(
            pczt::v2::Pczt::try_from(pczt::Pczt::parse(&bytes).unwrap()).unwrap(),
        )
        .unwrap();
        let send = index_of(&model, pool, value);
        let change = (0..outputs_len(&model, pool))
            .find(|i| {
                let mut m = model.clone();
                let o = output(&mut m, pool, *i);
                *i != send && o["value"].as_u64().unwrap_or(0) > 0
            })
            .unwrap();
        let cm = if pool == "sapling" { "cmu" } else { "cmx" };
        let take = |field: &str| {
            let mut m = model.clone();
            output(&mut m, pool, change)[field].clone()
        };
        let edits: Vec<Edit> = vec![
            (
                "recipient swapped for the change address",
                Box::new(|o: &mut Value| o["recipient"] = take("recipient")),
            ),
            (
                "value + 1",
                Box::new(move |o: &mut Value| o["value"] = json!(value + 1)),
            ),
            (
                "value - 1",
                Box::new(move |o: &mut Value| o["value"] = json!(value - 1)),
            ),
            (
                "rseed swapped",
                Box::new(|o: &mut Value| o["rseed"] = take("rseed")),
            ),
            (
                "recipient, value and rseed all copied from the change",
                Box::new(|o: &mut Value| {
                    o["recipient"] = take("recipient");
                    o["value"] = take("value");
                    o["rseed"] = take("rseed");
                }),
            ),
            (
                "commitment swapped",
                Box::new(|o: &mut Value| o[cm] = take(cm)),
            ),
            (
                "ciphertext swapped",
                Box::new(|o: &mut Value| {
                    o["enc_ciphertext"] = take("enc_ciphertext");
                    o["ephemeral_key"] = take("ephemeral_key");
                }),
            ),
        ];
        for (what, edit) in edits {
            let tampered = edit_pczt(&bytes, |v| edit(output(v, pool, send)));
            let err = render(&tampered).expect_err(&format!("{name}: {what} was rendered"));
            let msg = err.to_string();
            assert!(
                msg.contains("note commitment") || msg.contains("recover") || msg.contains("PCZT"),
                "{name}: {what}: unexpected error {msg}"
            );
        }
    }
}

#[test]
fn redacted_output_fields_are_refused() {
    for (name, pool) in [
        ("orchard_send_v5", "orchard"),
        ("ironwood_send_v6", "ironwood"),
        ("sapling_send_v5", "sapling"),
    ] {
        for field in ["recipient", "value", "rseed"] {
            let tampered = edit_pczt(&fixture(name), |v| output(v, pool, 0)[field] = Value::Null);
            assert!(render(&tampered).is_err(), "{name}: {field} redacted");
        }
    }
}

#[test]
fn padded_bundle_is_counted_and_priced() {
    let spec = Spec {
        height: builder::PRE_NU6_3,
        orchard_in: Some(10_000_000),
        orchard_out: vec![(1_000_000, "padded")],
        change: Some(ChangePool::Orchard),
        orchard_pad_to: Some(50),
        ..Spec::default()
    };
    let bytes = builder::build(&spec);
    let r = render(&bytes).unwrap();
    assert_eq!(r.padding, 48);
    assert_eq!(r.fee, builder::fee_for(&spec));
    assert_eq!(r.fee, 50 * 5_000);
    assert_eq!(r.fee_check, "equals the ZIP-317 conventional fee");
}

/// `transparent_mix_v5` with its zero-value padding action copied until the
/// Orchard bundle has `actions` actions. The fee still pays for two.
fn padded_to(actions: usize) -> Vec<u8> {
    edit_pczt(&fixture("transparent_mix_v5"), |v| {
        let list = v["orchard"]["actions"].as_array_mut().unwrap();
        let pad = list
            .iter()
            .find(|a| a["output"]["value"] == 0)
            .unwrap()
            .clone();
        let extra = actions - list.len();
        list.extend(std::iter::repeat_n(pad, extra));
    })
}

#[test]
fn oversized_action_count_is_flagged_as_underpaid() {
    // Up to the parser's output budget the PCZT renders, flagged as paying
    // less than the ZIP-317 fee for its action count.
    let r = render(&padded_to(MAX_SHIELDED_OUTPUTS)).unwrap();
    assert!(
        r.fee_check
            .starts_with("below the ZIP-317 conventional fee"),
        "{}",
        r.fee_check
    );
    assert_eq!(r.padding, MAX_SHIELDED_OUTPUTS - 1);
}

#[test]
fn pczt_too_large_for_any_valid_transaction_is_refused() {
    // Regression for the finding pinned against #578 at 89281ac8: 1 000
    // padding actions (over 3 MB once proven, so no block can hold the
    // transaction) were parsed in full, about 6 ms of note checks per action.
    // 0308bce7 refuses more than MAX_SHIELDED_OUTPUTS outputs/actions right
    // after deserialization.
    for actions in [MAX_SHIELDED_OUTPUTS + 1, 1_000] {
        let started = Instant::now();
        let err = render(&padded_to(actions)).unwrap_err();
        assert!(
            err.to_string().contains("the limit is"),
            "{actions} actions: {err}"
        );
        println!("{actions} actions refused in {:?}", started.elapsed());
    }
}
