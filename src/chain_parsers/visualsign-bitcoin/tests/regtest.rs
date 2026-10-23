#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Validates the PSBT parser against a live Bitcoin Core node on regtest.
//!
//! Ignored by default; the `test:bitcoin-regtest` CI lane (and the README)
//! runs it with:
//!
//! ```text
//! BITCOIND=/path/to/bitcoind BITCOIN_CLI=/path/to/bitcoin-cli \
//!   cargo test -p visualsign-bitcoin --test regtest -- --ignored --nocapture
//! ```
//!
//! The test starts its own `bitcoind -regtest` in a temporary data directory,
//! funds a descriptor wallet with one UTXO of every address type, and has Core
//! build PSBTs with `walletcreatefundedpsbt`. For each PSBT the parser's
//! rendering must match Core's own `decodepsbt` (recipients, amounts, fee,
//! change claims), and the honest PSBTs are then signed, finalized and accepted
//! by `testmempoolaccept`, so the fixtures are real, valid transactions. The
//! adversarial cases tamper with Core-built PSBTs the way an attacker
//! controlling the coordinator could, and check the parser refuses or flags
//! them as #577 describes.

mod common;

use base64::Engine as _;
use bitcoin::psbt::{Psbt, PsbtSighashType};
use bitcoin::{Amount, Network, Transaction};
use common::{Rendered, RenderedFee, render_no_panic};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CHANGE_TEXT: &str = "claims change (unverified by parser)";

struct Node {
    cli: PathBuf,
    datadir: PathBuf,
    rpcport: u16,
    child: Child,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn env_path(name: &str) -> PathBuf {
    let value = std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set; this test needs a Bitcoin Core binary (see the module docs)")
    });
    PathBuf::from(value)
}

impl Node {
    fn start() -> Self {
        let bitcoind = env_path("BITCOIND");
        let cli = env_path("BITCOIN_CLI");
        let datadir = std::env::temp_dir().join(format!(
            "visualsign-bitcoin-regtest-{}-{}",
            std::process::id(),
            free_port()
        ));
        std::fs::create_dir_all(&datadir).unwrap();
        let rpcport = free_port();
        let child = Command::new(bitcoind)
            .arg("-regtest")
            .arg(format!("-datadir={}", datadir.display()))
            .arg(format!("-rpcport={rpcport}"))
            .arg(format!("-port={}", free_port()))
            .args([
                "-listen=0",
                "-server=1",
                "-txindex=1",
                "-fallbackfee=0.0002",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let node = Self {
            cli,
            datadir,
            rpcport,
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if node.try_rpc(None, &["getblockchaininfo"]).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "bitcoind did not start");
            std::thread::sleep(Duration::from_millis(250));
        }
        node
    }

    fn try_rpc(&self, wallet: Option<&str>, args: &[&str]) -> Result<Value, String> {
        let mut cmd = Command::new(&self.cli);
        cmd.arg("-regtest")
            .arg(format!("-datadir={}", self.datadir.display()))
            .arg(format!("-rpcport={}", self.rpcport));
        if let Some(w) = wallet {
            cmd.arg(format!("-rpcwallet={w}"));
        }
        let out = cmd.args(args).output().unwrap();
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let text = String::from_utf8(out.stdout).unwrap();
        let text = text.trim();
        Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
    }

    fn rpc(&self, wallet: Option<&str>, args: &[&str]) -> Value {
        self.try_rpc(wallet, args)
            .unwrap_or_else(|e| panic!("bitcoin-cli {args:?} failed: {e}"))
    }

    fn address(&self, wallet: &str, kind: &str) -> String {
        self.rpc(Some(wallet), &["getnewaddress", "", kind])
            .as_str()
            .unwrap()
            .to_string()
    }

    fn mine(&self, blocks: u32) {
        let to = self.address("alice", "bech32");
        self.rpc(None, &["generatetoaddress", &blocks.to_string(), &to]);
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.try_rpc(None, &["stop"]);
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.datadir);
    }
}

fn sat(btc: &Value) -> u64 {
    let amount = btc.as_f64().unwrap();
    (amount * 100_000_000.0).round() as u64
}

fn decode(psbt: &str) -> Psbt {
    Psbt::deserialize(
        &base64::engine::general_purpose::STANDARD
            .decode(psbt)
            .unwrap(),
    )
    .unwrap()
}

