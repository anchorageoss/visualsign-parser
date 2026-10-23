//! Golden and behavior tests over committed PCZT fixtures.
//!
//! `REGEN_FIXTURES=1 cargo test -p visualsign-zcash --test fixtures golden`
//! rewrites every `.expected` from the current parser output. Review the diff.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/builder.rs"]
mod builder;

use base64::Engine as _;
use std::fs;
use std::path::PathBuf;
use visualsign::SignablePayload;
use visualsign::vsptrait::{
    TransactionParseError, VisualSignConverterFromString, VisualSignError, VisualSignOptions,
};
use visualsign_zcash::{
    MAX_PCZT_BYTES, UNVERIFIED, ZcashTransactionWrapper, ZcashVisualSignConverter,
};
use zcash_keys::address::UnifiedAddress;
use zcash_keys::encoding::AddressCodec;

const FIXTURES: [&str; 5] = [
    "orchard_send_v5",
    "ironwood_send_v6",
    "sapling_send_v5",
    "transparent_mix_v5",
    "keystone_v1_orchard_to_transparent",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn input(name: &str) -> String {
    fs::read_to_string(fixtures_dir().join(format!("{name}.input")))
        .unwrap()
        .trim()
        .to_string()
}

fn bytes(name: &str) -> Vec<u8> {
    hex::decode(input(name)).unwrap()
}

fn parse(data: &str) -> Result<SignablePayload, VisualSignError> {
    ZcashVisualSignConverter
        .to_visual_sign_payload_from_string(data, VisualSignOptions::default())
        .map(|r| r.payload)
}

fn parse_bytes(b: &[u8]) -> Result<SignablePayload, VisualSignError> {
    parse(&hex::encode(b))
}

fn text<'a>(p: &'a SignablePayload, label: &str) -> &'a str {
    p.fields
        .iter()
        .find(|f| f.label() == label)
        .unwrap_or_else(|| panic!("no field {label}: {:#?}", p.fields))
        .fallback_text()
}

fn has(p: &SignablePayload, label: &str) -> bool {
    p.fields.iter().any(|f| f.label() == label)
}

/// Fallback texts of every "Output n" field, in order.
fn outputs(p: &SignablePayload) -> Vec<String> {
    (1..)
        .map_while(|n| {
            has(p, &format!("Output {n}")).then(|| {
                format!(
                    "{} {}",
                    text(p, &format!("Output {n} amount")),
                    text(p, &format!("Output {n}"))
                )
            })
        })
        .collect()
}

fn ua(addr: orchard::Address) -> String {
    UnifiedAddress::from_receivers(Some(addr), None, None, None, None)
        .unwrap()
        .encode(&builder::MAIN)
}

fn zec(z: u64) -> String {
    format!("{}.{:08}", z / 100_000_000, z % 100_000_000)
}

fn sighash(b: &[u8]) -> String {
    let s = pczt::roles::signer::Signer::new(pczt::Pczt::parse(b).unwrap())
        .unwrap()
        .shielded_sighash();
    hex::encode(s)
}

/// Replace the unique occurrence of `from` in `bytes` with `to` (same length).
fn replace_once(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let hits: Vec<usize> = (0..=bytes.len() - from.len())
        .filter(|&i| &bytes[i..i + from.len()] == from)
        .collect();
    assert_eq!(hits.len(), 1, "pattern must occur exactly once");
    let mut out = bytes.to_vec();
    out[hits[0]..hits[0] + to.len()].copy_from_slice(to);
    out
}

