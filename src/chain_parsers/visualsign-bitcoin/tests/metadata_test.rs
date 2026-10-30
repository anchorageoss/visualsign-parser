#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Wallet-metadata (UTXO ownership) prototype tests.
//!
//! Keys come from the BIP-39 "abandon ... about" seed used by `lib_test.rs`
//! (our wallet) and from a fixed 32-byte seed (a third party). Run with
//! `--nocapture` to print the example renderings used in the design notes.

use bitcoin::bip32::{ChildNumber, DerivationPath, Fingerprint, Xpriv, Xpub};
use bitcoin::consensus::serialize;
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use bitcoin::{
    Amount, CompressedPublicKey, Network, NetworkKind, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness, absolute, transaction,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use visualsign::vsptrait::{
    Transaction as VsTransaction, VisualSignConverter, VisualSignConverterFromString,
    VisualSignOptions,
};
use visualsign_bitcoin::metadata::{
    Account, AccountKey, AccountPolicy, AccountScriptType, OutputLabel, OutputSelector,
    UtxoMetadata,
};
use visualsign_bitcoin::{
    BitcoinMetadata, BitcoinTransactionWrapper, BitcoinVisualSignConverter, Ownership,
};

const SEED_HEX: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";
const THIRD_PARTY_SEED: [u8; 32] = [7; 32];

/// A change to valid metadata, and the error it must cause.
type MetadataCase = (fn(&mut BitcoinMetadata), &'static str);

fn master(seed: &[u8]) -> Xpriv {
    Xpriv::new_master(NetworkKind::Main, seed).unwrap()
}

fn ours() -> Xpriv {
    master(&hex::decode(SEED_HEX).unwrap())
}

fn theirs() -> Xpriv {
    master(&THIRD_PARTY_SEED)
}

fn fingerprint() -> Fingerprint {
    ours().fingerprint(&Secp256k1::new())
}

fn path(p: &str) -> DerivationPath {
    DerivationPath::from_str(p).unwrap()
}

fn pubkey(root: &Xpriv, p: &str) -> PublicKey {
    let secp = Secp256k1::new();
    root.derive_priv(&secp, &path(p))
        .unwrap()
        .private_key
        .public_key(&secp)
}

fn p2wpkh(root: &Xpriv, p: &str) -> ScriptBuf {
    ScriptBuf::new_p2wpkh(&CompressedPublicKey(pubkey(root, p)).wpubkey_hash())
}

fn p2tr(root: &Xpriv, p: &str) -> ScriptBuf {
    let secp = Secp256k1::verification_only();
    ScriptBuf::new_p2tr(&secp, pubkey(root, p).x_only_public_key().0, None)
}

fn account(name: &str, origin: &str, script_type: AccountScriptType) -> Account {
    let secp = Secp256k1::new();
    let xpriv = ours().derive_priv(&secp, &path(origin)).unwrap();
    Account {
        name: name.to_string(),
        policy: AccountPolicy::SingleSig {
            key: AccountKey {
                master_fingerprint: fingerprint(),
                origin: path(origin),
                xpub: Xpub::from_priv(&secp, &xpriv),
            },
            script_type,
        },
        descriptor: None,
    }
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

fn spend(prev: &Transaction, outputs: Vec<(ScriptBuf, u64)>) -> Psbt {
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(prev.compute_txid(), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: outputs
            .into_iter()
            .map(|(script_pubkey, sat)| TxOut {
                value: Amount::from_sat(sat),
                script_pubkey,
            })
            .collect(),
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0].witness_utxo = Some(prev.output[0].clone());
    psbt
}

/// Savings (BIP-84 account 0) spends 100_000 sat: 60_000 to a third party,
/// 39_000 change to m/84'/0'/0'/1/7, 1_000 fee. The PSBT carries only
/// `witness_utxo`; the previous transaction comes from the metadata.
fn p2wpkh_spend() -> (Psbt, Transaction) {
    let k = ours();
    let prev = prev_tx(p2wpkh(&k, "m/84'/0'/0'/0/3"), 100_000, 1);
    let mut psbt = spend(
        &prev,
        vec![
            (p2wpkh(&theirs(), "m/84'/0'/0'/0/0"), 60_000),
            (p2wpkh(&k, "m/84'/0'/0'/1/7"), 39_000),
        ],
    );
    psbt.inputs[0].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/0'/0/3"),
        (fingerprint(), path("m/84'/0'/0'/0/3")),
    );
    psbt.outputs[1].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/0'/1/7"),
        (fingerprint(), path("m/84'/0'/0'/1/7")),
    );
    (psbt, prev)
}

fn savings() -> Account {
    account("Savings", "m/84'/0'/0'", AccountScriptType::P2wpkh)
}

fn p2wpkh_metadata(prev: &Transaction) -> BitcoinMetadata {
    BitcoinMetadata {
        network: Network::Bitcoin,
        accounts: vec![savings()],
        utxos: vec![UtxoMetadata {
            outpoint: OutPoint::new(prev.compute_txid(), 0),
            label: Some("Deposit #1234".to_string()),
            prev_tx: Some(serialize(prev)),
        }],
        output_labels: vec![OutputLabel {
            output: OutputSelector::Index(0),
            label: "Invoice 2024-117 (Acme Corp)".to_string(),
        }],
    }
}

/// BIP-86 single-sig spend with change carrying `tap_key_origins`.
fn p2tr_spend() -> Psbt {
    let k = ours();
    let prev = prev_tx(p2tr(&k, "m/86'/0'/0'/0/0"), 100_000, 2);
    let mut psbt = spend(
        &prev,
        vec![
            (p2tr(&theirs(), "m/86'/0'/0'/0/0"), 70_000),
            (p2tr(&k, "m/86'/0'/0'/1/2"), 29_000),
        ],
    );
    let input_key = pubkey(&k, "m/86'/0'/0'/0/0").x_only_public_key().0;
    psbt.inputs[0].tap_internal_key = Some(input_key);
    psbt.inputs[0].tap_key_origins.insert(
        input_key,
        (vec![], (fingerprint(), path("m/86'/0'/0'/0/0"))),
    );
    let change_key = pubkey(&k, "m/86'/0'/0'/1/2").x_only_public_key().0;
    psbt.outputs[1].tap_internal_key = Some(change_key);
    psbt.outputs[1].tap_key_origins.insert(
        change_key,
        (vec![], (fingerprint(), path("m/86'/0'/0'/1/2"))),
    );
    psbt
}

fn taproot_metadata() -> BitcoinMetadata {
    BitcoinMetadata {
        network: Network::Bitcoin,
        accounts: vec![account(
            "Hot wallet",
            "m/86'/0'/0'",
            AccountScriptType::P2tr,
        )],
        utxos: vec![],
        output_labels: vec![],
    }
}

/// The fake-change attack: output 1 pays a third party but carries a
/// derivation with our fingerprint and our key at a change path.
fn fake_change_spend() -> (Psbt, Transaction) {
    let (mut psbt, prev) = p2wpkh_spend();
    psbt.unsigned_tx.output[1].script_pubkey = p2wpkh(&theirs(), "m/84'/0'/0'/1/0");
    (psbt, prev)
}

fn convert(psbt: &Psbt, metadata: &BitcoinMetadata) -> Result<Value, String> {
    let wrapper = BitcoinTransactionWrapper::new(psbt.clone());
    let payload = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .to_visual_sign_payload_with_metadata(wrapper, VisualSignOptions::default(), metadata)
        .map_err(|e| e.to_string())?
        .payload;
    payload.validate_charset().unwrap();
    Ok(serde_json::from_str(&payload.to_json().unwrap()).unwrap())
}

fn field<'a>(json: &'a Value, label: &str) -> &'a Value {
    json["Fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["Label"] == label)
        .unwrap_or_else(|| panic!("no field {label}"))
}

