#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `build_wallet_check` over an in-memory wallet, the node `ismine`
//! cross-check, and descriptor parsing.
//!
//! Keys come from the BIP-39 "abandon ... about" seed (our wallet) and a
//! fixed 32-byte seed (a third party), as in `metadata_test.rs`. Run with
//! `--nocapture` to print the renderings.

use bitcoin::bip32::{DerivationPath, Fingerprint, Xpriv, Xpub};
use bitcoin::hashes::Hash;
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use bitcoin::{
    Amount, CompressedPublicKey, Network, NetworkKind, OutPoint, ScriptBuf, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness, absolute, transaction,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_bitcoin::metadata::{AccountPolicy, AccountScriptType, OutputSelector};
use visualsign_bitcoin::wallet_source::{
    InMemorySource, SourceAccounts, WalletCheck, accounts_from_descriptors, build_metadata,
    build_wallet_check, display_text, parse_descriptor,
};
use visualsign_bitcoin::{
    BitcoinTransactionWrapper, BitcoinVisualSignConverter, MetadataSource, NodeCheck, Ownership,
};

const SEED_HEX: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";
const THIRD_PARTY_SEED: [u8; 32] = [7; 32];
const NODE: &str = "your local node (wallet airgap, no chain sync)";

fn master(seed: &[u8]) -> Xpriv {
    Xpriv::new_master(NetworkKind::Main, seed).unwrap()
}

fn ours() -> Xpriv {
    master(&hex::decode(SEED_HEX).unwrap())
}

fn theirs() -> Xpriv {
    master(&THIRD_PARTY_SEED)
}

fn fingerprint(root: &Xpriv) -> Fingerprint {
    root.fingerprint(&Secp256k1::new())
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

/// `wpkh([fp/84'/0'/0']xpub/<branch>/*)`, as `listdescriptors` prints it.
fn descriptor(root: &Xpriv, kind: &str, origin: &str, branch: &str) -> String {
    let secp = Secp256k1::new();
    let xpub = Xpub::from_priv(&secp, &root.derive_priv(&secp, &path(origin)).unwrap());
    let origin = origin.trim_start_matches("m/").replace('\'', "h");
    let fp = fingerprint(root);
    let key = format!("[{fp}/{origin}]{xpub}/{branch}/*");
    match kind {
        "sh-wpkh" => format!("sh(wpkh({key}))#00000000"),
        kind => format!("{kind}({key})#00000000"),
    }
}

fn prev_tx(script: ScriptBuf, value: u64) -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::all_zeros(), 1),
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

/// Savings (BIP-84 account 0) spends 100_000 sat: 60_000 to a third party,
/// 39_000 change to m/84'/0'/0'/1/7. The PSBT carries only `witness_utxo`.
fn spend() -> (Psbt, Transaction) {
    let k = ours();
    let prev = prev_tx(p2wpkh(&k, "m/84'/0'/0'/0/3"), 100_000);
    let tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(prev.compute_txid(), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![
            TxOut {
                value: Amount::from_sat(60_000),
                script_pubkey: p2wpkh(&theirs(), "m/84'/0'/0'/0/0"),
            },
            TxOut {
                value: Amount::from_sat(39_000),
                script_pubkey: p2wpkh(&k, "m/84'/0'/0'/1/7"),
            },
        ],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0].witness_utxo = Some(prev.output[0].clone());
    psbt.inputs[0].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/0'/0/3"),
        (fingerprint(&k), path("m/84'/0'/0'/0/3")),
    );
    psbt.outputs[1].bip32_derivation.insert(
        pubkey(&k, "m/84'/0'/0'/1/7"),
        (fingerprint(&k), path("m/84'/0'/0'/1/7")),
    );
    (psbt, prev)
}

