#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Fixture-driven tests for the Bitcoin PSBT parser.
//!
//! Fixture provenance:
//! - `bip174-*`: test vectors copied verbatim from BIP-174
//!   (<https://github.com/bitcoin/bips/blob/master/bip-0174.mediawiki>, "Test Vectors").
//! - `bip371-*`: test vectors copied verbatim from BIP-371
//!   (<https://github.com/bitcoin/bips/blob/master/bip-0371.mediawiki>, "Test Vectors").
//!   The BIP-174 and BIP-371 vectors were generated on testnet, but a PSBT does not
//!   name its network, so the golden files render addresses with the default
//!   mainnet encoding.
//! - `synthetic-*`: built by this file from the BIP-39 "abandon ... about" seed
//!   with the BIP-84 / BIP-86 account-0 paths (the keys of the BIP-84 and BIP-86
//!   test vectors, whose first receive addresses are asserted below). The previous
//!   transactions are synthetic. `REGEN_FIXTURES=1` rewrites their `.input` files.

use base64::Engine as _;
use bitcoin::bip32::{DerivationPath, Fingerprint, Xpriv};
use bitcoin::hashes::Hash;
use bitcoin::key::TapTweak;
use bitcoin::opcodes::all::{OP_CHECKMULTISIG, OP_CHECKSIG, OP_CHECKSIGADD, OP_NUMEQUAL};
use bitcoin::psbt::{Psbt, PsbtSighashType};
use bitcoin::script::Builder;
use bitcoin::secp256k1::{PublicKey, Secp256k1, XOnlyPublicKey};
use bitcoin::taproot::{LeafVersion, TapLeafHash, TapTree, TaprootBuilder};
use bitcoin::{
    Address, Amount, EcdsaSighashType, Network, NetworkKind, OutPoint, ScriptBuf, Sequence,
    TapSighashType, Transaction, TxIn, TxOut, Txid, Witness, absolute, transaction,
};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use visualsign::vsptrait::{
    Transaction as VsTransaction, VisualSignConverter, VisualSignConverterFromString,
    VisualSignError, VisualSignOptions,
};
use visualsign_bitcoin::{BitcoinTransactionWrapper, BitcoinVisualSignConverter};

// BIP-39 seed of "abandon abandon abandon abandon abandon abandon abandon abandon
// abandon abandon abandon about" with an empty passphrase (BIP-84 / BIP-86 vectors).
const SEED_HEX: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";

fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

fn regen() -> bool {
    std::env::var("REGEN_FIXTURES")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

fn convert_with(
    data: &str,
    network: Network,
) -> Result<visualsign::SignablePayload, VisualSignError> {
    BitcoinVisualSignConverter::new(network)
        .to_visual_sign_payload_from_string(data, VisualSignOptions::default())
        .map(|r| r.payload)
}

fn convert_json(data: &str) -> String {
    convert_with(data, Network::Bitcoin)
        .unwrap()
        .to_json()
        .unwrap()
}

fn read_input(name: &str) -> String {
    fs::read_to_string(fixture_path(&format!("{name}.input")))
        .unwrap_or_else(|_| panic!("missing fixture {name}.input"))
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Synthetic PSBT builders
// ---------------------------------------------------------------------------

struct Keys {
    master: Xpriv,
    fingerprint: Fingerprint,
}

fn keys() -> Keys {
    let secp = Secp256k1::new();
    let seed = hex::decode(SEED_HEX).unwrap();
    let master = Xpriv::new_master(NetworkKind::Main, &seed).unwrap();
    let fingerprint = master.fingerprint(&secp);
    Keys {
        master,
        fingerprint,
    }
}

fn path(p: &str) -> DerivationPath {
    DerivationPath::from_str(p).unwrap()
}

fn pubkey(p: &str) -> PublicKey {
    let secp = Secp256k1::new();
    keys()
        .master
        .derive_priv(&secp, &path(p))
        .unwrap()
        .private_key
        .public_key(&secp)
}

fn xonly(p: &str) -> XOnlyPublicKey {
    pubkey(p).x_only_public_key().0
}

fn p2wpkh(p: &str) -> ScriptBuf {
    ScriptBuf::new_p2wpkh(&bitcoin::CompressedPublicKey(pubkey(p)).wpubkey_hash())
}

fn p2tr(p: &str) -> ScriptBuf {
    let secp = Secp256k1::verification_only();
    let (tweaked, _) = xonly(p).tap_tweak(&secp, None);
    ScriptBuf::new_p2tr_tweaked(tweaked)
}

fn prev_tx(script: ScriptBuf, value: u64, tag: u32) -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::from_consensus(tag),
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::all_zeros(), tag),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey: script,
        }],
    }
}

fn spend(prev: &Transaction, outputs: Vec<TxOut>) -> Psbt {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(prev.compute_txid(), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: outputs,
    };
    Psbt::from_unsigned_tx(tx).unwrap()
}

fn txout(script: ScriptBuf, value: u64) -> TxOut {
    TxOut {
        value: Amount::from_sat(value),
        script_pubkey: script,
    }
}