fn encode(psbt: &Psbt) -> String {
    base64::engine::general_purpose::STANDARD.encode(psbt.serialize())
}

/// The parser's rendering must agree with Core's `decodepsbt` of the same PSBT.
fn assert_matches_core(node: &Node, name: &str, psbt: &str) -> Rendered {
    let rendered = render_no_panic(psbt, Network::Regtest)
        .unwrap_or_else(|e| panic!("{name}: parser rejected a Core-built PSBT: {e}"));
    let core = node.rpc(None, &["decodepsbt", psbt]);
    let vout = core["tx"]["vout"].as_array().unwrap();
    assert_eq!(rendered.outputs.len(), vout.len(), "{name}: output count");
    for (i, (got, want)) in rendered.outputs.iter().zip(vout).enumerate() {
        assert_eq!(
            got.amount_sat,
            sat(&want["value"]),
            "{name}: output {i} amount"
        );
        let spk = &want["scriptPubKey"];
        match spk["address"].as_str() {
            Some(addr) => assert_eq!(got.address.as_deref(), Some(addr), "{name}: output {i}"),
            None => assert_eq!(
                got.script_hex.as_deref(),
                spk["hex"].as_str(),
                "{name}: output {i} script"
            ),
        }
        let meta = &core["outputs"][i];
        let derives =
            meta.get("bip32_derivs").is_some() || meta.get("taproot_bip32_derivs").is_some();
        assert_eq!(
            got.claims_change, derives,
            "{name}: output {i} change claim"
        );
    }

    let inputs = core["inputs"].as_array().unwrap();
    assert_eq!(rendered.inputs.len(), inputs.len(), "{name}: input count");
    let mut all_full = true;
    let mut all_taproot = true;
    for (i, (got, want)) in rendered.inputs.iter().zip(inputs).enumerate() {
        let full = want.get("non_witness_utxo").is_some();
        all_full &= full;
        let spk_type = want["witness_utxo"]["scriptPubKey"]["type"]
            .as_str()
            .unwrap_or_default();
        all_taproot &= spk_type == "witness_v1_taproot";
        let expected_source = if full {
            "verified against previous transaction"
        } else if want.get("witness_utxo").is_some() {
            "claimed by witness_utxo only"
        } else {
            "missing"
        };
        assert_eq!(
            got.source, expected_source,
            "{name}: input {i} amount source"
        );
        if let Some(w) = want.get("witness_utxo") {
            assert_eq!(
                got.amount_sat,
                Some(sat(&w["amount"])),
                "{name}: input {i} amount"
            );
            assert_eq!(
                got.address.as_deref(),
                w["scriptPubKey"]["address"].as_str(),
                "{name}: input {i} address"
            );
        }
    }

    let core_fee = core["fee"].as_f64().map(|_| sat(&core["fee"]));
    assert_eq!(rendered.fee.sat(), core_fee, "{name}: fee");
    let sighash_ok = rendered.inputs.iter().all(|i| i.sighash.is_none());
    let expect_verified = sighash_ok && (all_full || all_taproot);
    assert_eq!(
        matches!(rendered.fee, RenderedFee::Verified(_)),
        expect_verified,
        "{name}: fee verification ({:?})",
        rendered.fee
    );
    rendered
}

struct Funded {
    txid: String,
    vout: u64,
}

/// Fund one new alice address per entry of `kinds` (address type, BTC) in a
/// single `sendmany`, then mine it, so no later wallet spend can touch these
/// UTXOs (the PSBTs below are signed but never broadcast).
fn fund_all(node: &Node, kinds: &[(&str, f64)]) -> Vec<Funded> {
    let addrs: Vec<String> = kinds
        .iter()
        .map(|(k, _)| node.address("alice", k))
        .collect();
    let amounts: serde_json::Map<String, Value> = addrs
        .iter()
        .zip(kinds)
        .map(|(a, (_, btc))| (a.clone(), json!(btc)))
        .collect();
    let txid = node.rpc(
        Some("alice"),
        &["sendmany", "", &Value::Object(amounts).to_string()],
    );
    let txid = txid.as_str().unwrap().to_string();
    node.mine(1);
    let tx = node.rpc(None, &["getrawtransaction", &txid, "true"]);
    addrs
        .iter()
        .map(|addr| {
            let vout = tx["vout"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["scriptPubKey"]["address"] == addr.as_str())
                .unwrap()["n"]
                .as_u64()
                .unwrap();
            Funded {
                txid: txid.clone(),
                vout,
            }
        })
        .collect()
}