fn expanded<'a>(field: &'a Value, label: &str) -> Option<&'a str> {
    field["PreviewLayout"]["Expanded"]["Fields"]
        .as_array()?
        .iter()
        .find(|f| f["Label"] == label)
        .and_then(|f| f["FallbackText"].as_str())
}

fn text(field: &Value) -> &str {
    field["FallbackText"].as_str().unwrap()
}

/// Plain-text view of a payload: each top-level field, then its expanded
/// children indented.
fn render_text(json: &Value) -> String {
    let mut out = String::new();
    for f in json["Fields"].as_array().unwrap() {
        out.push_str(&format!("{}: {}\n", f["Label"].as_str().unwrap(), text(f)));
        if let Some(kids) = f["PreviewLayout"]["Expanded"]["Fields"].as_array() {
            for k in kids {
                out.push_str(&format!(
                    "    {}: {}\n",
                    k["Label"].as_str().unwrap(),
                    text(k)
                ));
            }
        }
    }
    out
}

#[test]
fn p2wpkh_change_is_verified_and_attributed() {
    let (psbt, prev) = p2wpkh_spend();
    let json = convert(&psbt, &p2wpkh_metadata(&prev)).unwrap();
    println!("--- p2wpkh change verified ---\n{}", render_text(&json));

    let summary = field(&json, "Summary");
    assert_eq!(
        text(summary),
        "Sending 0.00060000 BTC | Returning to your accounts 0.00039000 BTC | Fee 0.00001000 BTC"
    );
    assert_eq!(
        expanded(summary, "Returning to your accounts"),
        Some("0.00039000 BTC to 1 output - verified from wallet metadata")
    );

    let change = field(&json, "Output 1");
    assert_eq!(
        expanded(change, "Ownership"),
        Some("Change - Savings (m/84'/0'/0'/1/7) - verified from wallet metadata")
    );
    let payment = field(&json, "Output 0");
    assert_eq!(
        expanded(payment, "Ownership"),
        Some("External - not one of your accounts in wallet metadata")
    );
    assert_eq!(
        expanded(payment, "Label"),
        Some("Invoice 2024-117 (Acme Corp) - label from wallet metadata")
    );

    let input = field(&json, "Input 0");
    assert!(text(input).starts_with(
        "From your account - Savings (m/84'/0'/0'/0/3) - verified from wallet metadata | Deposit #1234 - label from wallet metadata | "
    ));
    // The previous transaction from the metadata verifies the amount and fee,
    // and says so.
    assert_eq!(
        expanded(input, "Amount Source"),
        Some("verified against previous transaction from wallet metadata")
    );
    assert_eq!(field(&json, "Fee")["Type"], "amount_v2");
    assert_eq!(
        text(field(&json, "Fee Source")),
        "previous transactions for input 0 from wallet metadata"
    );
    assert!(!json.to_string().contains("WARNING"));
}