/// Our receive and change descriptors, a third party's account, and two
/// multisig descriptors the prototype cannot use.
fn source(prev: &Transaction) -> InMemorySource {
    let k = ours();
    let descriptors = [
        descriptor(&k, "wpkh", "m/84'/0'/0'", "0"),
        descriptor(&k, "wpkh", "m/84'/0'/0'", "1"),
        descriptor(&theirs(), "wpkh", "m/84'/0'/0'", "<0;1>"),
        "wsh(sortedmulti(2,[d34db33f/48h/0h/0h/2h]xpub1/0/*,[d34db33f/48h/0h/0h/2h]xpub2/0/*))"
            .to_string(),
        "sh(multi(1,02aa,03bb))".to_string(),
    ];
    let mut accounts = accounts_from_descriptors(descriptors.iter().map(String::as_str), "Savings");
    // The third party's account is under another name, to tell them apart.
    accounts.accounts[1].name = "Neighbour".to_string();
    InMemorySource {
        network: Network::Bitcoin,
        description: "wallet airgap, no chain sync".to_string(),
        accounts,
        transactions: vec![prev.clone()],
        owned: [p2wpkh(&k, "m/84'/0'/0'/0/3"), p2wpkh(&k, "m/84'/0'/0'/1/7")]
            .into_iter()
            .collect(),
        labels: [
            (p2wpkh(&k, "m/84'/0'/0'/0/3"), "Deposit #1234".to_string()),
            (
                p2wpkh(&theirs(), "m/84'/0'/0'/0/0"),
                "Acme Corp".to_string(),
            ),
        ]
        .into_iter()
        .collect(),
    }
}

fn convert(psbt: &Psbt, check: &WalletCheck) -> Value {
    let payload = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .to_visual_sign_payload_with_metadata_from(
            BitcoinTransactionWrapper::new(psbt.clone()),
            VisualSignOptions::default(),
            &check.metadata,
            &check.source,
        )
        .unwrap()
        .payload;
    payload.validate_charset().unwrap();
    serde_json::from_str(&payload.to_json().unwrap()).unwrap()
}

fn field<'a>(json: &'a Value, label: &str) -> &'a Value {
    json["Fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["Label"] == label)
        .unwrap_or_else(|| panic!("no field {label}"))
}

fn fields<'a>(json: &'a Value, label: &str) -> Vec<&'a str> {
    json["Fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["Label"] == label)
        .map(|f| f["FallbackText"].as_str().unwrap())
        .collect()
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

fn render_text(json: &Value) -> String {
    let mut out = String::new();
    for f in json["Fields"].as_array().unwrap() {
        out.push_str(&format!("{}: {}\n", f["Label"].as_str().unwrap(), text(f)));
        for k in f["PreviewLayout"]["Expanded"]["Fields"]
            .as_array()
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "    {}: {}\n",
                k["Label"].as_str().unwrap(),
                text(k)
            ));
        }
    }
    out
}

#[test]
fn builds_only_what_the_psbt_needs() {
    let (psbt, prev) = spend();
    let check = build_wallet_check(&psbt, &source(&prev)).unwrap();
    // Receive and change merge into one account; the third party's
    // fingerprint is not named by the PSBT.
    assert_eq!(check.accounts_available, 2);
    assert_eq!(check.metadata.accounts.len(), 1);
    assert_eq!(check.metadata.accounts[0].name, "Savings");
    assert_eq!(check.entries_read, 5);
    assert_eq!(
        check.unsupported,
        BTreeMap::from([("multisig".to_string(), 2)])
    );
    assert_eq!(check.metadata.utxos.len(), 1);
    assert_eq!(
        check.metadata.utxos[0].label.as_deref(),
        Some("Deposit #1234")
    );
    assert!(check.metadata.utxos[0].prev_tx.is_some());
    assert_eq!(check.metadata.output_labels.len(), 1);
    assert_eq!(
        check.metadata.output_labels[0].output,
        OutputSelector::Index(0)
    );
    let MetadataSource::LocalNode(node) = &check.source else {
        panic!("not a local node source");
    };
    assert_eq!(node.inputs_ismine, vec![Some(true)]);
    assert_eq!(node.outputs_ismine, vec![Some(false), Some(true)]);
    assert_eq!(
        build_metadata(&psbt, &source(&prev)).unwrap(),
        check.metadata
    );
}

#[test]
fn node_rows_are_attributed_to_the_node() {
    let (psbt, prev) = spend();
    let check = build_wallet_check(&psbt, &source(&prev)).unwrap();
    let json = convert(&psbt, &check);
    println!("--- local node, honest ---\n{}", render_text(&json));

    let change = field(&json, "Output 1");
    assert_eq!(
        expanded(change, "Ownership"),
        Some(format!("Change - Savings (m/84'/0'/0'/1/7) - verified from {NODE}").as_str())
    );
    assert_eq!(
        expanded(change, "Node Check"),
        Some(format!("ismine=true, agrees - from {NODE}").as_str())
    );
    let payment = field(&json, "Output 0");
    assert_eq!(
        expanded(payment, "Ownership"),
        Some(format!("External - not one of your accounts on {NODE}").as_str())
    );
    assert_eq!(
        expanded(payment, "Label"),
        Some(format!("Acme Corp - label from {NODE}").as_str())
    );
    let input = field(&json, "Input 0");
    assert_eq!(
        expanded(input, "Amount Source"),
        Some(format!("verified against previous transaction from {NODE}").as_str())
    );
    assert_eq!(
        text(field(&json, "Fee Source")),
        format!("previous transactions for input 0 from {NODE}")
    );
    let summary = field(&json, "Summary");
    assert_eq!(
        expanded(summary, "Ownership Source"),
        Some(format!("{NODE}: its public descriptors, re-derived against the PSBT").as_str())
    );
    assert_eq!(
        expanded(summary, "Returning to your accounts"),
        Some(format!("0.00039000 BTC to 1 output - verified from {NODE}").as_str())
    );
    assert!(!json.to_string().contains("WARNING"));
    assert!(!json.to_string().contains("wallet metadata"));
}