fn create_psbt(node: &Node, inputs: &[&Funded], outputs: &Value) -> String {
    let inputs: Vec<Value> = inputs
        .iter()
        .map(|f| json!({"txid": f.txid, "vout": f.vout}))
        .collect();
    let res = node.rpc(
        Some("alice"),
        &[
            "-named",
            "walletcreatefundedpsbt",
            &format!("inputs={}", Value::Array(inputs)),
            &format!("outputs={outputs}"),
            "options={\"add_inputs\":false}",
            "bip32derivs=true",
        ],
    );
    res["psbt"].as_str().unwrap().to_string()
}

/// Sign with Core, finalize, and require Core to accept the result.
fn sign_and_accept(node: &Node, name: &str, psbt: &str, sighash: &str) -> String {
    let signed = node.rpc(
        Some("alice"),
        &["walletprocesspsbt", psbt, "true", sighash, "true", "false"],
    );
    let signed_psbt = signed["psbt"].as_str().unwrap().to_string();
    let fin = node.rpc(None, &["finalizepsbt", &signed_psbt]);
    assert_eq!(fin["complete"], true, "{name}: Core could not finalize");
    let hex = fin["hex"].as_str().unwrap();
    let accept = node.rpc(None, &["testmempoolaccept", &format!("[\"{hex}\"]")]);
    if accept[0]["allowed"] != true {
        let tx = node.rpc(None, &["decoderawtransaction", hex]);
        for vin in tx["vin"].as_array().unwrap() {
            let txid = vin["txid"].as_str().unwrap();
            let vout = vin["vout"].to_string();
            eprintln!(
                "{name}: vin {txid}:{vout} gettxout={}",
                node.rpc(None, &["gettxout", txid, &vout])
            );
        }
    }
    assert_eq!(
        accept[0]["allowed"], true,
        "{name}: Core rejected the transaction: {accept}"
    );
    signed_psbt
}

#[test]
#[ignore = "needs bitcoind; run by the test:bitcoin-regtest lane"]
fn regtest_core_psbts_match_parser() {
    let node = Node::start();
    node.rpc(None, &["createwallet", "alice"]);
    node.rpc(None, &["createwallet", "bob"]);
    node.mine(101);

    let kinds = ["legacy", "p2sh-segwit", "bech32", "bech32m"];
    let mut funded = fund_all(
        &node,
        &[
            ("legacy", 2.0),
            ("p2sh-segwit", 2.0),
            ("bech32", 2.0),
            ("bech32m", 2.0),
            ("bech32", 1.0),
            ("bech32m", 1.0),
        ],
    );
    let extra_tr = funded.pop().unwrap();
    let extra_v0 = funded.pop().unwrap();
    let utxos = funded;

    // Recipients: one per bob address type, a P2WSH 1-of-1 multisig, and an
    // OP_RETURN data output. walletcreatefundedpsbt adds alice's change.
    let bob_pubkey = node.rpc(
        Some("bob"),
        &["getaddressinfo", &node.address("bob", "bech32")],
    )["pubkey"]
        .as_str()
        .unwrap()
        .to_string();
    let p2wsh = node.rpc(
        None,
        &[
            "createmultisig",
            "1",
            &format!("[\"{bob_pubkey}\"]"),
            "bech32",
        ],
    )["address"]
        .as_str()
        .unwrap()
        .to_string();
    let outputs = json!([
        {node.address("bob", "legacy"): 0.1},
        {node.address("bob", "p2sh-segwit"): 0.05},
        {node.address("bob", "bech32"): 0.2},
        {node.address("bob", "bech32m"): 0.3},
        {p2wsh: 0.15},
        {"data": "76697375616c7369676e"}
    ]);

    let mut cases: Vec<(String, Vec<&Funded>)> = kinds
        .iter()
        .zip(&utxos)
        .map(|(k, u)| (format!("spend {k}"), vec![u]))
        .collect();
    cases.push(("spend bech32 + bech32m".into(), vec![&utxos[2], &extra_tr]));
    cases.push(("spend two bech32".into(), vec![&utxos[2], &extra_v0]));
    cases.push(("spend all four types".into(), utxos.iter().collect()));

    for (name, inputs) in &cases {
        let psbt = create_psbt(&node, inputs, &outputs);
        let rendered = assert_matches_core(&node, name, &psbt);
        assert!(
            rendered.outputs.iter().any(|o| o.claims_change),
            "{name}: Core's change output is not marked as claimed change"
        );
        let signed = sign_and_accept(&node, name, &psbt, "ALL");
        assert_matches_core(&node, &format!("{name} (signed)"), &signed);
        eprintln!(
            "ok: {name}: {} inputs, fee {:?}",
            inputs.len(),
            rendered.fee
        );
    }

    adversarial(&node, &outputs, &utxos, &extra_v0);
}