#[test]
fn p2tr_change_is_verified_from_tap_key_origins() {
    let json = convert(&p2tr_spend(), &taproot_metadata()).unwrap();
    println!("--- p2tr change verified ---\n{}", render_text(&json));
    assert_eq!(
        expanded(field(&json, "Output 1"), "Ownership"),
        Some("Change - Hot wallet (m/86'/0'/0'/1/2) - verified from wallet metadata")
    );
    assert_eq!(
        expanded(field(&json, "Input 0"), "Ownership"),
        Some("From your account - Hot wallet (m/86'/0'/0'/0/0) - verified from wallet metadata")
    );
    assert_eq!(
        text(field(&json, "Summary")),
        "Sending 0.00070000 BTC | Returning to your accounts 0.00029000 BTC | Fee 0.00001000 BTC (claimed; enforced by BIP-341 signature)"
    );
}

#[test]
fn fake_change_is_flagged_and_counted_as_sending() {
    let (psbt, prev) = fake_change_spend();
    let metadata = p2wpkh_metadata(&prev);
    let json = convert(&psbt, &metadata).unwrap();
    println!("--- fake change ---\n{}", render_text(&json));

    let summary = field(&json, "Summary");
    assert_eq!(
        text(summary),
        "Sending 0.00099000 BTC | Returning to your accounts 0.00000000 BTC | Fee 0.00001000 BTC"
    );
    assert_eq!(
        expanded(summary, "Sending"),
        Some(
            "0.00099000 BTC to 2 outputs not verified as yours (includes 1 falsely claiming to be yours)"
        )
    );
    assert_eq!(
        text(field(&json, "Warning")),
        "Output 1 claims to be your change but does not match your account; counted as sending"
    );
    let ownership = expanded(field(&json, "Output 1"), "Ownership").unwrap();
    assert_eq!(
        ownership,
        "WARNING: claims to be your change but does not match your account (script is not account Savings at m/84'/0'/0'/1/7); counted as sending"
    );

    let report = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .ownership_report(&psbt, &metadata)
        .unwrap();
    assert!(matches!(
        report.outputs[1].ownership,
        Ownership::ClaimedOwnUnverified { .. }
    ));
    assert_eq!(report.own_total, Amount::ZERO);
    assert_eq!(report.external_total, Amount::from_sat(99_000));
    assert_eq!(report.external_count, 2);
}

