#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Checks a PSBT against an air-gapped watch-only Bitcoin Core wallet.
//!
//! Ignored by default, and needs the `bitcoind-rpc` feature:
//!
//! ```text
//! BITCOIND=/path/to/bitcoind BITCOIN_CLI=/path/to/bitcoin-cli \
//!   cargo test -p visualsign-bitcoin --features bitcoind-rpc --test airgap_regtest \
//!   -- --ignored --nocapture
//! ```
//!
//! Two `bitcoind -regtest` nodes that are never connected. The online one
//! holds alice's descriptor wallet, mines to it and builds a PSBT paying bob
//! with change. The air-gapped one (`-networkactive=0 -listen=0`) has only a
//! blank watch-only wallet with alice's public descriptors imported, and no
//! blocks. The PSBT is checked against the air-gapped node alone, honest and
//! with fake change.

use base64::Engine as _;
use bitcoin::psbt::Psbt;
use bitcoin::{Address, Network};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant};
use visualsign::vsptrait::VisualSignOptions;
use visualsign_bitcoin::bitcoind_rpc::{BitcoindRpc, RpcAuth};
use visualsign_bitcoin::wallet_source::{WalletCheck, build_wallet_check};
use visualsign_bitcoin::{BitcoinTransactionWrapper, BitcoinVisualSignConverter, MetadataSource};