/// Postcard (LEB128) varint encoding of a u64.
fn varint(mut v: u64) -> Vec<u8> {
    let mut out = vec![];
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

#[test]
fn golden() {
    let regen = std::env::var("REGEN_FIXTURES").is_ok();
    for name in FIXTURES {
        let actual = parse(&input(name))
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .to_json()
            .unwrap();
        let path = fixtures_dir().join(format!("{name}.expected"));
        if regen {
            fs::write(&path, format!("{actual}\n")).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing {path:?}; run with REGEN_FIXTURES=1"));
        assert_eq!(actual, expected.trim_end(), "{name}");
    }
}

#[test]
// Integration mapping check only; independent raw-transaction known-answer
// coverage is in zip244_vectors.rs and ironwood_v6_vectors.rs.
fn sighash_field_maps_to_the_pczt_signer_digest() {
    for name in FIXTURES {
        let p = parse(&input(name)).unwrap();
        assert_eq!(
            text(&p, "Shielded sighash (ZIP-244)"),
            sighash(&bytes(name)),
            "{name}"
        );
    }
}

#[test]
fn orchard_send_shows_recipients_memos_fee_and_unverified_change() {
    let spec = &builder::fixtures()[0].1;
    let p = parse(&input("orchard_send_v5")).unwrap();
    assert_eq!(p.title, "Zcash Transaction");
    assert_eq!(p.payload_type, "ZcashTx");
    assert_eq!(text(&p, "Declared network (unsigned)"), "Zcash mainnet");

    // Without a viewing key every output is listed, change included; the
    // builder shuffles actions, so compare as a set.
    let to = ua(builder::foreign_orchard_address());
    let change = ua(builder::sender_orchard().address_at(0u32, orchard::keys::Scope::Internal));
    let mut got = outputs(&p);
    got.sort();
    let mut want = vec![
        format!("{} ZEC {to}", zec(100_000)),
        format!("{} ZEC {to}", zec(200_000)),
        format!("{} ZEC {change}", zec(builder::change_for(spec))),
    ];
    want.sort();
    assert_eq!(got, want);

    let memos: Vec<&str> = (1..=3)
        .filter(|n| has(&p, &format!("Output {n} memo")))
        .map(|n| text(&p, &format!("Output {n} memo")))
        .collect();
    assert!(memos.contains(&"thanks"), "{memos:?}");
    // Non-ASCII text is shown as its UTF-8 bytes ("caf\u{e9}").
    assert!(
        memos.contains(&"text memo, UTF-8 0x636166c3a9"),
        "{memos:?}"
    );
    assert_eq!(memos.len(), 2, "empty change memo is not shown");

    let fee = builder::fee_for(spec);
    assert_eq!(fee, 15_000);
    assert_eq!(text(&p, "Fee"), format!("{} ZEC", zec(fee)));
    assert_eq!(text(&p, "Fee check"), "equals the ZIP-317 conventional fee");
    assert!(text(&p, "Change").starts_with(UNVERIFIED));
    assert_eq!(
        text(&p, "Orchard spent"),
        format!("{} ZEC; spend ownership {UNVERIFIED}", zec(100_000_000))
    );
    let nu6_2 = u32::from(zcash_protocol::consensus::BranchId::Nu6_2);
    assert_eq!(
        text(&p, "Consensus branch"),
        format!("Nu6_2 (0x{nu6_2:08x})")
    );
}

#[test]
fn ironwood_send_is_handled_like_orchard() {
    let spec = &builder::fixtures()[1].1;
    let b = bytes("ironwood_send_v6");
    assert_eq!(*pczt::Pczt::parse(&b).unwrap().global().tx_version(), 6);
    let p = parse_bytes(&b).unwrap();
    let to = ua(builder::foreign_orchard_address());
    assert!(
        outputs(&p).contains(&format!("{} ZEC {to}", zec(1_000_000))),
        "{:?}",
        outputs(&p)
    );
    let n = (1..)
        .find(|n| text(&p, &format!("Output {n}")) == to)
        .unwrap();
    assert_eq!(text(&p, &format!("Output {n} memo")), "iw");
    let badge = match &p.fields.iter().find(|f| f.label() == "Output 1").unwrap() {
        visualsign::SignablePayloadField::AddressV2 { address_v2, .. } => {
            address_v2.badge_text.clone().unwrap()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(badge, "Ironwood");
    assert_eq!(
        text(&p, "Orchard spent"),
        format!("{} ZEC; spend ownership {UNVERIFIED}", zec(10_000_000))
    );
    assert_eq!(
        text(&p, "Fee"),
        format!("{} ZEC", zec(builder::fee_for(spec)))
    );
    assert_eq!(text(&p, "Fee check"), "equals the ZIP-317 conventional fee");
    // ZIP 326 same-address zero-value outputs are padding, not recipients.
    assert!(has(&p, "Padding outputs"));
}

#[test]
fn sapling_send() {
    let p = parse(&input("sapling_send_v5")).unwrap();
    let to = builder::foreign_sapling_address().encode(&builder::MAIN);
    let n = (1..)
        .find(|n| text(&p, &format!("Output {n}")) == to)
        .unwrap();
    assert_eq!(text(&p, &format!("Output {n} amount")), "0.01000000 ZEC");
    assert_eq!(text(&p, &format!("Output {n} memo")), "rent");
    assert_eq!(
        text(&p, "Sapling spent"),
        format!("{} ZEC; spend ownership {UNVERIFIED}", zec(50_000_000))
    );
    assert_eq!(text(&p, "Fee check"), "equals the ZIP-317 conventional fee");
    assert!(!has(&p, "Sapling proof key"));
}

#[test]
fn transparent_inputs_outputs_and_claimed_change() {
    let spec = &builder::fixtures()[3].1;
    let p = parse(&input("transparent_mix_v5")).unwrap();
    let ours = zcash_transparent::address::TransparentAddress::from_pubkey(
        &builder::sender_tkey().public_key(),
    );
    assert_eq!(
        text(&p, "Transparent input 1"),
        format!(
            "{} (P2PKH) {} ZEC",
            ours.encode(&builder::MAIN),
            zec(2_000_000)
        )
    );
    let all = outputs(&p);
    assert!(all.contains(&format!(
        "{} ZEC {}",
        zec(300_000),
        builder::foreign_taddr().encode(&builder::MAIN)
    )));
    let change = format!(
        "{} ZEC {}",
        zec(builder::change_for(spec)),
        ours.encode(&builder::MAIN)
    );
    let n = all.iter().position(|o| *o == change).unwrap() + 1;
    let badge = match p
        .fields
        .iter()
        .find(|f| *f.label() == format!("Output {n}"))
        .unwrap()
    {
        visualsign::SignablePayloadField::AddressV2 { address_v2, .. } => {
            address_v2.badge_text.clone().unwrap()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(
        badge,
        format!("transparent P2PKH, claimed change {UNVERIFIED}")
    );
    assert_eq!(
        text(&p, "Fee"),
        format!("{} ZEC", zec(builder::fee_for(spec)))
    );
    assert_eq!(text(&p, "Fee check"), "equals the ZIP-317 conventional fee");
}

#[test]
fn keystone_firmware_vector_parses() {
    let b = bytes("keystone_v1_orchard_to_transparent");
    assert_eq!(&b[4..8], &1u32.to_le_bytes());
    let p = parse_bytes(&b).unwrap();
    assert_eq!(text(&p, "Declared network (unsigned)"), "Zcash mainnet");
    assert!(has(&p, "Transparent input 1") || has(&p, "Orchard spent"));
    assert!(has(&p, "Output 1"));
}

#[test]
fn tampered_output_amount_is_refused() {
    let b = bytes("orchard_send_v5");
    let t = replace_once(&b, &varint(100_000), &varint(100_001));
    let e = parse_bytes(&t).unwrap_err();
    assert!(
        format!("{e}").contains("note commitment") || matches!(e, VisualSignError::ParseError(_)),
        "{e}"
    );
}

#[test]
fn tampered_spend_amount_is_refused() {
    let b = bytes("orchard_send_v5");
    let t = replace_once(&b, &varint(100_000_000), &varint(100_000_001));
    assert!(parse_bytes(&t).is_err());
}

#[test]
fn lowered_transparent_input_amount_is_refused() {
    let b = bytes("transparent_mix_v5");
    let t = replace_once(&b, &varint(2_000_000), &varint(1_000_000));
    assert!(parse_bytes(&t).is_err());
}

#[test]
fn user_address_must_contain_the_receiver() {
    let pczt = pczt::Pczt::parse(&bytes("orchard_send_v5")).unwrap();
    let to = builder::foreign_orchard_address();
    let send = pczt
        .orchard()
        .actions()
        .iter()
        .position(|a| *a.output().recipient() == Some(to.to_raw_address_bytes()))
        .unwrap();
    let set = |addr: String| {
        pczt::roles::updater::Updater::new(pczt.clone())
            .update_orchard_with(|mut u| {
                u.update_action_with(send, |mut a| {
                    a.set_output_user_address(addr);
                    Ok(())
                })
            })
            .unwrap()
            .finish()
            .serialize()
            .unwrap()
    };

    let good = ua(to);
    assert!(
        outputs(&parse_bytes(&set(good.clone())).unwrap())
            .iter()
            .any(|o| o.ends_with(&good))
    );

    let other = ua(builder::sender_orchard().address_at(5u32, orchard::keys::Scope::External));
    assert!(matches!(
        parse_bytes(&set(other)),
        Err(VisualSignError::ValidationError(_))
    ));

    let testnet = UnifiedAddress::from_receivers(Some(to), None, None, None, None)
        .unwrap()
        .encode(&zcash_protocol::consensus::Network::TestNetwork);
    assert!(matches!(
        parse_bytes(&set(testnet)),
        Err(VisualSignError::ValidationError(_))
    ));
}

#[test]
fn v2_encoding_is_accepted() {
    let pczt = pczt::Pczt::parse(&bytes("orchard_send_v5")).unwrap();
    let v2 = pczt::v2::Pczt::try_from(pczt).unwrap().serialize();
    assert_eq!(&v2[4..8], &2u32.to_le_bytes());
    let p = parse_bytes(&v2).unwrap();
    assert_eq!(
        text(&p, "Shielded sighash (ZIP-244)"),
        sighash(&bytes("orchard_send_v5"))
    );
    let mut trailing = v2.clone();
    trailing.push(0);
    assert!(parse_bytes(&trailing).is_err());
}

fn parse_error(r: Result<SignablePayload, VisualSignError>) -> TransactionParseError {
    match r {
        Err(VisualSignError::ParseError(e)) => e,
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn malformed_input_is_refused() {
    let b = bytes("orchard_send_v5");
    parse_error(parse(""));
    parse_error(parse("zz not hex or base64 !!"));
    parse_error(parse_bytes(b"PCZT\x01\x00\x00\x00garbage"));
    parse_error(parse_bytes(&b[..b.len() / 2]));
    parse_error(parse_bytes(&b[..6]));
    let mut magic = b.clone();
    magic[0] = b'X';
    parse_error(parse_bytes(&magic));
    let mut version = b.clone();
    version[4] = 9;
    parse_error(parse_bytes(&version));
    let mut trailing = b.clone();
    trailing.push(0);
    parse_error(parse_bytes(&trailing));
    let big = vec![0u8; MAX_PCZT_BYTES + 1];
    assert!(matches!(
        parse_error(parse_bytes(&big)),
        TransactionParseError::InvalidFormat(_)
    ));
}

#[test]
fn base64_and_0x_hex_inputs_are_accepted() {
    let b = bytes("sapling_send_v5");
    let want = sighash(&b);
    for s in [
        base64::engine::general_purpose::STANDARD.encode(&b),
        format!("0x{}", hex::encode(&b)),
    ] {
        assert_eq!(
            text(&parse(&s).unwrap(), "Shielded sighash (ZIP-244)"),
            want
        );
    }
}

// Mutate the public v2 serialization model, leaving note commitments and
// transaction structure intact. Derivation maps are irrelevant to these cases
// and their byte-array keys cannot be represented as JSON object keys.
fn edit_pczt(b: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let pczt = pczt::roles::redactor::Redactor::new(pczt::Pczt::parse(b).unwrap())
        .redact_transparent_with(|mut r| {
            r.redact_inputs(|mut i| i.clear_bip32_derivation());
            r.redact_outputs(|mut o| o.clear_bip32_derivation());
        })
        .finish();
    let mut value = serde_json::to_value(pczt::v2::Pczt::try_from(pczt).unwrap()).unwrap();
    edit(&mut value);
    serde_json::from_value::<pczt::v2::Pczt>(value)
        .unwrap()
        .serialize()
}

#[test]
fn review_rejects_non_all_transparent_sighashes() {
    for sighash_type in [2u8, 3, 0x81, 0x82, 0x83] {
        let b = edit_pczt(&bytes("transparent_mix_v5"), |v| {
            v["transparent"]["inputs"][0]["sighash_type"] = sighash_type.into();
        });
        let err = parse_bytes(&b).unwrap_err();
        assert!(err.to_string().contains("sighash"), "{err}");
    }
}

#[test]
fn review_transparent_input_digest_commits_to_its_amount() {
    let b = bytes("transparent_mix_v5");
    let changed = edit_pczt(&b, |v| {
        v["transparent"]["inputs"][0]["value"] = 2_000_001u64.into();
    });
    let original = parse_bytes(&b).unwrap();
    let changed = parse_bytes(&changed).unwrap();
    // ZIP 244 also commits the shielded digest to all transparent amounts.
    assert_ne!(
        text(&original, "Shielded sighash (ZIP-244)"),
        text(&changed, "Shielded sighash (ZIP-244)")
    );
    assert_ne!(
        text(&original, "Transparent input 1 sighash (ZIP-244)"),
        text(&changed, "Transparent input 1 sighash (ZIP-244)")
    );
}

#[test]
fn review_declared_network_is_explicitly_unsigned() {
    let b = bytes("orchard_send_v5");
    let changed = edit_pczt(&b, |v| {
        v["global"]["coin_type"] = 1.into();
    });
    let original = parse_bytes(&b).unwrap();
    let changed = parse_bytes(&changed).unwrap();
    assert_eq!(
        text(&original, "Shielded sighash (ZIP-244)"),
        text(&changed, "Shielded sighash (ZIP-244)")
    );
    assert_eq!(
        text(&original, "Declared network (unsigned)"),
        "Zcash mainnet"
    );
    assert_eq!(
        text(&changed, "Declared network (unsigned)"),
        "Zcash testnet"
    );
    assert!(text(&changed, "Network trust").contains("address prefixes"));
}

#[test]
fn review_nonzero_shielded_outputs_must_recover() {
    for (fixture, pool) in [
        ("orchard_send_v5", "orchard"),
        ("ironwood_send_v6", "ironwood"),
        ("sapling_send_v5", "sapling"),
    ] {
        let b = edit_pczt(&bytes(fixture), |v| {
            let output = if pool == "sapling" {
                v[pool]["outputs"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|o| o["value"].as_u64().unwrap_or(0) > 0)
                    .unwrap()
            } else {
                &mut v[pool]["actions"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|a| a["output"]["value"].as_u64().unwrap_or(0) > 0)
                    .unwrap()["output"]
            };
            output["enc_ciphertext"] = if pool == "sapling" {
                serde_json::json!(vec![0u8; 580])
            } else {
                serde_json::json!({"Encrypted": vec![0u8; 580]})
            };
        });
        let err = parse_bytes(&b).unwrap_err();
        assert!(err.to_string().contains("recover"), "{fixture}: {err}");
    }
}

#[test]
fn review_ua_expiry_height_is_enforced_and_time_is_visible() {
    let b = bytes("orchard_send_v5");
    let expiry = *pczt::Pczt::parse(&b).unwrap().global().expiry_height();
    for (tx_expiry, ua_expiry, accepted) in [
        (expiry, expiry - 1, false),
        (0, expiry, false),
        (expiry, expiry, true),
        (expiry, expiry + 1, true),
    ] {
        let addr = UnifiedAddress::from_receivers(
            Some(builder::foreign_orchard_address()),
            None,
            None,
            Some(zcash_protocol::consensus::BlockHeight::from_u32(ua_expiry)),
            Some(1_900_000_000),
        )
        .unwrap()
        .encode(&builder::MAIN);
        let edited = edit_pczt(&b, |v| {
            v["global"]["expiry_height"] = tx_expiry.into();
            for a in v["orchard"]["actions"].as_array_mut().unwrap() {
                if a["output"]["value"] == 100_000u64 {
                    a["output"]["user_address"] = addr.clone().into();
                }
            }
        });
        let result = parse_bytes(&edited);
        if accepted {
            let p = result.unwrap();
            let n = (1..=3)
                .find(|n| text(&p, &format!("Output {n}")) == addr)
                .unwrap();
            assert_eq!(
                text(&p, &format!("Output {n} address expiry time")),
                "1900000000 Unix seconds; not checked (parser has no clock)"
            );
        } else {
            assert!(result.unwrap_err().to_string().contains("expiry"));
        }
    }
}

#[test]
fn review_tex_requires_transparent_only_sources() {
    let shielded = builder::build(&builder::Spec {
        height: builder::PRE_NU6_3,
        orchard_in: Some(1_000_000),
        transparent_out: vec![100_000],
        change: Some(builder::ChangePool::Orchard),
        ..Default::default()
    });
    let transparent = bytes("transparent_mix_v5");
    let tex = zcash_keys::address::Address::Tex([0x42; 20]).encode(&builder::MAIN);
    for (b, accepted) in [(shielded, false), (transparent, true)] {
        let edited = edit_pczt(&b, |v| {
            v["transparent"]["outputs"][0]["user_address"] = tex.clone().into();
        });
        let result = parse_bytes(&edited);
        if accepted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(result.unwrap_err().to_string().contains("TEX"));
        }
    }
}

#[test]
fn review_transparent_only_transaction_has_a_distinct_digest_for_each_input() {
    let b = builder::build(&builder::Spec {
        height: builder::PRE_NU6_3,
        transparent_in: Some(2_000_000),
        transparent_out: vec![100_000],
        change: Some(builder::ChangePool::Transparent),
        ..Default::default()
    });
    let b = edit_pczt(&b, |v| {
        let inputs = v["transparent"]["inputs"].as_array_mut().unwrap();
        let mut second = inputs[0].clone();
        second["prevout_index"] = 1.into();
        inputs.push(second);
    });
    let p = parse_bytes(&b).unwrap();
    let signer = pczt::roles::signer::Signer::new(pczt::Pczt::parse(&b).unwrap()).unwrap();
    for i in 0..2 {
        assert_eq!(
            text(
                &p,
                &format!("Transparent input {} sighash (ZIP-244)", i + 1)
            ),
            hex::encode(signer.transparent_sighash(i).unwrap())
        );
    }
    assert_ne!(
        text(&p, "Transparent input 1 sighash (ZIP-244)"),
        text(&p, "Transparent input 2 sighash (ZIP-244)")
    );
    assert_ne!(
        text(&p, "Transparent input 1 sighash (ZIP-244)"),
        text(&p, "Shielded sighash (ZIP-244)")
    );
}

// Repeat valid serialization records to exercise the parser's work budget.
// These need not be consensus-valid transactions: admission must happen before
// note checking, sighash extraction, or rendering can encounter any bad record.
fn repeat_shielded_records(b: &[u8], pool: &str, field: &str, count: usize) -> Vec<u8> {
    edit_pczt(b, |v| {
        let items = v[pool][field].as_array_mut().unwrap();
        let original = items.clone();
        *items = (0..count)
            .map(|i| original[i % original.len()].clone())
            .collect();
        for (other_pool, other_field) in [
            ("orchard", "actions"),
            ("ironwood", "actions"),
            ("sapling", "outputs"),
        ] {
            if other_pool != pool && v[other_pool].is_object() {
                v[other_pool][other_field] = serde_json::json!([]);
            }
        }
    })
}

#[test]
fn shielded_work_limit_bounds_each_pool_before_conversion() {
    for (fixture, pool, field) in [
        ("orchard_send_v5", "orchard", "actions"),
        // This fixture mixes a payment with zero-value padding, so ignoring
        // padding in the work budget would wrongly admit its repeated records.
        ("keystone_v1_orchard_to_transparent", "orchard", "actions"),
        ("ironwood_send_v6", "ironwood", "actions"),
        ("sapling_send_v5", "sapling", "outputs"),
    ] {
        for count in [64, 65] {
            let v2 = repeat_shielded_records(&bytes(fixture), pool, field, count);
            assert!(v2.len() < MAX_PCZT_BYTES);
            let pczt = pczt::Pczt::parse(&v2).unwrap();
            let mut encodings = vec![v2];
            // Ironwood cannot be encoded as v1.
            if let Ok(v1) = pczt::v1::Pczt::try_from(pczt) {
                encodings.push(v1.serialize());
            }
            for b in encodings {
                let result = ZcashTransactionWrapper::from_bytes(&b).map(|_| ());
                if count == 64 {
                    assert!(result.is_ok(), "{pool}: {result:?}");
                } else {
                    assert!(
                        matches!(result, Err(TransactionParseError::InvalidFormat(ref message))
                        if message.contains("shielded") && message.contains("64")),
                        "{pool}: {result:?}"
                    );
                    // Conversion must fail at admission, before invalid duplicated
                    // records could produce a later commitment or balance error.
                    assert!(matches!(
                        parse_error(parse_bytes(&b)),
                        TransactionParseError::InvalidFormat(_)
                    ));
                }
            }
        }
    }
}

#[test]
fn shielded_work_limit_is_shared_across_pools() {
    let orchard = repeat_shielded_records(&bytes("orchard_send_v5"), "orchard", "actions", 22);
    let sapling = repeat_shielded_records(&bytes("sapling_send_v5"), "sapling", "outputs", 21);
    let ironwood = repeat_shielded_records(&bytes("ironwood_send_v6"), "ironwood", "actions", 21);
    let mut source = serde_json::Value::Null;
    edit_pczt(&ironwood, |v| source = v.clone());
    let mut sapling_bundle = serde_json::Value::Null;
    edit_pczt(&sapling, |v| sapling_bundle = v["sapling"].clone());
    let boundary = edit_pczt(&orchard, |v| {
        v["ironwood"] = source["ironwood"].clone();
        v["sapling"] = sapling_bundle;
    });
    assert!(ZcashTransactionWrapper::from_bytes(&boundary).is_ok());
    let over = edit_pczt(&boundary, |v| {
        let outputs = v["sapling"]["outputs"].as_array_mut().unwrap();
        outputs.push(outputs[0].clone());
    });
    assert!(over.len() < MAX_PCZT_BYTES);
    assert!(matches!(
        ZcashTransactionWrapper::from_bytes(&over),
        Err(TransactionParseError::InvalidFormat(message)) if message.contains("shielded")
    ));
}

#[test]
fn shielded_work_limit_preserves_boundary_payment_rendering() {
    let pczt = builder::build(&builder::Spec {
        height: builder::PRE_NU6_3,
        transparent_in: Some(100_000_000),
        orchard_out: vec![(1_000, "batch payment"); 64],
        change: Some(builder::ChangePool::Transparent),
        ..Default::default()
    });
    let payload = parse_bytes(&pczt).unwrap();
    assert_eq!(outputs(&payload).len(), 65); // 64 payments plus transparent change.
    assert_eq!(text(&payload, "Output 64 amount"), "0.00001000 ZEC");
}