#[test]
fn ownership_report_exposes_the_classification() {
    let (psbt, prev) = p2wpkh_spend();
    let report = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .ownership_report(&psbt, &p2wpkh_metadata(&prev))
        .unwrap();
    assert_eq!(
        report.outputs[1].ownership,
        Ownership::OwnVerified {
            account: "Savings".to_string(),
            path: path("m/84'/0'/0'/1/7"),
        }
    );
    assert_eq!(report.outputs[0].ownership, Ownership::External);
    assert!(report.inputs[0].prev_tx_from_metadata);
    assert_eq!(report.inputs[0].label.as_deref(), Some("Deposit #1234"));
    assert_eq!(report.own_total, Amount::from_sat(39_000));
}

#[test]
fn label_for_an_outpoint_not_spent_is_rejected() {
    let (psbt, prev) = p2wpkh_spend();
    let mut metadata = p2wpkh_metadata(&prev);
    metadata.utxos.push(UtxoMetadata {
        outpoint: OutPoint::new(Txid::all_zeros(), 5),
        label: Some("Deposit #9".to_string()),
        prev_tx: None,
    });
    let err = convert(&psbt, &metadata).unwrap_err();
    assert!(err.contains("is not spent by this transaction"), "{err}");
}

#[test]
fn prev_tx_with_wrong_txid_is_rejected() {
    let (psbt, prev) = p2wpkh_spend();
    let mut metadata = p2wpkh_metadata(&prev);
    let other = prev_tx(p2wpkh(&ours(), "m/84'/0'/0'/0/3"), 100_000, 99);
    metadata.utxos[0].prev_tx = Some(serialize(&other));
    let err = convert(&psbt, &metadata).unwrap_err();
    assert!(err.contains("does not hash to its txid"), "{err}");
}

#[test]
fn prev_tx_contradicting_witness_utxo_is_rejected() {
    // The metadata's previous transaction is checked like a PSBT one: here it
    // proves the witness_utxo understates the input.
    let (mut psbt, prev) = p2wpkh_spend();
    psbt.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(70_000);
    let err = convert(&psbt, &p2wpkh_metadata(&prev)).unwrap_err();
    assert!(err.contains("witness_utxo disagrees"), "{err}");
}

#[test]
fn bad_output_labels_and_text_are_rejected() {
    let (psbt, prev) = p2wpkh_spend();
    let cases: [MetadataCase; 5] = [
        (
            |m| m.output_labels[0].output = OutputSelector::Index(2),
            "refers to an output this transaction does not have",
        ),
        (
            |m| m.output_labels[0].output = OutputSelector::Script(ScriptBuf::new()),
            "refers to an output this transaction does not have",
        ),
        (
            |m| m.output_labels[0].label = "Caf\u{e9}".to_string(),
            "printable ASCII",
        ),
        (
            |m| m.output_labels[0].label = "line\nbreak".to_string(),
            "printable ASCII",
        ),
        (
            |m| m.accounts[0].descriptor = Some("wpkh(...)".to_string()),
            "descriptors are not supported",
        ),
    ];
    for (mutate, want) in cases {
        let mut metadata = p2wpkh_metadata(&prev);
        mutate(&mut metadata);
        let err = convert(&psbt, &metadata).unwrap_err();
        assert!(err.contains(want), "{err}");
    }
}

#[test]
fn metadata_for_another_network_is_rejected() {
    let (psbt, prev) = p2wpkh_spend();
    let mut metadata = p2wpkh_metadata(&prev);
    metadata.network = Network::Testnet;
    let err = convert(&psbt, &metadata).unwrap_err();
    assert!(err.contains("metadata is for testnet"), "{err}");
}

