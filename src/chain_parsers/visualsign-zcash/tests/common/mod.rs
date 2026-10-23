//! Shared helpers for the validation tests: read the parser's rendered payload
//! back into plain values, trial-decrypt shielded outputs with viewing keys,
//! and rebuild a PCZT from a finished transaction, so the parser's keyless view
//! can be compared with sources that do not go through it.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sapling_crypto::note_encryption::{
    SaplingDomain, Zip212Enforcement, try_sapling_note_decryption,
};
use serde_json::{Value, json};
use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignError, VisualSignOptions};
use visualsign_zcash::ZcashVisualSignConverter;
use zcash_note_encryption::try_output_recovery_with_ovk;
use zcash_primitives::transaction::{Authorization, Transaction, TransactionData};
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::ZatBalance;
use zcash_transparent::address::TransparentAddress;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOutput {
    pub address: String,
    pub badge: String,
    pub zat: u64,
    pub memo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub network: String,
    pub outputs: Vec<RenderedOutput>,
    /// (pool, zatoshis) from the "<pool> spent" fields.
    pub spent: Vec<(String, u64)>,
    /// (address text, zatoshis) per transparent input.
    pub transparent_inputs: Vec<(String, u64)>,
    pub transparent_sighashes: Vec<String>,
    pub fee: u64,
    pub fee_check: String,
    pub padding: usize,
    pub expiry: u32,
    pub branch: String,
    pub sighash: String,
}

/// "1.23456789 ZEC" (or a leading "1.23456789") to zatoshis.
pub fn zec_to_zat(text: &str) -> u64 {
    let amount = text.split_whitespace().next().unwrap();
    let (whole, frac) = amount.split_once('.').unwrap();
    assert_eq!(frac.len(), 8, "amount {text:?} is not 8-decimal ZEC");
    whole.parse::<u64>().unwrap() * 100_000_000 + frac.parse::<u64>().unwrap()
}

/// Run the parser exactly as the CLI and servers do, then read the payload back.
pub fn render(pczt: &[u8]) -> Result<Rendered, VisualSignError> {
    let payload = ZcashVisualSignConverter
        .to_visual_sign_payload_from_string(&hex::encode(pczt), VisualSignOptions::default())?
        .payload;
    let json: Value = serde_json::from_str(&payload.to_json().unwrap()).unwrap();
    let fields = json["Fields"].as_array().unwrap();
    let text = |label: &str| -> Option<String> {
        fields
            .iter()
            .find(|f| f["Label"] == label)
            .map(|f| f["FallbackText"].as_str().unwrap().to_string())
    };
    let mut outputs = Vec::new();
    for n in 1.. {
        let label = format!("Output {n}");
        let Some(field) = fields.iter().find(|f| f["Label"] == label.as_str()) else {
            break;
        };
        outputs.push(RenderedOutput {
            address: field["AddressV2"]["Address"].as_str().unwrap().to_string(),
            badge: field["AddressV2"]["BadgeText"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            zat: zec_to_zat(&text(&format!("{label} amount")).unwrap()),
            memo: text(&format!("{label} memo")),
        });
    }
    let spent = ["Orchard", "Ironwood", "Sapling"]
        .iter()
        .filter_map(|pool| {
            text(&format!("{pool} spent")).map(|t| (pool.to_string(), zec_to_zat(&t)))
        })
        .collect();
    let mut transparent_inputs = Vec::new();
    let mut transparent_sighashes = Vec::new();
    for n in 1.. {
        let Some(t) = text(&format!("Transparent input {n}")) else {
            break;
        };
        let mut words = t.rsplitn(3, ' ');
        assert_eq!(words.next(), Some("ZEC"), "{t}");
        let zat = zec_to_zat(words.next().unwrap());
        transparent_inputs.push((words.next().unwrap().to_string(), zat));
        transparent_sighashes
            .push(text(&format!("Transparent input {n} sighash (ZIP-244)")).unwrap());
    }
    Ok(Rendered {
        network: text("Declared network (unsigned)").unwrap(),
        outputs,
        spent,
        transparent_inputs,
        transparent_sighashes,
        fee: zec_to_zat(&text("Fee").unwrap()),
        fee_check: text("Fee check").unwrap(),
        padding: text("Padding outputs").map_or(0, |t| t.parse().unwrap()),
        expiry: text("Expiry height").unwrap().parse().unwrap(),
        branch: text("Consensus branch").unwrap(),
        sighash: text("Shielded sighash (ZIP-244)").unwrap(),
    })
}

/// The memo bytes in the parser's display convention (ZIP 302 text if it is
/// printable ASCII, otherwise hex), for comparing decrypted memos with the
/// parser's memo field. `None` for an empty memo.
pub fn memo_text(memo: &[u8; 512]) -> Option<String> {
    match Memo::try_from(MemoBytes::from_bytes(memo).unwrap()).unwrap() {
        Memo::Empty => None,
        Memo::Text(t) => {
            let t = String::from(t);
            if t.is_empty() {
                None
            } else if t.bytes().all(|b| b == b' ' || b.is_ascii_graphic()) {
                Some(t)
            } else {
                // Shown as bytes: wallet text fields are printable ASCII only.
                Some(format!("text memo, UTF-8 0x{}", hex::encode(t.as_bytes())))
            }
        }
        _ => {
            let end = memo.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
            Some(format!("binary memo 0x{}", hex::encode(&memo[..end])))
        }
    }
}

/// One shielded output recovered with a viewing key: who can read it (the
/// recipient's IVK or the sender's OVK) and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decrypted {
    pub pool: &'static str,
    pub index: usize,
    /// Raw 43-byte receiver (diversifier || pk_d).
    pub recipient: [u8; 43],
    pub value: u64,
    pub memo: [u8; 512],
    pub by_ovk: bool,
}