/// BIP-84 single-sig: 100_000 sat in, 60_000 to a third party, 39_000 change
/// (carrying derivation data), 1_000 fee. Both UTXO fields present.
fn p2wpkh_single_sig() -> Psbt {
    let k = keys();
    let prev = prev_tx(p2wpkh("m/84'/0'/0'/0/0"), 100_000, 1);
    let mut p = spend(
        &prev,
        vec![
            txout(p2wpkh("m/84'/0'/7'/0/0"), 60_000),
            txout(p2wpkh("m/84'/0'/0'/1/0"), 39_000),
        ],
    );
    p.inputs[0].witness_utxo = Some(prev.output[0].clone());
    p.inputs[0].non_witness_utxo = Some(prev);
    p.inputs[0].bip32_derivation.insert(
        pubkey("m/84'/0'/0'/0/0"),
        (k.fingerprint, path("m/84'/0'/0'/0/0")),
    );
    p.outputs[1].bip32_derivation.insert(
        pubkey("m/84'/0'/0'/1/0"),
        (k.fingerprint, path("m/84'/0'/0'/1/0")),
    );
    p
}

/// Same spend with the previous transaction omitted: the segwit v0 amount is
/// only claimed.
fn p2wpkh_without_prev_tx() -> Psbt {
    let mut p = p2wpkh_single_sig();
    p.inputs[0].non_witness_utxo = None;
    p
}

/// A previous transaction that does not hash to the prevout txid.
fn p2wpkh_tampered_non_witness_utxo() -> Psbt {
    let mut p = p2wpkh_single_sig();
    p.inputs[0].non_witness_utxo = Some(prev_tx(p2wpkh("m/84'/0'/0'/0/0"), 100_000, 2));
    p
}

/// The classic segwit v0 fee attack: `witness_utxo` understates the amount that
/// the previous transaction pays.
fn p2wpkh_understated_witness_utxo() -> Psbt {
    let mut p = p2wpkh_single_sig();
    p.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(70_000);
    p
}

/// BIP-86 single-sig taproot key path: witness_utxo only, change carrying
/// `tap_key_origins`.
fn p2tr_single_sig() -> Psbt {
    let k = keys();
    let prev = prev_tx(p2tr("m/86'/0'/0'/0/0"), 100_000, 3);
    let mut p = spend(
        &prev,
        vec![
            txout(p2tr("m/86'/0'/5'/0/0"), 70_000),
            txout(p2tr("m/86'/0'/0'/1/0"), 29_000),
        ],
    );
    p.inputs[0].witness_utxo = Some(prev.output[0].clone());
    let x = xonly("m/86'/0'/0'/0/0");
    p.inputs[0].tap_internal_key = Some(x);
    p.inputs[0]
        .tap_key_origins
        .insert(x, (vec![], (k.fingerprint, path("m/86'/0'/0'/0/0"))));
    let c = xonly("m/86'/0'/0'/1/0");
    p.outputs[1].tap_internal_key = Some(c);
    p.outputs[1]
        .tap_key_origins
        .insert(c, (vec![], (k.fingerprint, path("m/86'/0'/0'/1/0"))));
    p
}

const NUMS_HEX: &str = "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

fn multi_a_script(k: i64, keys: &[XOnlyPublicKey]) -> ScriptBuf {
    let mut b = Builder::new();
    for (i, x) in keys.iter().enumerate() {
        b = b.push_x_only_key(x);
        b = b.push_opcode(if i == 0 { OP_CHECKSIG } else { OP_CHECKSIGADD });
    }
    b.push_int(k).push_opcode(OP_NUMEQUAL).into_script()
}

/// 2-of-3 `tr(internal, multi_a(2, ...))` spend with change back to the same
/// policy. Cosigner keys come from three BIP-86-style paths under the test seed.
fn tr_multi_a_with(internal: XOnlyPublicKey, leaf: Option<ScriptBuf>) -> Psbt {
    let k = keys();
    let secp = Secp256k1::new();
    let paths = ["m/86'/0'/0'/0/0", "m/86'/0'/1'/0/0", "m/86'/0'/2'/0/0"];
    let xs: Vec<XOnlyPublicKey> = paths.iter().map(|p| xonly(p)).collect();
    let script = leaf.unwrap_or_else(|| multi_a_script(2, &xs));
    let info = TaprootBuilder::new()
        .add_leaf(0, script.clone())
        .unwrap()
        .finalize(&secp, internal)
        .unwrap();
    let spk = ScriptBuf::new_p2tr_tweaked(info.output_key());
    let prev = prev_tx(spk.clone(), 100_000, 4);
    let mut p = spend(
        &prev,
        vec![txout(p2tr("m/86'/0'/5'/0/0"), 70_000), txout(spk, 29_000)],
    );
    p.inputs[0].witness_utxo = Some(prev.output[0].clone());
    p.inputs[0].tap_internal_key = Some(internal);
    let cb = info
        .control_block(&(script.clone(), LeafVersion::TapScript))
        .unwrap();
    p.inputs[0]
        .tap_scripts
        .insert(cb, (script.clone(), LeafVersion::TapScript));
    let leaf_hash = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    for (x, path_str) in xs.iter().zip(paths) {
        p.inputs[0]
            .tap_key_origins
            .insert(*x, (vec![leaf_hash], (k.fingerprint, path(path_str))));
        p.outputs[1]
            .tap_key_origins
            .insert(*x, (vec![leaf_hash], (k.fingerprint, path(path_str))));
    }
    p.outputs[1].tap_internal_key = Some(internal);
    p.outputs[1].tap_tree =
        Some(TapTree::try_from(TaprootBuilder::new().add_leaf(0, script).unwrap()).unwrap());
    p
}