#[test]
fn fake_change_gets_the_parser_warning_and_the_node_disagreement() {
    let (mut psbt, prev) = spend();
    psbt.unsigned_tx.output[1].script_pubkey = p2wpkh(&theirs(), "m/84'/0'/0'/1/0");
    let check = build_wallet_check(&psbt, &source(&prev)).unwrap();
    let json = convert(&psbt, &check);
    println!("--- local node, fake change ---\n{}", render_text(&json));

    let output = field(&json, "Output 1");
    assert!(
        expanded(output, "Ownership")
            .unwrap()
            .starts_with("WARNING: claims to be your change")
    );
    assert_eq!(
        expanded(output, "Node Check"),
        Some(
            "WARNING: your local node says this is not yours (ismine=false), but the PSBT claims it is yours; counted as sending"
        )
    );
    assert_eq!(
        fields(&json, "Warning"),
        vec![
            "Output 1 claims to be your change but does not match your account; counted as sending",
            "Output 1: your local node disagrees with the PSBT about ownership (ismine=false); counted as sending",
        ]
    );
    assert_eq!(
        text(field(&json, "Summary")),
        "Sending 0.00099000 BTC | Returning to your accounts 0.00000000 BTC | Fee 0.00001000 BTC"
    );
}

#[test]
fn change_the_node_disowns_is_counted_as_sending() {
    // The script re-derives from the account, but the node's wallet does not
    // know it (e.g. a branch it never imported): trust neither alone.
    let (psbt, prev) = spend();
    let mut source = source(&prev);
    source.owned.remove(&p2wpkh(&ours(), "m/84'/0'/0'/1/7"));
    let check = build_wallet_check(&psbt, &source).unwrap();
    let report = BitcoinVisualSignConverter::new(Network::Bitcoin)
        .ownership_report_from(&psbt, &check.metadata, &check.source)
        .unwrap();
    assert!(matches!(
        report.outputs[1].ownership,
        Ownership::OwnVerified { .. }
    ));
    assert_eq!(
        report.outputs[1].node,
        NodeCheck::Disagrees { ismine: false }
    );
    assert_eq!(report.own_total, Amount::ZERO);
    assert_eq!(report.external_total, Amount::from_sat(99_000));

    let json = convert(&psbt, &check);
    assert_eq!(
        expanded(field(&json, "Summary"), "Sending"),
        Some(
            "0.00099000 BTC to 2 outputs not verified as yours (includes 1 your local node says is not yours)"
        )
    );
    assert_eq!(
        expanded(field(&json, "Output 1"), "Node Check"),
        Some(
            "WARNING: your local node says this is not yours (ismine=false), but it re-derives from your account; counted as sending"
        )
    );
}

#[test]
fn node_claiming_an_unproven_output_is_a_warning() {
    // The node owns the payee address, but the PSBT has no derivation for it.
    let (psbt, prev) = spend();
    let mut source = source(&prev);
    source.owned.insert(p2wpkh(&theirs(), "m/84'/0'/0'/0/0"));
    let check = build_wallet_check(&psbt, &source).unwrap();
    let json = convert(&psbt, &check);
    assert_eq!(
        expanded(field(&json, "Output 0"), "Node Check"),
        Some(
            "WARNING: your local node says this is yours (ismine=true), but the PSBT does not prove it; counted as sending"
        )
    );
    assert!(text(field(&json, "Output 0")).contains("WARNING: your local node disagrees"));
}

#[test]
fn odd_labels_are_made_displayable() {
    assert_eq!(display_text("  "), None);
    assert_eq!(display_text("Caf\u{e9}\n").as_deref(), Some("Caf?"));
    let long = display_text(&"x".repeat(100)).unwrap();
    assert_eq!(long.len(), 64);
    assert!(long.ends_with("..."));
}