/// Trial-decrypt every action of an Orchard or Ironwood bundle, first with the
/// incoming viewing keys and then with the outgoing ones.
pub fn decrypt_orchard<T: orchard::bundle::Authorization, V>(
    pool: &'static str,
    bundle: &orchard::Bundle<T, V>,
    ivks: &[orchard::keys::IncomingViewingKey],
    ovks: &[orchard::keys::OutgoingViewingKey],
) -> Vec<Option<Decrypted>> {
    (0..bundle.actions().len())
        .map(|i| {
            let incoming = ivks
                .iter()
                .find_map(|k| bundle.decrypt_output_with_key(i, k))
                .map(|r| (r, false));
            let found = incoming.or_else(|| {
                ovks.iter()
                    .find_map(|k| bundle.recover_output_with_ovk(i, k))
                    .map(|r| (r, true))
            });
            found.map(|((note, addr, memo), by_ovk)| Decrypted {
                pool,
                index: i,
                recipient: addr.to_raw_address_bytes(),
                value: note.value().inner(),
                memo,
                by_ovk,
            })
        })
        .collect()
}

/// Trial-decrypt every output of a Sapling bundle (ZIP 212 enforced).
pub fn decrypt_sapling<A: sapling_crypto::bundle::Authorization>(
    bundle: &sapling_crypto::Bundle<A, ZatBalance>,
    ivks: &[sapling_crypto::keys::PreparedIncomingViewingKey],
    ovks: &[sapling_crypto::keys::OutgoingViewingKey],
) -> Vec<Option<Decrypted>> {
    bundle
        .shielded_outputs()
        .iter()
        .enumerate()
        .map(|(i, out)| {
            let incoming = ivks
                .iter()
                .find_map(|k| try_sapling_note_decryption(k, out, Zip212Enforcement::On))
                .map(|r| (r, false));
            let found = incoming.or_else(|| {
                let domain = SaplingDomain::new(Zip212Enforcement::On);
                ovks.iter()
                    .find_map(|k| {
                        try_output_recovery_with_ovk(
                            &domain,
                            k,
                            out,
                            out.cv(),
                            out.out_ciphertext(),
                        )
                    })
                    .map(|r| (r, true))
            });
            found.map(|((note, addr, memo), by_ovk)| Decrypted {
                pool: "Sapling",
                index: i,
                recipient: addr.to_bytes(),
                value: note.value().inner(),
                memo,
                by_ovk,
            })
        })
        .collect()
}