#[test]
fn unusual_branch_is_verified_with_a_warning() {
    let k = ours();
    let (mut psbt, prev) = p2wpkh_spend();
    psbt.unsigned_tx.output[1].script_pubkey = p2wpkh(&k, "m/84'/0'/0'/5/20000");
    psbt.outputs[1].bip32_derivation.clear();
    psbt.outputs[1].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/0'/5/20000"),
        (fingerprint(), path("m/84'/0'/0'/5/20000")),
    );
    let json = convert(&psbt, &p2wpkh_metadata(&prev)).unwrap();
    let output = field(&json, "Output 1");
    assert!(text(output).contains("WARNING: unusual derivation path"));
    assert_eq!(
        expanded(output, "Derivation Warning"),
        Some(
            "WARNING: unusual derivation path: branch 5 is neither receive (0) nor change (1) (from wallet metadata check)"
        )
    );
    assert_eq!(
        expanded(output, "Ownership"),
        Some(
            "Returning to your account - Savings (m/84'/0'/0'/5/20000) - verified from wallet metadata"
        )
    );
}

#[test]
fn path_outside_the_account_is_unverified() {
    // Our key and fingerprint, but under an account the metadata does not
    // list: the parser cannot regenerate it, so it is not ours.
    let k = ours();
    let (mut psbt, prev) = p2wpkh_spend();
    psbt.unsigned_tx.output[1].script_pubkey = p2wpkh(&k, "m/84'/0'/3'/1/0");
    psbt.outputs[1].bip32_derivation.clear();
    psbt.outputs[1].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/3'/1/0"),
        (fingerprint(), path("m/84'/0'/3'/1/0")),
    );
    let report = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .ownership_report(&psbt, &p2wpkh_metadata(&prev))
        .unwrap();
    assert_eq!(
        report.outputs[1].ownership,
        Ownership::ClaimedOwnUnverified {
            reason: "path m/84'/0'/3'/1/0 is outside account Savings (m/84'/0'/0')".to_string()
        }
    );
}

#[test]
fn hardened_steps_below_the_account_are_unverified() {
    let (mut psbt, prev) = p2wpkh_spend();
    let hardened: DerivationPath =
        path("m/84'/0'/0'").child(ChildNumber::from_hardened_idx(1).unwrap());
    psbt.outputs[1].bip32_derivation.clear();
    psbt.outputs[1]
        .bip32_derivation
        .insert(pubkey(&ours(), "m/84'/0'/0'/1'"), (fingerprint(), hardened));
    let report = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .ownership_report(&psbt, &p2wpkh_metadata(&prev))
        .unwrap();
    assert!(matches!(
        report.outputs[1].ownership,
        Ownership::ClaimedOwnUnverified { .. }
    ));
}

#[test]
fn no_metadata_leaves_the_payload_unchanged() {
    // The fixture golden file predates the metadata support.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let input = fs::read_to_string(dir.join("synthetic-p2wpkh-single-sig.input")).unwrap();
    let expected = fs::read_to_string(dir.join("synthetic-p2wpkh-single-sig.expected")).unwrap();
    let converter = BitcoinVisualSignConverter::new(Network::Bitcoin);
    let from_string = converter
        .to_visual_sign_payload_from_string(input.trim(), VisualSignOptions::default())
        .unwrap()
        .payload
        .to_json()
        .unwrap();
    assert_eq!(from_string.trim(), expected.trim());
    let wrapper = BitcoinTransactionWrapper::from_string(input.trim()).unwrap();
    let direct = converter
        .to_visual_sign_payload(wrapper, VisualSignOptions::default())
        .unwrap()
        .payload
        .to_json()
        .unwrap();
    assert_eq!(direct.trim(), expected.trim());
    assert!(!direct.contains("wallet metadata"));
}

#[test]
fn empty_metadata_still_announces_itself() {
    // Metadata with no accounts classifies everything as external, and the
    // summary still says the context came from wallet metadata.
    let (psbt, _) = p2wpkh_spend();
    let metadata = BitcoinMetadata {
        network: Network::Bitcoin,
        accounts: vec![],
        utxos: vec![],
        output_labels: vec![],
    };
    let json = convert(&psbt, &metadata).unwrap();
    assert_eq!(
        expanded(field(&json, "Summary"), "Ownership Source"),
        Some("wallet metadata supplied with this request, checked against the PSBT")
    );
    assert_eq!(
        expanded(field(&json, "Output 1"), "Ownership"),
        Some("External - not one of your accounts in wallet metadata")
    );
}