fn nums() -> XOnlyPublicKey {
    XOnlyPublicKey::from_str(NUMS_HEX).unwrap()
}

fn tr_multi_a_nums() -> Psbt {
    tr_multi_a_with(nums(), None)
}

/// Same policy, but the internal key is a real key: it can spend alone.
fn tr_multi_a_internal_key_spend() -> Psbt {
    tr_multi_a_with(xonly("m/86'/0'/9'/0/0"), None)
}

fn hex_of(p: &Psbt) -> String {
    hex::encode(p.serialize())
}

fn encode(p: &Psbt) -> String {
    hex_of(p)
}

fn json_of(p: &Psbt) -> String {
    convert_json(&encode(p))
}

type SyntheticBuilder = (&'static str, fn() -> Psbt);

const SYNTHETIC: [SyntheticBuilder; 7] = [
    ("synthetic-tr-multi-a-2of3", tr_multi_a_nums),
    (
        "synthetic-tr-multi-a-internal-key-spend",
        tr_multi_a_internal_key_spend,
    ),
    ("synthetic-p2wpkh-single-sig", p2wpkh_single_sig),
    ("synthetic-p2wpkh-no-prev-tx", p2wpkh_without_prev_tx),
    (
        "synthetic-p2wpkh-tampered-non-witness-utxo",
        p2wpkh_tampered_non_witness_utxo,
    ),
    (
        "synthetic-p2wpkh-understated-witness-utxo",
        p2wpkh_understated_witness_utxo,
    ),
    ("synthetic-p2tr-single-sig", p2tr_single_sig),
];

/// PSBTs that must decode, with their golden output.
static VALID_FIXTURES: [&str; 13] = [
    "synthetic-tr-multi-a-2of3",
    "synthetic-tr-multi-a-internal-key-spend",
    "bip174-p2pkh",
    "bip174-p2pkh-and-p2sh-p2wpkh-signed",
    "bip174-p2sh-p2wsh-multisig",
    "bip174-p2wsh-multisig",
    "bip174-unknown-types",
    "bip371-p2tr-keypath-input",
    "bip371-p2tr-keypath-output",
    "bip371-p2tr-scriptpath-input",
    "synthetic-p2wpkh-single-sig",
    "synthetic-p2wpkh-no-prev-tx",
    "synthetic-p2tr-single-sig",
];

/// PSBTs that must be rejected.
static REJECTED_FIXTURES: [&str; 7] = [
    "bip174-invalid-not-psbt",
    "bip174-invalid-missing-outputs",
    "bip174-invalid-scriptsig-in-unsigned-tx",
    "bip174-invalid-duplicate-keys",
    "bip371-invalid-tap-key-sig-too-short",
    "synthetic-p2wpkh-tampered-non-witness-utxo",
    "synthetic-p2wpkh-understated-witness-utxo",
];

#[test]
fn synthetic_keys_match_bip84_and_bip86_vectors() {
    let secp = Secp256k1::verification_only();
    let bip84 = Address::p2wpkh(
        &bitcoin::CompressedPublicKey(pubkey("m/84'/0'/0'/0/0")),
        Network::Bitcoin,
    );
    assert_eq!(
        bip84.to_string(),
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
    );
    let bip86 = Address::p2tr(&secp, xonly("m/86'/0'/0'/0/0"), None, Network::Bitcoin);
    assert_eq!(
        bip86.to_string(),
        "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"
    );
}

#[test]
fn synthetic_fixtures_are_reproducible() {
    for (name, build) in SYNTHETIC {
        let built = hex_of(&build());
        let path = fixture_path(&format!("{name}.input"));
        if regen() {
            fs::write(&path, format!("{built}\n")).unwrap();
            continue;
        }
        assert_eq!(built, read_input(name), "fixture {name} is stale");
    }
}

#[test]
fn test_with_fixtures() {
    for name in VALID_FIXTURES {
        let actual = convert_json(&read_input(name));
        let expected_path = fixture_path(&format!("{name}.expected"));
        // `REGEN_FIXTURES=1 cargo test -p visualsign-bitcoin` rewrites every `.expected`
        // from the current output. Review the diff: an intended change and a regression
        // look identical here.
        if regen() {
            fs::write(&expected_path, format!("{}\n", actual.trim_end())).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing {expected_path:?}"));
        assert_eq!(actual.trim(), expected.trim(), "fixture '{name}' differs");
    }
}

#[test]
fn rejected_fixtures_fail_to_convert() {
    for name in REJECTED_FIXTURES {
        let result = convert_with(&read_input(name), Network::Bitcoin);
        assert!(result.is_err(), "{name} should be rejected, got {result:?}");
    }
}

#[test]
fn output_is_ascii_only() {
    for name in VALID_FIXTURES {
        let json = convert_json(&read_input(name));
        assert!(json.is_ascii(), "{name} produced non-ASCII output");
    }
}

// ---------------------------------------------------------------------------
// Behaviour
// ---------------------------------------------------------------------------

#[test]
fn hex_base64_and_prefixed_hex_agree() {
    let p = p2wpkh_single_sig();
    let bytes = p.serialize();
    let as_hex = hex::encode(&bytes);
    let as_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let expected = convert_json(&as_hex);
    assert_eq!(convert_json(&as_b64), expected);
    assert_eq!(convert_json(&format!("0x{as_hex}")), expected);
    assert_eq!(convert_json(&format!("  {as_hex}\n")), expected);
}

#[test]
fn verified_fee_and_all_outputs_are_reported() {
    let json = json_of(&p2wpkh_single_sig());
    assert!(json.contains(r#""Label":"Fee""#));
    assert!(
        json.contains(r#""FallbackText":"0.00001000 BTC""#),
        "{json}"
    );
    assert!(!json.contains("claims change"));
    assert!(!json.contains(r#""Label":"Change""#));
    assert!(json.contains("0.00039000 BTC"));
    assert!(json.contains("0.00060000 BTC"));
    assert!(json.contains("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"));
    assert!(json.contains("verified against previous transaction"));
}

#[test]
fn segwit_v0_without_prev_tx_flags_fee_unverified() {
    let json = json_of(&p2wpkh_without_prev_tx());
    assert!(
        json.contains("unknown (claimed 0.00001000 BTC; unverified:"),
        "{json}"
    );
    assert!(json.contains("claimed by witness_utxo only"));
}

#[test]
fn all_taproot_inputs_bind_the_fee() {
    let json = json_of(&p2tr_single_sig());
    assert!(!json.contains("unverified: input amounts"), "{json}");
    assert!(
        json.contains(r#""FallbackText":"0.00001000 BTC""#),
        "{json}"
    );
    assert!(!json.contains("claims change"));
}

#[test]
fn mixed_taproot_and_v0_claims_are_unverified() {
    // One claimed taproot input next to one claimed v0 input: a v0 signature does
    // not commit to the taproot amount, so the fee cannot be trusted.
    let v0_prev = prev_tx(p2wpkh("m/84'/0'/0'/0/0"), 50_000, 7);
    let tr_prev = prev_tx(p2tr("m/86'/0'/0'/0/0"), 50_000, 8);
    let mut p = p2tr_single_sig();
    p.unsigned_tx.input = vec![
        TxIn {
            previous_output: OutPoint::new(v0_prev.compute_txid(), 0),
            ..p.unsigned_tx.input[0].clone()
        },
        TxIn {
            previous_output: OutPoint::new(tr_prev.compute_txid(), 0),
            ..p.unsigned_tx.input[0].clone()
        },
    ];
    let v0_in = bitcoin::psbt::Input {
        witness_utxo: Some(v0_prev.output[0].clone()),
        ..Default::default()
    };
    let tr_in = bitcoin::psbt::Input {
        witness_utxo: Some(tr_prev.output[0].clone()),
        ..Default::default()
    };
    p.inputs = vec![v0_in, tr_in];
    let json = json_of(&p);
    assert!(json.contains("; unverified: input amounts"), "{json}");
}

#[test]
fn missing_utxo_data_means_unknown_fee() {
    let mut p = p2wpkh_single_sig();
    p.inputs[0].witness_utxo = None;
    p.inputs[0].non_witness_utxo = None;
    let json = json_of(&p);
    assert!(json.contains("unknown (input amounts missing)"), "{json}");
    assert!(json.contains("amount unknown"));
}

#[test]
fn outputs_exceeding_inputs_are_rejected() {
    let mut p = p2wpkh_single_sig();
    p.unsigned_tx.output[0].value = Amount::from_sat(70_000);
    let err = convert_with(&encode(&p), Network::Bitcoin).unwrap_err();
    assert!(err.to_string().contains("outputs exceed inputs"), "{err}");
}

#[test]
fn tampered_non_witness_utxo_error_names_the_cause() {
    let err = convert_with(
        &encode(&p2wpkh_tampered_non_witness_utxo()),
        Network::Bitcoin,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("does not match the prevout txid"),
        "{err}"
    );
    let err = convert_with(
        &encode(&p2wpkh_understated_witness_utxo()),
        Network::Bitcoin,
    )
    .unwrap_err();
    assert!(err.to_string().contains("witness_utxo disagrees"), "{err}");
}

#[test]
fn prevout_index_out_of_range_is_rejected() {
    let mut p = p2wpkh_single_sig();
    p.unsigned_tx.input[0].previous_output.vout = 5;
    let err = convert_with(&encode(&p), Network::Bitcoin).unwrap_err();
    assert!(err.to_string().contains("out of range"), "{err}");
}

#[test]
fn non_default_sighash_is_surfaced() {
    let mut p = p2wpkh_single_sig();
    p.inputs[0].sighash_type = Some(PsbtSighashType::from(EcdsaSighashType::AllPlusAnyoneCanPay));
    let json = json_of(&p);
    assert!(json.contains("Sighash Type"), "{json}");
    assert!(json.contains("SIGHASH_ALL|SIGHASH_ANYONECANPAY"), "{json}");

    let mut p = p2wpkh_single_sig();
    p.inputs[0].sighash_type = Some(PsbtSighashType::from(EcdsaSighashType::All));
    assert!(!json_of(&p).contains("Sighash Type"));
}

#[test]
fn op_return_output_shows_script_not_address() {
    let mut p = p2wpkh_single_sig();
    let data = bitcoin::script::PushBytesBuf::try_from(b"hello".to_vec()).unwrap();
    p.unsigned_tx
        .output
        .push(txout(ScriptBuf::new_op_return(data), 0));
    p.outputs.push(Default::default());
    let json = json_of(&p);
    assert!(json.contains("OP_RETURN"), "{json}");
    assert!(json.contains("6a0568656c6c6f"), "{json}");
}

#[test]
fn network_selects_the_address_encoding() {
    let data = encode(&p2wpkh_single_sig());
    let mainnet = convert_with(&data, Network::Bitcoin)
        .unwrap()
        .to_json()
        .unwrap();
    let testnet = convert_with(&data, Network::Testnet)
        .unwrap()
        .to_json()
        .unwrap();
    assert!(mainnet.contains("Bitcoin Mainnet") && mainnet.contains("bc1q"));
    assert!(testnet.contains("Bitcoin Testnet") && testnet.contains("tb1q"));
    assert!(!testnet.contains("bc1q"));
}

#[test]
fn transaction_name_overrides_title() {
    let wrapper = BitcoinTransactionWrapper::from_string(&encode(&p2wpkh_single_sig())).unwrap();
    let options = VisualSignOptions {
        transaction_name: Some("Send BTC".into()),
        ..Default::default()
    };
    let payload = BitcoinVisualSignConverter::default()
        .to_visual_sign_payload(wrapper, options)
        .unwrap()
        .payload;
    assert_eq!(payload.title, "Send BTC");
    assert_eq!(payload.payload_type, "BitcoinTx");
}

#[test]
fn malformed_and_oversized_input_is_rejected() {
    for bad in ["", "not a psbt", "70736274ff", "zz", "0x", "cHNidP8"] {
        assert!(
            BitcoinTransactionWrapper::from_string(bad).is_err(),
            "{bad:?} should be rejected"
        );
    }
    let huge = "a".repeat(4 * 1024 * 1024 + 2);
    assert!(BitcoinTransactionWrapper::from_string(&huge).is_err());
}

#[test]
fn parse_network_accepts_known_names() {
    assert_eq!(
        visualsign_bitcoin::parse_network("bitcoin"),
        Ok(Network::Bitcoin)
    );
    assert_eq!(
        visualsign_bitcoin::parse_network("SIGNET"),
        Ok(Network::Signet)
    );
    assert!(visualsign_bitcoin::parse_network("litecoin").is_err());
}

#[test]
fn bip174_wsh_multisig_shows_threshold_and_origins() {
    let json = convert_json(&read_input("bip174-p2wsh-multisig"));
    assert!(json.contains("2 of 2 (P2WSH)"), "{json}");
    assert!(json.contains(r#""Label":"Cosigner 1""#));
    assert!(json.contains(r#""Label":"Cosigner 2""#));
    // The vector carries bip32_derivation for both keys.
    assert!(!json.contains("no origin"), "{json}");
    assert!(!json.contains("unrecognized script"));
}

#[test]
fn tr_multi_a_with_nums_internal_key_has_no_warning() {
    let json = json_of(&tr_multi_a_nums());
    assert!(json.contains("2 of 3 (P2TR script path)"), "{json}");
    assert!(json.contains(NUMS_HEX));
    assert!(!json.contains("internal key can spend alone"));
    assert!(json.contains("verified") || json.contains(r#""Label":"Fee""#));
    // The change output shows the same policy through its tap tree.
    assert_eq!(json.matches("2 of 3 (P2TR script path)").count(), 2 * 2);
}

#[test]
fn tr_multi_a_with_real_internal_key_warns() {
    let json = json_of(&tr_multi_a_internal_key_spend());
    assert!(json.contains("internal key can spend alone"), "{json}");
}

#[test]
fn single_key_tap_leaf_is_unrecognized() {
    let key = xonly("m/86'/0'/0'/0/0");
    let leaf = Builder::new()
        .push_x_only_key(&key)
        .push_opcode(OP_CHECKSIG)
        .into_script();
    let json = json_of(&tr_multi_a_with(nums(), Some(leaf.clone())));
    assert!(json.contains("unrecognized script"), "{json}");
    assert!(json.contains(&leaf.to_hex_string()));
    assert!(!json.contains("P2TR script path"));
}

#[test]
fn witness_script_not_matching_the_output_is_unrecognized() {
    let mut p = p2wpkh_single_sig();
    let ws = ScriptBuf::from_hex("5121").unwrap();
    p.inputs[0].witness_script = Some(ws);
    let json = json_of(&p);
    assert!(
        json.contains("witness script does not match the output"),
        "{json}"
    );
    assert!(json.contains(r#""Script","#) || json.contains(r#""Label":"Script""#));
}

#[test]
fn tap_script_not_committed_by_the_output_is_unrecognized() {
    // Swap the prevout for a different taproot output: the control block no
    // longer commits to it.
    let mut p = tr_multi_a_nums();
    let other = prev_tx(p2tr("m/86'/0'/0'/0/0"), 100_000, 5);
    p.inputs[0].witness_utxo = Some(other.output[0].clone());
    let json = json_of(&p);
    assert!(json.contains("not committed to by the output"), "{json}");
    assert!(!json.contains("2 of 3 (P2TR script path)") || json.matches("2 of 3").count() == 2);
}

#[test]
fn bip174_p2sh_p2wsh_multisig_is_recognized() {
    let json = convert_json(&read_input("bip174-p2sh-p2wsh-multisig"));
    assert!(json.contains("2 of 2 (P2SH-P2WSH)"), "{json}");
}

#[test]
fn anyonecanpay_leaves_all_taproot_fees_unverified() {
    let sighash = TapSighashType::AllPlusAnyoneCanPay;
    let mut p = p2tr_single_sig();
    // Two inputs exercise the ANYONECANPAY amount-commitment gap.
    let prev = prev_tx(p2tr("m/86'/0'/1'/0/0"), 100_000, 10);
    p.unsigned_tx.input.push(TxIn {
        previous_output: OutPoint::new(prev.compute_txid(), 0),
        ..p.unsigned_tx.input[0].clone()
    });
    p.inputs.push(bitcoin::psbt::Input {
        witness_utxo: Some(prev.output[0].clone()),
        ..Default::default()
    });
    // Any weak input must downgrade the fee, even alongside DEFAULT.
    p.inputs[1].sighash_type = Some(sighash.into());
    let json = json_of(&p);
    assert!(
        json.contains("unknown (claimed 0.00101000 BTC; unverified: sighash"),
        "{sighash}: {json}"
    );
}

#[test]
fn anyonecanpay_leaves_prev_tx_verified_fees_unverified() {
    let sighash = EcdsaSighashType::AllPlusAnyoneCanPay;
    let mut p = p2wpkh_single_sig();
    p.inputs[0].sighash_type = Some(sighash.into());
    let json = json_of(&p);
    assert!(
        json.contains("unknown (claimed 0.00001000 BTC; unverified: sighash"),
        "{sighash}: {json}"
    );
}

#[test]
fn explicit_all_and_default_sighashes_keep_taproot_fee_verified() {
    for sighash in [TapSighashType::Default, TapSighashType::All] {
        let mut p = p2tr_single_sig();
        p.inputs[0].sighash_type = Some(sighash.into());
        let json = json_of(&p);
        assert!(!json.contains("unverified:"), "{json}");
        assert!(
            json.contains(r#""FallbackText":"0.00001000 BTC","Label":"Fee""#),
            "{json}"
        );
    }
}

#[test]
fn repeated_wsh_keys_are_shown_as_raw_scripts() {
    let a = pubkey("m/84'/0'/0'/0/0");
    let v = pubkey("m/84'/0'/1'/0/0");
    let script = Builder::new()
        .push_int(2)
        .push_slice(a.serialize())
        .push_slice(a.serialize())
        .push_slice(v.serialize())
        .push_int(3)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script();
    for wrapped in [false, true] {
        let redeem = script.to_p2wsh();
        let spk = if wrapped {
            redeem.to_p2sh()
        } else {
            redeem.clone()
        };
        let prev = prev_tx(spk.clone(), 100_000, 9);
        let mut p = spend(&prev, vec![txout(spk, 99_000)]);
        p.inputs[0].witness_utxo = Some(prev.output[0].clone());
        p.inputs[0].witness_script = Some(script.clone());
        p.outputs[0].witness_script = Some(script.clone());
        if wrapped {
            p.inputs[0].redeem_script = Some(redeem.clone());
            p.outputs[0].redeem_script = Some(redeem);
        }
        let json = json_of(&p);
        assert!(json.contains("unrecognized script"), "{json}");
        assert!(json.contains(&script.to_hex_string()));
        assert!(!json.contains("Multisig"), "{json}");
    }
}

#[test]
fn repeated_taproot_keys_are_shown_as_raw_scripts() {
    let a = xonly("m/86'/0'/0'/0/0");
    let v = xonly("m/86'/0'/1'/0/0");
    let script = multi_a_script(2, &[a, a, v]);
    let json = json_of(&tr_multi_a_with(nums(), Some(script.clone())));
    assert!(json.contains("unrecognized script"), "{json}");
    assert!(json.contains(&script.to_hex_string()));
    assert!(!json.contains("Multisig"), "{json}");
}

#[test]
fn psbt_network_is_unknown_while_address_encoding_is_explicit() {
    for network in [Network::Bitcoin, Network::Testnet, Network::Signet] {
        let json = convert_with(&encode(&p2tr_single_sig()), network)
            .unwrap()
            .to_json()
            .unwrap();
        assert!(json.contains(r#""Label":"Address Encoding""#), "{json}");
        assert!(json.contains("transaction network unknown"), "{json}");
        assert!(!json.contains(r#""Label":"Network""#), "{json}");
    }
}

#[test]
fn claimed_input_amount_is_marked_in_preview_subtitle() {
    let claimed = json_of(&p2wpkh_without_prev_tx());
    assert!(
        claimed.contains("0.00100000 BTC (claimed amount, unverified) from"),
        "{claimed}"
    );
    let verified = json_of(&p2wpkh_single_sig());
    assert!(
        !verified.contains("claimed amount, unverified"),
        "{verified}"
    );
}

// Review regressions: these exercise the public conversion boundary, including
// rust-bitcoin's otherwise permissive unknown-key and trailing-byte handling.
#[test]
fn review_v0_rejects_v2_only_fields() {
    for ty in 0x02..=0x06 {
        let mut p = p2wpkh_single_sig();
        p.unknown.insert(
            bitcoin::psbt::raw::Key {
                type_value: ty,
                key: vec![],
            },
            vec![0; 4],
        );
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "global {ty:#x}"
        );
    }
    for ty in 0x0e..=0x12 {
        let mut p = p2wpkh_single_sig();
        p.inputs[0].unknown.insert(
            bitcoin::psbt::raw::Key {
                type_value: ty,
                key: vec![],
            },
            vec![0; 4],
        );
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "input {ty:#x}"
        );
    }
    for ty in 0x03..=0x04 {
        let mut p = p2wpkh_single_sig();
        p.outputs[0].unknown.insert(
            bitcoin::psbt::raw::Key {
                type_value: ty,
                key: vec![],
            },
            vec![0; 4],
        );
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "output {ty:#x}"
        );
    }
}

#[test]
fn review_bip174_zero_input_psbt_has_unknown_fee() {
    let json = convert_json(&read_input("bip174-zero-inputs"));
    assert!(json.contains("unknown (input amounts missing)"), "{json}");
    assert!(json.contains("Output 0"), "{json}");
}

#[test]
fn review_duplicate_outpoints_are_rejected() {
    let mut p = p2wpkh_single_sig();
    p.unsigned_tx.input.push(p.unsigned_tx.input[0].clone());
    p.inputs.push(p.inputs[0].clone());
    let err = convert_with(&encode(&p), Network::Bitcoin)
        .map(|_| ())
        .expect_err("invalid PSBT must be rejected");
    assert!(
        err.to_string().contains("duplicate input outpoint"),
        "{err}"
    );
}

#[test]
fn review_unsigned_transaction_size_is_bounded() {
    let mut p = p2wpkh_single_sig();
    p.unsigned_tx.output = vec![txout(ScriptBuf::new(), 0); 12_000];
    p.outputs = vec![Default::default(); 12_000];
    // Only ~120 KiB of PSBT data, well below the encoded-input cap, but
    // 432k WU before signatures and tens of thousands of display fields.
    let err = convert_with(&encode(&p), Network::Bitcoin)
        .map(|_| ())
        .expect_err("invalid PSBT must be rejected");
    assert!(err.to_string().contains("400000 WU"), "{err}");
}

#[test]
fn review_money_range_is_enforced_for_values_and_totals() {
    let max = Amount::MAX_MONEY.to_sat();
    let mut p = p2wpkh_without_prev_tx();
    p.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(max + 1);
    assert!(
        convert_with(&encode(&p), Network::Bitcoin)
            .map(|_| ())
            .expect_err("invalid PSBT must be rejected")
            .to_string()
            .contains("MAX_MONEY")
    );
    let mut p = p2wpkh_single_sig();
    p.inputs[0].witness_utxo = None;
    p.inputs[0].non_witness_utxo = None;
    p.unsigned_tx.output[0].value = Amount::from_sat(max + 1);
    assert!(
        convert_with(&encode(&p), Network::Bitcoin)
            .map(|_| ())
            .expect_err("invalid PSBT must be rejected")
            .to_string()
            .contains("MAX_MONEY")
    );
    p.unsigned_tx.output[0].value = Amount::from_sat(max);
    p.unsigned_tx.output[1].value = Amount::from_sat(1);
    assert!(
        convert_with(&encode(&p), Network::Bitcoin)
            .map(|_| ())
            .expect_err("invalid PSBT must be rejected")
            .to_string()
            .contains("MAX_MONEY")
    );
    let mut p = p2wpkh_without_prev_tx();
    p.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(max);
    p.unsigned_tx.input.push(TxIn {
        previous_output: OutPoint::new(Txid::all_zeros(), 1),
        ..p.unsigned_tx.input[0].clone()
    });
    p.inputs.push(bitcoin::psbt::Input {
        witness_utxo: Some(txout(ScriptBuf::new(), 1)),
        ..Default::default()
    });
    assert!(
        convert_with(&encode(&p), Network::Bitcoin)
            .map(|_| ())
            .expect_err("invalid PSBT must be rejected")
            .to_string()
            .contains("MAX_MONEY")
    );
}

#[test]
fn review_none_and_single_are_refused_including_legacy_single_bug() {
    for sighash in [
        EcdsaSighashType::None,
        EcdsaSighashType::NonePlusAnyoneCanPay,
        EcdsaSighashType::Single,
        EcdsaSighashType::SinglePlusAnyoneCanPay,
    ] {
        let mut p = p2wpkh_single_sig();
        p.inputs[0].sighash_type = Some(sighash.into());
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "{sighash}"
        );
        let prev = prev_tx(
            Address::p2pkh(
                bitcoin::PublicKey::new(pubkey("m/84'/0'/0'/0/0")),
                Network::Bitcoin,
            )
            .script_pubkey(),
            100_000,
            11,
        );
        p.unsigned_tx.input[0].previous_output = OutPoint::new(prev.compute_txid(), 0);
        p.inputs[0].witness_utxo = None;
        p.inputs[0].non_witness_utxo = Some(prev);
        p.unsigned_tx.output.clear();
        p.outputs.clear();
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "out-of-range {sighash}"
        );
    }
    for sighash in [
        TapSighashType::None,
        TapSighashType::NonePlusAnyoneCanPay,
        TapSighashType::Single,
        TapSighashType::SinglePlusAnyoneCanPay,
    ] {
        let mut p = p2tr_single_sig();
        p.inputs[0].sighash_type = Some(sighash.into());
        assert!(
            convert_with(&encode(&p), Network::Bitcoin).is_err(),
            "{sighash}"
        );
    }
}

#[test]
fn review_trailing_bytes_are_rejected() {
    let encoded = format!("{}00", hex_of(&p2wpkh_single_sig()));
    assert!(BitcoinTransactionWrapper::from_string(&encoded).is_err());
}

#[test]
fn review_arbitrary_derivation_does_not_claim_change() {
    let mut p = p2wpkh_single_sig();
    p.outputs[0].bip32_derivation.insert(
        pubkey("m/84'/0'/99'/0/0"),
        (Fingerprint::from([1, 2, 3, 4]), path("m/84'/0'/99'/0/0")),
    );
    let json = json_of(&p);
    assert!(!json.contains("claims change"), "{json}");
    assert!(!json.contains(r#""Label":"Change""#), "{json}");
}

#[test]
fn review_claimed_fee_is_unknown_in_primary_display() {
    let json = json_of(&p2wpkh_without_prev_tx());
    assert!(json.contains("unknown (claimed 0.00001000 BTC;"), "{json}");
}

#[test]
fn review_version_sequence_and_effective_locktime_are_displayed() {
    let mut p = p2wpkh_single_sig();
    p.unsigned_tx.lock_time = absolute::LockTime::from_consensus(100);
    let json = json_of(&p);
    assert!(
        json.contains("inactive (all input sequences final)"),
        "{json}"
    );
    assert!(
        json.contains(r#""FallbackText":"2","Label":"Version""#),
        "{json}"
    );
    assert!(
        json.contains("0xffffffff (does not signal replaceability; relative lock disabled)"),
        "{json}"
    );
    p.unsigned_tx.input[0].sequence = Sequence::from_consensus(10);
    let json = json_of(&p);
    assert!(json.contains("block height 100"), "{json}");
    assert!(json.contains("signals replaceability"), "{json}");
    assert!(json.contains("relative lock: 10 blocks"), "{json}");
    p.unsigned_tx.lock_time = absolute::LockTime::from_consensus(500_000_123);
    p.unsigned_tx.input[0].sequence = Sequence::from_consensus((1 << 22) | 2);
    let json = json_of(&p);
    assert!(json.contains("Unix timestamp 500000123"), "{json}");
    assert!(json.contains("relative lock: 1024 seconds"), "{json}");
    p.unsigned_tx.version = transaction::Version::ONE;
    let json = json_of(&p);
    assert!(
        json.contains(r#""FallbackText":"1","Label":"Version""#),
        "{json}"
    );
    assert!(
        json.contains("relative lock inactive (version < 2)"),
        "{json}"
    );
    p.unsigned_tx.input[0].sequence = Sequence::from_consensus(0xfffffffe);
    let json = json_of(&p);
    assert!(json.contains("Unix timestamp 500000123"), "{json}");
    assert!(
        json.contains("0xfffffffe (does not signal replaceability; relative lock disabled)"),
        "{json}"
    );
}