/// Every shielded output of `tx` that one of the keys can read, in pool order
/// (Orchard, Ironwood, Sapling), with `None` for outputs no key reads.
pub fn decrypt_tx<A: Authorization>(
    tx: &TransactionData<A>,
    orchard_ivks: &[orchard::keys::IncomingViewingKey],
    orchard_ovks: &[orchard::keys::OutgoingViewingKey],
    sapling_ivks: &[sapling_crypto::keys::PreparedIncomingViewingKey],
    sapling_ovks: &[sapling_crypto::keys::OutgoingViewingKey],
) -> Vec<Option<Decrypted>> {
    let mut out = Vec::new();
    if let Some(b) = tx.orchard_bundle() {
        out.extend(decrypt_orchard("Orchard", b, orchard_ivks, orchard_ovks));
    }
    if let Some(b) = tx.ironwood_bundle() {
        out.extend(decrypt_orchard("Ironwood", b, orchard_ivks, orchard_ovks));
    }
    if let Some(b) = tx.sapling_bundle() {
        out.extend(decrypt_sapling(b, sapling_ivks, sapling_ovks));
    }
    out
}

/// The address a P2PKH or P2SH scriptPubKey pays, if it is one of those.
pub fn script_address(script: &[u8]) -> Option<TransparentAddress> {
    match script {
        [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] if hash.len() == 20 => {
            Some(TransparentAddress::PublicKeyHash(hash.try_into().unwrap()))
        }
        [0xa9, 0x14, hash @ .., 0x87] if hash.len() == 20 => {
            Some(TransparentAddress::ScriptHash(hash.try_into().unwrap()))
        }
        _ => None,
    }
}