const NODE: &str = "your local node (wallet airgap, no chain sync)";

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
    fn start(name: &str, extra: &[&str]) -> Self {
        let bitcoind = env_path("BITCOIND");
        let cli = env_path("BITCOIN_CLI");
        let datadir = std::env::temp_dir().join(format!(
            "visualsign-bitcoin-airgap-{name}-{}-{}",
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
            .args(["-listen=0", "-server=1"])
            .args(extra)
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
        while node.try_rpc(None, &["getblockchaininfo"]).is_err() {
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

    fn address(&self, wallet: &str) -> String {
        self.rpc(Some(wallet), &["getnewaddress", "", "bech32"])
            .as_str()
            .unwrap()
            .to_string()
    }

    /// This crate's client, as `parser_cli bitcoin wallet-check` builds it.
    fn wallet_source(&self, wallet: &str) -> BitcoindRpc {
        BitcoindRpc::new(
            &format!("http://127.0.0.1:{}", self.rpcport),
            Some(wallet),
            &RpcAuth::Cookie(self.datadir.join("regtest/.cookie")),
            Network::Regtest,
        )
        .unwrap()
        .with_account_name(Some("Savings".to_string()))
        .connect()
        .unwrap()
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

fn decode(psbt: &str) -> Psbt {
    Psbt::deserialize(
        &base64::engine::general_purpose::STANDARD
            .decode(psbt)
            .unwrap(),
    )
    .unwrap()
}

fn convert(psbt: &Psbt, check: &WalletCheck) -> Value {
    let payload = BitcoinVisualSignConverter::new(Network::Regtest)
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

fn has_field(json: &Value, label: &str) -> bool {
    json["Fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["Label"] == label)
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

fn expanded<'a>(field: &'a Value, label: &str) -> &'a str {
    field["PreviewLayout"]["Expanded"]["Fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["Label"] == label)
        .and_then(|f| f["FallbackText"].as_str())
        .unwrap_or_else(|| panic!("no expanded field {label}"))
}

fn render_text(json: &Value) -> String {
    let mut out = String::new();
    for f in json["Fields"].as_array().unwrap() {
        let text = f["FallbackText"].as_str().unwrap();
        out.push_str(&format!("{}: {text}\n", f["Label"].as_str().unwrap()));
        for k in f["PreviewLayout"]["Expanded"]["Fields"]
            .as_array()
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "    {}: {}\n",
                k["Label"].as_str().unwrap(),
                k["FallbackText"].as_str().unwrap()
            ));
        }
    }
    out
}

/// Import `online`'s public descriptors for `wallet` into a new blank
/// watch-only wallet `airgap` on `airgap_node`.
fn import_public_descriptors(online: &Node, wallet: &str, airgap_node: &Node) {
    let listed = online.rpc(Some(wallet), &["listdescriptors", "false"]);
    let requests: Vec<Value> = listed["descriptors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let desc = d["desc"].as_str().unwrap();
            assert!(!desc.contains("prv"), "private key in {desc}");
            json!({
                "desc": desc,
                "timestamp": "now",
                "active": d["active"],
                "internal": d["internal"],
                "range": d["range"],
                "next_index": d.get("next_index").or(d.get("next")).cloned().unwrap_or(json!(0)),
            })
        })
        .collect();
    airgap_node.rpc(
        None,
        &[
            "-named",
            "createwallet",
            "wallet_name=airgap",
            "disable_private_keys=true",
            "blank=true",
        ],
    );
    let result = airgap_node.rpc(
        Some("airgap"),
        &["importdescriptors", &Value::Array(requests).to_string()],
    );
    for r in result.as_array().unwrap() {
        assert_eq!(r["success"], true, "importdescriptors: {result}");
    }
}

#[test]
#[ignore = "needs bitcoind; run with BITCOIND and BITCOIN_CLI set"]
fn airgapped_watch_only_node_checks_ownership() {
    let online = Node::start("online", &["-fallbackfee=0.0002"]);
    let airgap = Node::start("offline", &["-networkactive=0"]);

    online.rpc(None, &["createwallet", "alice"]);
    online.rpc(None, &["createwallet", "bob"]);
    let mining = online.address("alice");
    online.rpc(None, &["generatetoaddress", "101", &mining]);
    let payee = online.address("bob");
    let created = online.rpc(
        Some("alice"),
        &[
            "-named",
            "walletcreatefundedpsbt",
            "inputs=[]",
            &format!("outputs=[{{\"{payee}\":1.0}}]"),
            "options={\"changePosition\":1}",
            "bip32derivs=true",
        ],
    );
    let honest = decode(created["psbt"].as_str().unwrap());

    import_public_descriptors(&online, "alice", &airgap);
    airgap.rpc(Some("airgap"), &["setlabel", &mining, "Mining rewards"]);
    airgap.rpc(Some("airgap"), &["setlabel", &payee, "Bob"]);
    // The air-gapped node never saw a block or a peer.
    assert_eq!(airgap.rpc(None, &["getblockchaininfo"])["blocks"], 0);
    assert_eq!(airgap.rpc(None, &["getconnectioncount"]), 0);

    let source = airgap.wallet_source("airgap");
    let check = build_wallet_check(&honest, &source).unwrap();
    assert_eq!(
        check.entries_read, 8,
        "pkh, sh(wpkh), wpkh, tr; receive and change"
    );
    assert_eq!(check.accounts_available, 4);
    assert!(check.unsupported.is_empty(), "{:?}", check.unsupported);
    assert_eq!(check.metadata.accounts.len(), 4);
    // Prevouts come from the PSBT's non_witness_utxo: the offline wallet has
    // no transactions.
    assert!(check.metadata.utxos.iter().all(|u| u.prev_tx.is_none()));
    assert!(honest.inputs.iter().all(|i| i.non_witness_utxo.is_some()));
    let MetadataSource::LocalNode(node) = &check.source else {
        panic!("not a local node source");
    };
    assert_eq!(node.detail, "wallet airgap, no chain sync");
    assert_eq!(node.outputs_ismine, vec![Some(false), Some(true)]);
    assert!(node.inputs_ismine.iter().all(|m| *m == Some(true)));

    let json = convert(&honest, &check);
    println!("--- air-gapped node, honest ---\n{}", render_text(&json));
    let input = field(&json, "Input 0");
    let ownership = expanded(input, "Ownership");
    assert!(
        ownership.starts_with("From your account - Savings (m/84'/1'/0'/0/")
            && ownership.ends_with(&format!(") - verified from {NODE}")),
        "{ownership}"
    );
    assert_eq!(
        expanded(input, "Node Check"),
        format!("ismine=true, agrees - from {NODE}")
    );
    assert_eq!(
        expanded(input, "Label"),
        format!("Mining rewards - label from {NODE}")
    );
    assert_eq!(
        expanded(input, "Amount Source"),
        "verified against previous transaction"
    );
    assert!(!has_field(&json, "Fee Source"));
    assert_eq!(field(&json, "Fee")["Type"], "amount_v2");

    let change = field(&json, "Output 1");
    let ownership = expanded(change, "Ownership");
    assert!(
        ownership.starts_with("Change - Savings (m/84'/1'/0'/1/")
            && ownership.ends_with(&format!(") - verified from {NODE}")),
        "{ownership}"
    );
    assert_eq!(
        expanded(change, "Node Check"),
        format!("ismine=true, agrees - from {NODE}")
    );
    let payment = field(&json, "Output 0");
    assert_eq!(
        expanded(payment, "Ownership"),
        format!("External - not one of your accounts on {NODE}")
    );
    assert_eq!(
        expanded(payment, "Node Check"),
        format!("ismine=false, agrees - from {NODE}")
    );
    assert_eq!(
        expanded(payment, "Label"),
        format!("Bob - label from {NODE}")
    );
    assert!(!json.to_string().contains("WARNING"));
    assert!(!json.to_string().contains("wallet metadata"));

    // Fake change: keep output 1's derivation, pay bob instead.
    let mut fake = honest.clone();
    let bob_change = online.rpc(Some("bob"), &["getrawchangeaddress", "bech32"]);
    let bob_change = Address::from_str(bob_change.as_str().unwrap())
        .unwrap()
        .require_network(Network::Regtest)
        .unwrap();
    fake.unsigned_tx.output[1].script_pubkey = bob_change.script_pubkey();
    assert!(!fake.outputs[1].bip32_derivation.is_empty());
    let check = build_wallet_check(&fake, &source).unwrap();
    let MetadataSource::LocalNode(node) = &check.source else {
        panic!("not a local node source");
    };
    assert_eq!(node.outputs_ismine, vec![Some(false), Some(false)]);
    let json = convert(&fake, &check);
    println!(
        "--- air-gapped node, fake change ---\n{}",
        render_text(&json)
    );
    assert_eq!(
        fields(&json, "Warning"),
        vec![
            "Output 1 claims to be your change but does not match your account; counted as sending",
            "Output 1: your local node disagrees with the PSBT about ownership (ismine=false); counted as sending",
        ]
    );
    let ownership = expanded(field(&json, "Output 1"), "Ownership");
    assert!(
        ownership.contains("(script is not account Savings at m/84'/1'/0'/1/"),
        "{ownership}"
    );
    assert_eq!(
        expanded(field(&json, "Output 1"), "Node Check"),
        "WARNING: your local node says this is not yours (ismine=false), but the PSBT claims it is yours; counted as sending"
    );
    let report = BitcoinVisualSignConverter::new(Network::Regtest)
        .ownership_report_from(&fake, &check.metadata, &check.source)
        .unwrap();
    let total: bitcoin::Amount = fake.unsigned_tx.output.iter().map(|o| o.value).sum();
    assert_eq!(report.own_total, bitcoin::Amount::ZERO);
    assert_eq!(report.external_total, total);
    assert_eq!(report.external_count, 2);
    assert!(
        expanded(field(&json, "Summary"), "Sending").ends_with(
            "to 2 outputs not verified as yours (includes 1 falsely claiming to be yours)"
        )
    );
}