fn adversarial(node: &Node, outputs: &Value, utxos: &[Funded], extra_v0: &Funded) {
    let base = create_psbt(node, &[&utxos[2]], outputs);
    let honest = render_no_panic(&base, Network::Regtest).unwrap();

    // Fee hiding (CVE-2020-14199 shape): drop the previous transaction and lie
    // about the input amount in witness_utxo. Must not be shown as verified.
    let mut p = decode(&base);
    p.inputs[0].non_witness_utxo = None;
    let utxo = p.inputs[0].witness_utxo.as_mut().unwrap();
    utxo.value = utxo.value.checked_add(Amount::from_sat(5_000_000)).unwrap();
    let r = render_no_panic(&encode(&p), Network::Regtest).unwrap();
    assert_eq!(r.inputs[0].source, "claimed by witness_utxo only");
    assert!(
        matches!(r.fee, RenderedFee::Unverified { .. }),
        "lying witness_utxo produced a verified fee: {:?}",
        r.fee
    );
    assert_ne!(
        r.fee.sat(),
        honest.fee.sat(),
        "tampered amount not reflected"
    );
    eprintln!("ok: lying witness_utxo -> {:?}", r.fee);

    // non_witness_utxo for a different transaction: must be refused.
    let mut p = decode(&base);
    // Any other transaction will do; take the coinbase of the tip block.
    let tip = node.rpc(None, &["getbestblockhash"]);
    let block = node.rpc(None, &["getblock", tip.as_str().unwrap(), "1"]);
    let coinbase = block["tx"][0].as_str().unwrap().to_string();
    assert_ne!(coinbase, extra_v0.txid);
    let other = node.rpc(None, &["getrawtransaction", &coinbase]);
    let other: Transaction =
        bitcoin::consensus::encode::deserialize_hex(other.as_str().unwrap()).unwrap();
    p.inputs[0].non_witness_utxo = Some(other);
    let err = render_no_panic(&encode(&p), Network::Regtest).unwrap_err();
    assert!(
        err.contains("non_witness_utxo does not match the prevout txid"),
        "{err}"
    );
    eprintln!("ok: mismatched non_witness_utxo refused");

    // witness_utxo disagreeing with the real previous transaction: refused.
    let mut p = decode(&base);
    let utxo = p.inputs[0].witness_utxo.as_mut().unwrap();
    utxo.value = utxo.value.checked_add(Amount::from_sat(1)).unwrap();
    let err = render_no_panic(&encode(&p), Network::Regtest).unwrap_err();
    assert!(
        err.contains("witness_utxo disagrees with non_witness_utxo"),
        "{err}"
    );
    eprintln!("ok: witness_utxo/non_witness_utxo disagreement refused");

    // Fake change: put alice's change derivation on bob's recipient output.
    // The parser cannot verify ownership, so it must show the claim as
    // unverified, and only as a claim.
    let mut p = decode(&base);
    let change = p
        .outputs
        .iter()
        .position(|o| !o.bip32_derivation.is_empty() || !o.tap_key_origins.is_empty())
        .unwrap();
    let recipient = (0..p.outputs.len())
        .find(|&i| i != change && p.unsigned_tx.output[i].script_pubkey.is_p2tr())
        .unwrap();
    assert!(!render_no_panic(&base, Network::Regtest).unwrap().outputs[recipient].claims_change);
    p.outputs[recipient].bip32_derivation = p.outputs[change].bip32_derivation.clone();
    p.outputs[recipient].tap_key_origins = p.outputs[change].tap_key_origins.clone();
    p.outputs[recipient].tap_internal_key = p.outputs[change].tap_internal_key;
    let tampered = encode(&p);
    let r = render_no_panic(&tampered, Network::Regtest).unwrap();
    assert!(
        r.outputs[recipient].claims_change,
        "fake change claim not shown"
    );
    let payload =
        visualsign::vsptrait::VisualSignConverterFromString::to_visual_sign_payload_from_string(
            &visualsign_bitcoin::BitcoinVisualSignConverter::new(Network::Regtest),
            &tampered,
            visualsign::vsptrait::VisualSignOptions::default(),
        )
        .unwrap()
        .payload
        .to_json()
        .unwrap();
    assert!(payload.contains(CHANGE_TEXT));
    assert!(!payload.to_lowercase().contains("verified change"));
    eprintln!("ok: fake change shown as an unverified claim");

    // Taproot-only spend with a lying witness_utxo amount. #577 shows this fee
    // as verified because a BIP-341 ALL/DEFAULT signature commits to every
    // input amount: a signature made over the wrong amount is invalid, so the
    // lie cannot produce a spendable transaction. Check that claim on-chain.
    let tr = create_psbt(node, &[&utxos[3]], outputs);
    let honest_tr = render_no_panic(&tr, Network::Regtest).unwrap();
    let mut p = decode(&tr);
    p.inputs[0].non_witness_utxo = None;
    let utxo = p.inputs[0].witness_utxo.as_mut().unwrap();
    utxo.value = utxo.value.checked_sub(Amount::from_sat(3_000)).unwrap();
    let lied = encode(&p);
    let r = render_no_panic(&lied, Network::Regtest).unwrap();
    assert_eq!(r.fee.sat(), honest_tr.fee.sat().map(|f| f - 3_000));
    assert!(
        matches!(r.fee, RenderedFee::Verified(_)),
        "taproot-only fee: {:?}",
        r.fee
    );
    // The fee above is displayed as verified but is wrong. It is still safe to
    // sign: the BIP-341 sighash commits to every input amount, so a signature
    // over the lied amount cannot verify against the real prevout. Check that
    // the two sighashes differ (Core repairs witness_utxo from its wallet when
    // signing, so it cannot be used to sign the lie).
    let real_prevouts: Vec<bitcoin::TxOut> = decode(&tr)
        .inputs
        .iter()
        .map(|i| i.witness_utxo.clone().unwrap())
        .collect();
    let lied_prevouts: Vec<bitcoin::TxOut> = p
        .inputs
        .iter()
        .map(|i| i.witness_utxo.clone().unwrap())
        .collect();
    // A fresh cache per sighash: SighashCache memoizes the hash of the prevout
    // amounts, so reusing one would hide the difference being checked.
    let real = bitcoin::sighash::SighashCache::new(&p.unsigned_tx)
        .taproot_key_spend_signature_hash(
            0,
            &bitcoin::sighash::Prevouts::All(&real_prevouts),
            bitcoin::TapSighashType::Default,
        )
        .unwrap();
    let lied_hash = bitcoin::sighash::SighashCache::new(&p.unsigned_tx)
        .taproot_key_spend_signature_hash(
            0,
            &bitcoin::sighash::Prevouts::All(&lied_prevouts),
            bitcoin::TapSighashType::Default,
        )
        .unwrap();
    assert_ne!(
        real, lied_hash,
        "BIP-341 sighash does not commit to the input amount"
    );
    eprintln!(
        "ok: taproot lying witness_utxo: fee shown verified but sighash binds the real amount"
    );

    // Non-default sighash, signed by Core: the fee no longer binds all inputs
    // and outputs, so it must be flagged.
    for (kind, utxo, sighash) in [
        ("bech32", extra_v0, "SINGLE|ANYONECANPAY"),
        ("bech32m", &utxos[3], "NONE"),
    ] {
        let psbt = create_psbt(node, &[utxo], outputs);
        let signed = node.rpc(
            Some("alice"),
            &["walletprocesspsbt", &psbt, "true", sighash, "true", "false"],
        );
        let signed = signed["psbt"].as_str().unwrap();
        let p = decode(signed);
        assert!(
            p.inputs[0]
                .sighash_type
                .is_some_and(|t| t != PsbtSighashType::from_u32(1)),
            "Core did not record sighash {sighash}"
        );
        let r = render_no_panic(signed, Network::Regtest).unwrap();
        assert!(
            r.inputs[0].sighash.is_some(),
            "{kind} {sighash}: no sighash warning"
        );
        assert!(
            matches!(r.fee, RenderedFee::Unverified { .. }),
            "{kind} {sighash}: fee shown as verified: {:?}",
            r.fee
        );
        eprintln!("ok: {kind} {sighash} -> {:?}", r.inputs[0].sighash);
    }
}