/// A spent transparent output: value and scriptPubKey.
pub struct Prevout {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

fn bytes(b: &[u8]) -> Value {
    json!(b)
}

fn orchard_bundle_json<V: Copy + Into<i64>>(
    b: &orchard::Bundle<orchard::bundle::Authorized, V>,
    note_version: &str,
) -> Value {
    let vb: i64 = (*b.value_balance()).into();
    let actions: Vec<Value> = b
        .actions()
        .iter()
        .map(|a| {
            let note = a.encrypted_note();
            json!({
                "cv_net": bytes(&a.cv_net().to_bytes()),
                "spend": {
                    "nullifier": bytes(&a.nullifier().to_bytes()),
                    "rk": bytes(&<[u8; 32]>::from(a.rk())),
                    "spend_auth_sig": bytes(&<[u8; 64]>::from(a.authorization())),
                    "recipient": null, "value": null, "rho": null, "rseed": null,
                    "fvk": null, "witness": null, "alpha": null,
                    "zip32_derivation": null, "dummy_sk": null, "proprietary": {},
                },
                "output": {
                    "cmx": bytes(&a.cmx().to_bytes()),
                    "ephemeral_key": bytes(&note.epk_bytes),
                    "enc_ciphertext": {"Encrypted": bytes(note.enc_ciphertext.as_ref())},
                    "out_ciphertext": bytes(&note.out_ciphertext),
                    "recipient": null, "value": null, "rseed": null, "ock": null,
                    "zip32_derivation": null, "user_address": null, "proprietary": {},
                },
                "rcv": null,
            })
        })
        .collect();
    json!({
        "actions": actions,
        "flags": b.flags().to_byte(b.bundle_version()).unwrap(),
        "value_sum": [vb.unsigned_abs(), vb < 0],
        "anchor": bytes(&b.anchor().to_bytes()),
        "note_version": note_version,
        "zkproof": bytes(b.authorization().proof().as_ref()),
        "bsk": null,
    })
}

/// Rebuild the PCZT a coordinator would have handed the signer for `tx`: every
/// effecting field and proof, the spent outputs, SIGHASH_ALL on every
/// transparent input, and no note plaintexts (they are not on chain).
pub fn pczt_from_tx(tx: &Transaction, prevouts: &[Prevout], coin_type: u32) -> Vec<u8> {
    let transparent = tx.transparent_bundle().map(|b| {
        assert_eq!(b.vin.len(), prevouts.len(), "one prevout per input");
        let inputs: Vec<Value> = b
            .vin
            .iter()
            .zip(prevouts)
            .map(|(i, p)| {
                json!({
                    "prevout_txid": bytes(i.prevout().hash()),
                    "prevout_index": i.prevout().n(),
                    "sequence": i.sequence(),
                    "required_time_lock_time": null,
                    "required_height_lock_time": null,
                    "script_sig": null,
                    "value": p.value,
                    "script_pubkey": bytes(&p.script_pubkey),
                    "redeem_script": null,
                    "partial_signatures": {},
                    "sighash_type": 1,
                    "bip32_derivation": {},
                    "ripemd160_preimages": {},
                    "sha256_preimages": {},
                    "hash160_preimages": {},
                    "hash256_preimages": {},
                    "proprietary": {},
                })
            })
            .collect();
        let outputs: Vec<Value> = b
            .vout
            .iter()
            .map(|o| {
                json!({
                    "value": u64::from(o.value()),
                    "script_pubkey": bytes(&o.script_pubkey().0.0),
                    "redeem_script": null,
                    "bip32_derivation": {},
                    "user_address": null,
                    "proprietary": {},
                })
            })
            .collect();
        json!({ "inputs": inputs, "outputs": outputs })
    });
    let sapling = tx.sapling_bundle().map(|b| {
        let anchor = b
            .shielded_spends()
            .first()
            .map(|s| s.anchor().to_bytes())
            .unwrap_or([0; 32]);
        let spends: Vec<Value> = b
            .shielded_spends()
            .iter()
            .map(|s| {
                json!({
                    "cv": bytes(&s.cv().to_bytes()),
                    "nullifier": bytes(&s.nullifier().0),
                    "rk": bytes(&<[u8; 32]>::from(*s.rk())),
                    "zkproof": bytes(s.zkproof()),
                    "spend_auth_sig": bytes(&<[u8; 64]>::from(*s.spend_auth_sig())),
                    "recipient": null, "value": null, "rcm": null, "rseed": null,
                    "rcv": null, "proof_generation_key": null, "witness": null,
                    "alpha": null, "zip32_derivation": null, "dummy_ask": null,
                    "proprietary": {},
                })
            })
            .collect();
        let outputs: Vec<Value> = b
            .shielded_outputs()
            .iter()
            .map(|o| {
                json!({
                    "cv": bytes(&o.cv().to_bytes()),
                    "cmu": bytes(&o.cmu().to_bytes()),
                    "ephemeral_key": bytes(&o.ephemeral_key().0),
                    "enc_ciphertext": bytes(o.enc_ciphertext().as_ref()),
                    "out_ciphertext": bytes(o.out_ciphertext()),
                    "zkproof": bytes(o.zkproof()),
                    "recipient": null, "value": null, "rseed": null, "rcv": null,
                    "ock": null, "zip32_derivation": null, "user_address": null,
                    "proprietary": {},
                })
            })
            .collect();
        json!({
            "spends": spends,
            "outputs": outputs,
            "value_sum": i64::from(*b.value_balance()),
            "anchor": bytes(&anchor),
            "bsk": null,
        })
    });
    let orchard = tx.orchard_bundle().map(|b| orchard_bundle_json(b, "V2"));
    let ironwood = tx.ironwood_bundle().map(|b| orchard_bundle_json(b, "V3"));
    let doc = json!({
        "global": {
            "tx_version": tx.version().header() & 0x7fff_ffff,
            "version_group_id": tx.version().version_group_id(),
            "consensus_branch_id": u32::from(tx.consensus_branch_id()),
            "fallback_lock_time": tx.lock_time(),
            "expiry_height": u32::from(tx.expiry_height()),
            "coin_type": coin_type,
            "tx_modifiable": 0,
            "proprietary": {},
        },
        "transparent": transparent,
        "sapling": sapling,
        "orchard": orchard,
        "ironwood": ironwood,
    });
    serde_json::from_value::<pczt::v2::Pczt>(doc)
        .unwrap()
        .serialize()
}

/// Edit a PCZT through its public v2 serialization model. Transparent BIP 32
/// derivations are cleared first: their byte-array keys have no JSON form.
pub fn edit_pczt(b: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
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