#[test]
fn descriptors_are_parsed_narrowly() {
    let k = ours();
    for (kind, origin, script_type) in [
        ("pkh", "m/44'/0'/0'", AccountScriptType::P2pkh),
        ("sh-wpkh", "m/49'/0'/0'", AccountScriptType::P2shP2wpkh),
        ("wpkh", "m/84'/0'/0'", AccountScriptType::P2wpkh),
        ("tr", "m/86'/0'/0'", AccountScriptType::P2tr),
    ] {
        for branch in ["0", "1", "<0;1>"] {
            let (t, key) = parse_descriptor(&descriptor(&k, kind, origin, branch)).unwrap();
            assert_eq!(t, script_type);
            assert_eq!(key.master_fingerprint, fingerprint(&k));
            assert_eq!(key.origin, path(origin));
        }
    }
    let wpkh = descriptor(&k, "wpkh", "m/84'/0'/0'", "0");
    let xpub = wpkh.split(']').nth(1).unwrap().split('/').next().unwrap();
    let fp = fingerprint(&k);
    for (descriptor, reason) in [
        (
            format!("wsh(multi(1,[{fp}/48h/0h/0h/2h]{xpub}/0/*))"),
            "multisig",
        ),
        (
            format!("tr([{fp}/86h/0h/0h]{xpub}/0/*,pk({xpub}/1/*))"),
            "taproot script tree",
        ),
        (format!("wpkh({xpub}/0/*)"), "no key origin"),
        (
            format!("wpkh([{fp}/84h/0h/0h]{xpub}/0/*h)"),
            "hardened wildcard",
        ),
        (format!("wpkh([{fp}/84h/0h/0h]{xpub}/0)"), "not ranged"),
        (format!("wpkh([{fp}/84h/0h/0h]{xpub})"), "not ranged"),
        (
            format!("wpkh([{fp}/84h/0h/0h]{xpub}/2/*)"),
            "unsupported derivation",
        ),
        (
            format!("wpkh([{fp}/84h/0h/0h]{xpub}/0/1/*)"),
            "unsupported derivation",
        ),
        (
            format!("wpkh([{fp}/84h/0h/0h]02{})", "ab".repeat(32)),
            "not an xpub",
        ),
        (
            format!("combo([{fp}/84h/0h/0h]{xpub}/0/*)"),
            "unsupported script type",
        ),
        ("addr(bc1qexample)".to_string(), "unsupported script type"),
    ] {
        assert_eq!(parse_descriptor(&descriptor), Err(reason), "{descriptor}");
    }
}

#[test]
fn receive_and_change_descriptors_merge_per_account() {
    let k = ours();
    let descriptors = [
        descriptor(&k, "wpkh", "m/84'/0'/0'", "0"),
        descriptor(&k, "wpkh", "m/84'/0'/0'", "1"),
        descriptor(&k, "tr", "m/86'/0'/0'", "0"),
        descriptor(&k, "tr", "m/86'/0'/0'", "1"),
        descriptor(&k, "wpkh", "m/84'/0'/1'", "<0;1>"),
    ];
    let SourceAccounts {
        accounts,
        unsupported,
        read,
    } = accounts_from_descriptors(descriptors.iter().map(String::as_str), "Savings");
    assert_eq!(read, 5);
    assert!(unsupported.is_empty());
    let origins: BTreeSet<String> = accounts
        .iter()
        .map(|a| {
            let AccountPolicy::SingleSig { key, .. } = &a.policy;
            format!("m/{}", key.origin)
        })
        .collect();
    assert_eq!(accounts.len(), 3);
    assert_eq!(
        origins,
        ["m/84'/0'/0'", "m/84'/0'/1'", "m/86'/0'/0'"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
}

#[cfg(feature = "json")]
#[test]
fn metadata_json_round_trips() {
    let (psbt, prev) = spend();
    let metadata = build_metadata(&psbt, &source(&prev)).unwrap();
    let json = metadata.to_json();
    println!("--- metadata JSON ---\n{json}");
    assert_eq!(
        visualsign_bitcoin::BitcoinMetadata::from_json(&json).unwrap(),
        metadata
    );
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["accounts"][0]["origin"], "m/84'/0'/0'");
    assert_eq!(value["accounts"][0]["script_type"], "p2wpkh");
    assert!(
        visualsign_bitcoin::BitcoinMetadata::from_json(r#"{"network":"bitcoin","extra":1}"#)
            .unwrap_err()
            .contains("unknown field")
    );
}
