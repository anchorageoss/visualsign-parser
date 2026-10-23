#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Validates the PCZT parser against a live Zebra node on regtest.
//!
//! Ignored by default; the `test:zcash-regtest` CI lane (and the README) runs
//! it in release mode, since it creates real Orchard and Sapling proofs:
//!
//! ```text
//! ZEBRAD=/path/to/zebrad ZCASH_PARAMS=/path/to/sapling-params \
//!   cargo test --release -p visualsign-zcash --test regtest -- --ignored --nocapture
//! ```
//!
//! The test starts its own `zebrad` on regtest with NU5 through NU6.2 active
//! from height 1 and NU6.3 (v6 transactions, the Ironwood pool) from
//! `NU6_3_HEIGHT`, mines transparent coinbase to a deterministic key, and
//! drives the PCZT roles a wallet uses (zcash_primitives builder -> Creator ->
//! IO Finalizer -> Updater -> Prover -> Signer -> Spend Finalizer ->
//! Transaction Extractor). For every honest transaction the parser's view of
//! the unsigned PCZT must equal the construction, the transaction must be
//! mined, and the mined transaction's shielded outputs, trial-decrypted with
//! the test viewing keys, must say what the parser showed.
//!
//! The adversarial cases are what a coordinator controlling the PCZT could
//! try. Each must be refused by the parser or flagged by it, and where the
//! signer would still produce a valid transaction, the node shows what the
//! signer would actually have paid.

#[path = "support/builder.rs"]
mod builder;
mod common;

use common::{Decrypted, decrypt_tx, edit_pczt, memo_text, render, script_address};
use getrandom::SysRng;
use incrementalmerkletree::{Hashable, Level};
use orchard::keys::{Scope, SpendAuthorizingKey, SpendingKey};
use orchard::tree::MerkleHashOrchard;
use pczt::roles::{
    creator::Creator, io_finalizer::IoFinalizer, prover::Prover, signer::Signer,
    spend_finalizer::SpendFinalizer, tx_extractor::TransactionExtractor, updater::Updater,
};
use rand_core::UnwrapErr;
use sapling_crypto::keys::PreparedIncomingViewingKey;
use sapling_crypto::zip32::ExtendedSpendingKey;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use visualsign::vsptrait::VisualSignError;
use zcash_keys::address::UnifiedAddress;
use zcash_keys::encoding::AddressCodec;
use zcash_primitives::transaction::Transaction;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::{fixed, zip317};
use zcash_protocol::consensus::{BlockHeight, BranchId, Network, ZIP212_GRACE_PERIOD};
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_transparent::address::TransparentAddress;
use zcash_transparent::bundle::{OutPoint, TxOut};

/// The builder makes PCZTs only once ZIP 212 is fully enforced, which is
/// `ZIP212_GRACE_PERIOD` blocks after Canopy (height 1 here).
const ZIP212_ON: u32 = 1 + ZIP212_GRACE_PERIOD;
/// NU6.3 activates here; everything before it is NU6.2 (v5 transactions).
const NU6_3_HEIGHT: u32 = ZIP212_ON + 100;
/// Coinbase outputs mature after this many blocks.
const COINBASE_MATURITY: u32 = 100;
/// How the parser encodes regtest addresses: a PCZT carries only a coin type,
/// and regtest shares testnet's (1).
const SHOWN_AS: Network = Network::TestNetwork;

fn params() -> LocalNetwork {
    let one = Some(BlockHeight::from_u32(1));
    LocalNetwork {
        overwinter: one,
        sapling: one,
        blossom: one,
        heartwood: one,
        canopy: one,
        nu5: one,
        nu6: one,
        nu6_1: one,
        nu6_2: one,
        nu6_3: Some(BlockHeight::from_u32(NU6_3_HEIGHT)),
        nu7: None,
    }
}

fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
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
        panic!("{name} is not set; this test needs zebrad and the Sapling parameters (see the module docs)")
    });
    PathBuf::from(value)
}

fn zat(v: u64) -> Zatoshis {
    Zatoshis::const_from_u64(v)
}

fn sender_taddr() -> TransparentAddress {
    TransparentAddress::from_pubkey(&builder::sender_tkey().public_key())
}

struct Node {
    rpc_port: u16,
    dir: PathBuf,
    child: Child,
}

impl Node {
    fn start() -> Self {
        let zebrad = env_path("ZEBRAD");
        let dir = std::env::temp_dir().join(format!(
            "visualsign-zcash-regtest-{}-{}",
            std::process::id(),
            free_port()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let rpc_port = free_port();
        let config = format!(
            r#"
[network]
network = "Regtest"
listen_addr = "127.0.0.1:{p2p}"
initial_testnet_peers = []
cache_dir = false

[network.testnet_parameters.activation_heights]
NU5 = 1
NU6 = 1
"NU6.1" = 1
"NU6.2" = 1
"NU6.3" = {NU6_3_HEIGHT}

[state]
ephemeral = true
cache_dir = "{cache}"

[rpc]
listen_addr = "127.0.0.1:{rpc_port}"
enable_cookie_auth = false

[mining]
miner_address = "{miner}"

[tracing]
use_color = false
"#,
            p2p = free_port(),
            cache = dir.join("state").display(),
            miner = sender_taddr().encode(&SHOWN_AS),
        );
        let config_path = dir.join("zebrad.toml");
        std::fs::write(&config_path, config).unwrap();
        let log = std::fs::File::create(dir.join("zebrad.log")).unwrap();
        let child = Command::new(zebrad)
            .arg("-c")
            .arg(&config_path)
            .arg("start")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let node = Self {
            rpc_port,
            dir,
            child,
        };
        let deadline = Instant::now() + Duration::from_secs(120);
        while node.call("getblockcount", json!([])).is_err() {
            assert!(
                Instant::now() < deadline,
                "zebrad did not start; log:\n{}",
                node.log()
            );
            std::thread::sleep(Duration::from_millis(500));
        }
        node
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("zebrad.log")).unwrap_or_default()
    }

    /// One JSON-RPC call over plain HTTP/1.1.
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body =
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        let mut stream =
            TcpStream::connect(("127.0.0.1", self.rpc_port)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(300)))
            .unwrap();
        write!(
            stream,
            "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&raw);
        let (head, body) = text.split_once("\r\n\r\n").ok_or("no HTTP body")?;
        let body = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            dechunk(body)
        } else {
            body.to_string()
        };
        let reply: Value = serde_json::from_str(&body).map_err(|e| format!("{e}: {body}"))?;
        match &reply["error"] {
            Value::Null => Ok(reply["result"].clone()),
            e => Err(e.to_string()),
        }
    }

    fn rpc(&self, method: &str, params: Value) -> Value {
        self.call(method, params.clone())
            .unwrap_or_else(|e| panic!("{method} {params} failed: {e}"))
    }

    fn height(&self) -> u32 {
        self.rpc("getblockcount", json!([])).as_u64().unwrap() as u32
    }

    fn generate(&self, blocks: u32) {
        self.rpc("generate", json!([blocks]));
    }

    fn raw_tx(&self, txid: &str) -> (Vec<u8>, Value) {
        let verbose = self.rpc("getrawtransaction", json!([txid, 1]));
        let raw = hex::decode(verbose["hex"].as_str().unwrap()).unwrap();
        (raw, verbose)
    }

    fn block_txids(&self, height: u32) -> Vec<String> {
        self.rpc("getblock", json!([height.to_string(), 1]))["tx"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect()
    }

    fn send(&self, tx: &Transaction) -> Result<String, String> {
        let mut raw = Vec::new();
        tx.write(&mut raw).unwrap();
        self.call("sendrawtransaction", json!([hex::encode(raw)]))
            .map(|v| v.as_str().unwrap().to_string())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.call("stop", json!([]));
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&tail[..n]);
        rest = &tail[n + 2..];
    }
    out
}

fn txid_hex(tx: &Transaction) -> String {
    let mut id = tx.txid().as_ref().to_vec();
    id.reverse();
    hex::encode(id)
}

/// Authentication path and root for `leaves[pos]` in a depth-32 tree whose
/// other leaves are empty.
fn auth_path<H: Hashable + Clone>(leaves: &[H], pos: usize) -> (Vec<H>, H) {
    let mut level: Vec<H> = leaves.to_vec();
    let mut idx = pos;
    let mut path = Vec::new();
    for l in 0..32u8 {
        let empty = H::empty_root(Level::from(l));
        path.push(level.get(idx ^ 1).cloned().unwrap_or_else(|| empty.clone()));
        level = level
            .chunks(2)
            .map(|c| H::combine(Level::from(l), &c[0], c.get(1).unwrap_or(&empty)))
            .collect();
        idx /= 2;
    }
    let root = level
        .into_iter()
        .next()
        .unwrap_or_else(|| H::empty_root(Level::from(32)));
    (path, root)
}

/// What the test wallet holds, rebuilt by scanning every mined transaction.
#[derive(Default)]
struct Wallet {
    orchard_leaves: Vec<MerkleHashOrchard>,
    sapling_leaves: Vec<sapling_crypto::Node>,
    ironwood_leaves: Vec<MerkleHashOrchard>,
    /// (note, position) for unspent sender Orchard notes.
    orchard_notes: Vec<(orchard::Note, usize)>,
    /// (note, position, internal scope) for unspent sender Sapling notes.
    sapling_notes: Vec<(sapling_crypto::Note, usize, bool)>,
    /// (outpoint, coin, is coinbase, mined at) for sender transparent UTXOs.
    utxos: Vec<(OutPoint, TxOut, bool, u32)>,
    scanned: u32,
}

impl Wallet {
    fn scan(&mut self, node: &Node) {
        let tip = node.height();
        for h in self.scanned + 1..=tip {
            for (i, txid) in node.block_txids(h).iter().enumerate() {
                let (raw, _) = node.raw_tx(txid);
                let branch = BranchId::for_height(&params(), BlockHeight::from_u32(h));
                let tx = Transaction::read(&raw[..], branch).unwrap();
                self.add(&tx, i == 0, h);
            }
        }
        self.scanned = tip;
    }

    fn add(&mut self, tx: &Transaction, coinbase: bool, height: u32) {
        let fvk = builder::sender_orchard();
        let ivks = [fvk.to_ivk(Scope::External), fvk.to_ivk(Scope::Internal)];
        if let Some(b) = tx.orchard_bundle() {
            for (i, a) in b.actions().iter().enumerate() {
                let pos = self.orchard_leaves.len();
                self.orchard_leaves
                    .push(MerkleHashOrchard::from_cmx(a.cmx()));
                if let Some((note, _, _)) =
                    ivks.iter().find_map(|k| b.decrypt_output_with_key(i, k))
                    && note.value().inner() > 0
                {
                    self.orchard_notes.push((note, pos));
                }
            }
        }
        if let Some(b) = tx.ironwood_bundle() {
            for a in b.actions() {
                self.ironwood_leaves
                    .push(MerkleHashOrchard::from_cmx(a.cmx()));
            }
        }
        if let Some(b) = tx.sapling_bundle() {
            let dfvk = builder::sender_sapling();
            let ivks = [
                PreparedIncomingViewingKey::new(&dfvk.to_ivk(Scope::External)),
                PreparedIncomingViewingKey::new(&dfvk.to_ivk(Scope::Internal)),
            ];
            for out in b.shielded_outputs() {
                let pos = self.sapling_leaves.len();
                self.sapling_leaves
                    .push(sapling_crypto::Node::from_cmu(out.cmu()));
                let found = ivks.iter().enumerate().find_map(|(scope, k)| {
                    sapling_crypto::note_encryption::try_sapling_note_decryption(
                        k,
                        out,
                        sapling_crypto::note_encryption::Zip212Enforcement::On,
                    )
                    .map(|(note, _, _)| (note, scope == 1))
                });
                if let Some((note, internal)) = found.filter(|(n, _)| n.value().inner() > 0) {
                    self.sapling_notes.push((note, pos, internal));
                }
            }
        }
        if let Some(b) = tx.transparent_bundle() {
            for (n, out) in b.vout.iter().enumerate() {
                if script_address(&out.script_pubkey().0.0) == Some(sender_taddr()) {
                    let outpoint = OutPoint::new(*tx.txid().as_ref(), n as u32);
                    self.utxos.push((outpoint, out.clone(), coinbase, height));
                }
            }
            for input in &b.vin {
                self.utxos.retain(|(o, ..)| o != input.prevout());
            }
        }
    }

    /// A mature coinbase UTXO, removed from the wallet.
    fn take_coinbase(&mut self, tip: u32) -> (OutPoint, TxOut) {
        let i = self
            .utxos
            .iter()
            .position(|(_, _, cb, h)| *cb && h + COINBASE_MATURITY <= tip)
            .expect("no mature coinbase");
        let (o, c, ..) = self.utxos.remove(i);
        (o, c)
    }

    fn take_utxo(&mut self) -> (OutPoint, TxOut) {
        let i = self
            .utxos
            .iter()
            .position(|(_, _, cb, _)| !cb)
            .expect("no UTXO");
        let (o, c, ..) = self.utxos.remove(i);
        (o, c)
    }
}

/// Where a payment goes.
#[derive(Clone)]
enum Pay {
    Orchard(orchard::Address, u64, &'static str),
    Ironwood(orchard::Address, u64, &'static str),
    Sapling(sapling_crypto::PaymentAddress, u64, &'static str),
    Transparent(TransparentAddress, u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Change {
    Orchard,
    Ironwood,
    Sapling,
    Transparent,
}

/// A transaction to build from the wallet's coins.
#[derive(Clone, Default)]
struct Plan {
    name: &'static str,
    coinbase: bool,
    utxo: bool,
    orchard_note: bool,
    sapling_note: bool,
    pays: Vec<Pay>,
    change: Option<Change>,
    /// Use this fee instead of the ZIP-317 one.
    fixed_fee: Option<u64>,
    orchard_pad_to: Option<u8>,
    /// Claim the transparent input is worth this much less than it is.
    understate_input: u64,
}

/// What a plan spends, chosen from the wallet before building.
struct Coins {
    transparent: Option<(OutPoint, TxOut)>,
    orchard: Option<(orchard::Note, usize)>,
    sapling: Option<(sapling_crypto::Note, usize, bool)>,
}

impl Coins {
    fn take(plan: &Plan, wallet: &mut Wallet, tip: u32) -> Self {
        let transparent = if plan.coinbase {
            Some(wallet.take_coinbase(tip))
        } else if plan.utxo {
            Some(wallet.take_utxo())
        } else {
            None
        };
        let orchard = plan.orchard_note.then(|| wallet.orchard_notes.remove(0));
        let sapling = plan.sapling_note.then(|| wallet.sapling_notes.remove(0));
        Self {
            transparent,
            orchard,
            sapling,
        }
    }

    fn total(&self) -> u64 {
        self.transparent
            .as_ref()
            .map_or(0, |(_, c)| u64::from(c.value()))
            + self.orchard.as_ref().map_or(0, |(n, _)| n.value().inner())
            + self.sapling.as_ref().map_or(0, |(n, ..)| n.value().inner())
    }
}

struct Built {
    pczt: pczt::Pczt,
    fee: u64,
    change: u64,
    orchard_spend: Option<usize>,
    sapling_spend: Option<usize>,
    sapling_internal: bool,
}

/// The sender's Sapling spending key: external, or ZIP 32 internal (change).
fn sapling_key(internal: bool) -> ExtendedSpendingKey {
    let master = ExtendedSpendingKey::master(&[3; 32]).unwrap();
    if internal {
        master.derive_internal().unwrap()
    } else {
        master
    }
}

fn memo(text: &str) -> MemoBytes {
    if text.is_empty() {
        MemoBytes::empty()
    } else {
        MemoBytes::from_bytes(text.as_bytes()).unwrap()
    }
}

fn builder_for(
    plan: &Plan,
    coins: &Coins,
    wallet: &Wallet,
    target: u32,
    change: u64,
) -> Builder<LocalNetwork, ()> {
    let fvk = builder::sender_orchard();
    let dfvk = builder::sender_sapling();
    let (orchard_path, orchard_root) = coins
        .orchard
        .as_ref()
        .map(|(_, pos)| auth_path(&wallet.orchard_leaves, *pos))
        .unwrap_or_else(|| auth_path(&wallet.orchard_leaves, 0));
    let (sapling_path, sapling_root) = coins
        .sapling
        .as_ref()
        .map(|(_, pos, _)| auth_path(&wallet.sapling_leaves, *pos))
        .unwrap_or_else(|| auth_path(&wallet.sapling_leaves, 0));
    let mut b = Builder::new(
        params(),
        BlockHeight::from_u32(target),
        BuildConfig::Standard {
            sapling_anchor: Some(sapling_crypto::Anchor::from(sapling_root)),
            orchard_anchor: Some(orchard::Anchor::from(orchard_root)),
            ironwood_anchor: (target >= NU6_3_HEIGHT)
                .then(|| orchard::Anchor::from(auth_path(&wallet.ironwood_leaves, 0).1)),
            orchard_padding: BundlePadding {
                bundle_required: false,
                pad_to_minimum: plan.orchard_pad_to,
            },
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    if let Some((note, pos)) = &coins.orchard {
        let path =
            orchard::tree::MerklePath::from_parts(*pos as u32, orchard_path.try_into().unwrap());
        b.add_orchard_spend::<zip317::FeeRule>(fvk.clone(), *note, path)
            .unwrap();
    }
    if let Some((note, pos, internal)) = &coins.sapling {
        let path =
            sapling_crypto::MerklePath::from_parts(sapling_path, (*pos as u64).into()).unwrap();
        let fvk = sapling_key(*internal).to_diversifiable_full_viewing_key();
        b.add_sapling_spend::<zip317::FeeRule>(fvk.fvk().clone(), note.clone(), path)
            .unwrap();
    }
    if let Some((outpoint, coin)) = &coins.transparent {
        let claimed = TxOut::new(
            zat(u64::from(coin.value()) - plan.understate_input),
            coin.script_pubkey().clone(),
        );
        b.add_transparent_p2pkh_input(
            builder::sender_tkey().public_key(),
            outpoint.clone(),
            claimed,
        )
        .unwrap();
    }
    let ovk = Some(fvk.to_ovk(Scope::External));
    for pay in &plan.pays {
        match pay {
            Pay::Orchard(to, v, m) => b
                .add_orchard_output::<zip317::FeeRule>(ovk.clone(), *to, zat(*v), memo(m))
                .unwrap(),
            Pay::Ironwood(to, v, m) => b
                .add_ironwood_output::<zip317::FeeRule>(ovk.clone(), *to, zat(*v), memo(m))
                .unwrap(),
            Pay::Sapling(to, v, m) => b
                .add_sapling_output::<zip317::FeeRule>(
                    Some(dfvk.to_ovk(Scope::External)),
                    *to,
                    zat(*v),
                    memo(m),
                )
                .unwrap(),
            Pay::Transparent(to, v) => b.add_transparent_output(to, zat(*v)).unwrap(),
        }
    }
    let internal = fvk.address_at(0u32, Scope::Internal);
    match plan.change {
        Some(Change::Orchard) => b
            .add_orchard_output::<zip317::FeeRule>(
                Some(fvk.to_ovk(Scope::Internal)),
                internal,
                zat(change),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(Change::Ironwood) => b
            .add_ironwood_output::<zip317::FeeRule>(
                Some(fvk.to_ovk(Scope::Internal)),
                internal,
                zat(change),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(Change::Sapling) => b
            .add_sapling_output::<zip317::FeeRule>(
                Some(dfvk.to_ovk(Scope::Internal)),
                dfvk.change_address().1,
                zat(change),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(Change::Transparent) => b
            .add_transparent_output(&sender_taddr(), zat(change))
            .unwrap(),
        None => {}
    }
    b
}

/// The fee `b` pays under the plan's fee rule.
fn fee_for(plan: &Plan, b: &Builder<LocalNetwork, ()>) -> u64 {
    match plan.fixed_fee {
        Some(f) => f,
        None => u64::from(b.get_fee(&zip317::FeeRule::standard()).unwrap()),
    }
}

/// Build the unsigned, unproven PCZT a coordinator would hand the signer.
fn build(plan: &Plan, coins: &Coins, wallet: &Wallet, target: u32) -> Built {
    build_with(plan, coins, wallet, target, |_| {})
}

/// `build`, with `tamper` applied to the PCZT before the IO Finalizer runs,
/// as a malicious coordinator could.
fn build_with(
    plan: &Plan,
    coins: &Coins,
    wallet: &Wallet,
    target: u32,
    tamper: impl FnOnce(&mut Value),
) -> Built {
    let fee = fee_for(plan, &builder_for(plan, coins, wallet, target, 1));
    let paid: u64 = plan
        .pays
        .iter()
        .map(|p| match p {
            Pay::Orchard(_, v, _)
            | Pay::Ironwood(_, v, _)
            | Pay::Sapling(_, v, _)
            | Pay::Transparent(_, v) => *v,
        })
        .sum();
    let change = if plan.change.is_some() {
        coins.total() - plan.understate_input - paid - fee
    } else {
        0
    };
    let b = builder_for(plan, coins, wallet, target, change);
    let result = match plan.fixed_fee {
        Some(f) => b
            .build_for_pczt(rng(), &fixed::FeeRule::non_standard(zat(f)))
            .map_err(|e| format!("{e:?}")),
        None => b
            .build_for_pczt(rng(), &zip317::FeeRule::standard())
            .map_err(|e| format!("{e:?}")),
    };
    let PcztResult {
        pczt_parts,
        sapling_meta,
        orchard_meta,
        ..
    } = result.unwrap_or_else(|e| panic!("{}: build: {e}", plan.name));
    let pczt = Creator::build_from_parts(pczt_parts).unwrap();
    let pczt = pczt::Pczt::parse(&edit_pczt(&pczt.serialize().unwrap(), tamper)).unwrap();
    let pczt = IoFinalizer::new(pczt).finalize_io(rng()).unwrap();
    let sapling_spend = coins
        .sapling
        .as_ref()
        .map(|_| sapling_meta.spend_index(0).unwrap());
    let pczt = match sapling_spend {
        Some(i) => {
            let extsk = sapling_key(coins.sapling.as_ref().unwrap().2);
            let pgk = extsk.expsk().proof_generation_key();
            Updater::new(pczt)
                .update_sapling_with(|mut u| {
                    u.update_spend_with(i, |mut s| s.set_proof_generation_key(pgk))
                })
                .unwrap()
                .finish()
        }
        None => pczt,
    };
    Built {
        pczt,
        fee,
        change,
        orchard_spend: coins
            .orchard
            .as_ref()
            .map(|_| orchard_meta.spend_action_index(0).unwrap()),
        sapling_spend,
        sapling_internal: coins.sapling.as_ref().is_some_and(|s| s.2),
    }
}

struct Provers {
    orchard_v5: orchard::circuit::ProvingKey,
    orchard_v6: orchard::circuit::ProvingKey,
    spend: sapling_crypto::circuit::SpendParameters,
    output: sapling_crypto::circuit::OutputParameters,
}

fn provers() -> &'static Provers {
    static P: OnceLock<Provers> = OnceLock::new();
    P.get_or_init(|| {
        let dir = env_path("ZCASH_PARAMS");
        let read = |f: &str| std::fs::File::open(dir.join(f)).unwrap();
        Provers {
            orchard_v5: orchard::circuit::ProvingKey::build(
                orchard::circuit::OrchardCircuitVersion::FixedPostNu6_2,
            ),
            orchard_v6: orchard::circuit::ProvingKey::build(
                orchard::circuit::OrchardCircuitVersion::PostNu6_3,
            ),
            spend: sapling_crypto::circuit::SpendParameters::read(
                read("sapling-spend.params"),
                false,
            )
            .unwrap(),
            output: sapling_crypto::circuit::OutputParameters::read(
                read("sapling-output.params"),
                false,
            )
            .unwrap(),
        }
    })
}

/// Create every proof the PCZT needs.
fn prove(pczt: pczt::Pczt) -> pczt::Pczt {
    let p = provers();
    let v6 = *pczt.global().tx_version() == 6;
    let pk = if v6 { &p.orchard_v6 } else { &p.orchard_v5 };
    let mut prover = Prover::new(pczt);
    if prover.requires_orchard_proof() {
        prover = prover.create_orchard_proof(rng(), pk).unwrap();
    }
    if prover.requires_ironwood_proof() {
        prover = prover.create_ironwood_proof(rng(), &p.orchard_v6).unwrap();
    }
    if prover.requires_sapling_proofs() {
        prover = prover
            .create_sapling_proofs(rng(), &p.spend, &p.output)
            .unwrap();
    }
    prover.finish()
}

/// Sign the sender's shielded spends, finalize, and extract the transaction.
fn sign_and_extract(pczt: pczt::Pczt, built: &Built) -> Result<Transaction, String> {
    let mut signer = Signer::new(pczt).map_err(|e| format!("{e:?}"))?;
    let ask = SpendAuthorizingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    if let Some(i) = built.orchard_spend {
        signer
            .sign_orchard(rng(), i, &ask)
            .map_err(|e| format!("{e:?}"))?;
    }
    if let Some(i) = built.sapling_spend {
        let extsk = sapling_key(built.sapling_internal);
        signer
            .sign_sapling(rng(), i, extsk.expsk().ask())
            .map_err(|e| format!("{e:?}"))?;
    }
    let pczt = SpendFinalizer::new(signer.finish())
        .finalize_spends()
        .map_err(|e| format!("{e:?}"))?;
    let p = provers();
    TransactionExtractor::new(pczt)
        .with_sapling(&p.spend.verifying_key(), &p.output.verifying_key())
        .extract(rng())
        .map_err(|e| format!("{e:?}"))
}

/// (badge pool, address, zatoshis, memo) rows, compared as multisets.
type Row = (String, String, u64, Option<String>);

fn sorted(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort();
    rows
}

fn shown_orchard(a: orchard::Address) -> String {
    UnifiedAddress::from_receivers(Some(a), None, None, None, None)
        .unwrap()
        .encode(&SHOWN_AS)
}

fn memo_shown(m: &str) -> Option<String> {
    memo_text(memo(m).as_array())
}

/// What the plan pays, from construction alone.
fn plan_rows(plan: &Plan, change: u64) -> Vec<Row> {
    let mut rows: Vec<Row> = plan
        .pays
        .iter()
        .map(|p| match p {
            Pay::Orchard(a, v, m) => ("Orchard".into(), shown_orchard(*a), *v, memo_shown(m)),
            Pay::Ironwood(a, v, m) => ("Ironwood".into(), shown_orchard(*a), *v, memo_shown(m)),
            Pay::Sapling(a, v, m) => ("Sapling".into(), a.encode(&SHOWN_AS), *v, memo_shown(m)),
            Pay::Transparent(a, v) => ("transparent".into(), a.encode(&SHOWN_AS), *v, None),
        })
        .collect();
    let internal = builder::sender_orchard().address_at(0u32, Scope::Internal);
    match plan.change {
        Some(Change::Orchard) => {
            rows.push(("Orchard".into(), shown_orchard(internal), change, None))
        }
        Some(Change::Ironwood) => {
            rows.push(("Ironwood".into(), shown_orchard(internal), change, None))
        }
        Some(Change::Sapling) => rows.push((
            "Sapling".into(),
            builder::sender_sapling()
                .change_address()
                .1
                .encode(&SHOWN_AS),
            change,
            None,
        )),
        Some(Change::Transparent) => rows.push((
            "transparent".into(),
            sender_taddr().encode(&SHOWN_AS),
            change,
            None,
        )),
        None => {}
    }
    rows
}

fn parser_rows(r: &common::Rendered) -> Vec<Row> {
    r.outputs
        .iter()
        .map(|o| {
            (
                o.badge.split(' ').next().unwrap().to_string(),
                o.address.clone(),
                o.zat,
                o.memo.clone(),
            )
        })
        .collect()
}

fn foreign_sapling() -> sapling_crypto::zip32::DiversifiableFullViewingKey {
    ExtendedSpendingKey::master(&[9; 32])
        .unwrap()
        .to_diversifiable_full_viewing_key()
}

/// The mined transaction's outputs as the test viewing keys read them.
fn mined_rows(tx: &Transaction) -> (Vec<Row>, Vec<Option<Decrypted>>) {
    let sender = builder::sender_orchard();
    let s = builder::sender_sapling();
    let decrypted = decrypt_tx(
        tx,
        &[
            sender.to_ivk(Scope::External),
            sender.to_ivk(Scope::Internal),
            builder::foreign_orchard().to_ivk(Scope::External),
        ],
        &[
            sender.to_ovk(Scope::External),
            sender.to_ovk(Scope::Internal),
        ],
        &[
            PreparedIncomingViewingKey::new(&s.to_ivk(Scope::External)),
            PreparedIncomingViewingKey::new(&s.to_ivk(Scope::Internal)),
            PreparedIncomingViewingKey::new(&foreign_sapling().to_ivk(Scope::External)),
        ],
        &[s.to_ovk(Scope::External), s.to_ovk(Scope::Internal)],
    );
    let mut rows = Vec::new();
    for d in decrypted.iter().flatten() {
        if d.value > 0 || memo_text(&d.memo).is_some() {
            let addr = if d.pool == "Sapling" {
                sapling_crypto::PaymentAddress::from_bytes(&d.recipient)
                    .unwrap()
                    .encode(&SHOWN_AS)
            } else {
                shown_orchard(orchard::Address::from_raw_address_bytes(&d.recipient).unwrap())
            };
            rows.push((d.pool.to_string(), addr, d.value, memo_text(&d.memo)));
        }
    }
    if let Some(t) = tx.transparent_bundle() {
        for o in &t.vout {
            let a = script_address(&o.script_pubkey().0.0).unwrap();
            rows.push((
                "transparent".into(),
                a.encode(&SHOWN_AS),
                u64::from(o.value()),
                None,
            ));
        }
    }
    (rows, decrypted)
}

struct Harness {
    node: Node,
    wallet: Wallet,
    /// name -> (PCZT bytes, txid) of each honest transaction, for the report.
    mined: BTreeMap<&'static str, String>,
}

impl Harness {
    fn tip(&self) -> u32 {
        self.node.height()
    }

    fn mine(&mut self, blocks: u32) {
        self.node.generate(blocks);
        self.wallet.scan(&self.node);
    }

    /// Mine up to `height` without scanning: these blocks hold only coinbase
    /// the test does not need.
    fn mine_to_unscanned(&mut self, height: u32) {
        let started = Instant::now();
        while self.tip() < height {
            self.node.generate((height - self.tip()).min(1_000));
        }
        self.wallet.scanned = self.tip();
        println!("mined to {height} in {:?}", started.elapsed());
    }

    /// Build, check the parser against the construction, prove, sign, mine,
    /// and check the mined outputs against the parser.
    fn honest(&mut self, plan: Plan) -> Transaction {
        let target = self.tip() + 1;
        let coins = Coins::take(&plan, &mut self.wallet, target - 1);
        let built = build(&plan, &coins, &self.wallet, target);
        let bytes = built.pczt.clone().serialize().unwrap();
        let r = render(&bytes).unwrap_or_else(|e| panic!("{}: parser refused: {e}", plan.name));
        let expected = sorted(plan_rows(&plan, built.change));
        assert_eq!(
            sorted(parser_rows(&r)),
            expected,
            "{}: parser vs construction",
            plan.name
        );
        assert_eq!(r.fee, built.fee, "{}: fee", plan.name);
        assert_eq!(r.network, "Zcash testnet", "{}", plan.name);
        println!(
            "{}: fee {} ({}), branch {}",
            plan.name, r.fee, r.fee_check, r.branch
        );

        let tx = sign_and_extract_all(
            prove(built.pczt.clone()),
            &built,
            coins.transparent.is_some(),
        )
        .unwrap_or_else(|e| panic!("{}: sign/extract: {e}", plan.name));
        let txid = self
            .node
            .send(&tx)
            .unwrap_or_else(|e| panic!("{}: node refused: {e}", plan.name));
        self.mine(1);
        let (raw, verbose) = self.node.raw_tx(&txid);
        assert_eq!(
            verbose["height"].as_u64(),
            Some(u64::from(self.tip())),
            "{}: not mined",
            plan.name
        );
        let branch = BranchId::for_height(&params(), BlockHeight::from_u32(self.tip()));
        let mined = Transaction::read(&raw[..], branch).unwrap();
        assert_eq!(txid_hex(&mined), txid);
        let (rows, decrypted) = mined_rows(&mined);
        assert_eq!(
            sorted(rows),
            expected,
            "{}: mined outputs vs parser",
            plan.name
        );
        let padding = decrypted
            .iter()
            .filter(|d| {
                d.as_ref()
                    .is_none_or(|d| d.value == 0 && memo_text(&d.memo).is_none())
            })
            .count();
        assert_eq!(r.padding, padding, "{}: padding", plan.name);
        self.mined.insert(plan.name, txid);
        mined
    }
}

/// `sign_and_extract`, signing the single transparent input when there is one.
fn sign_and_extract_all(
    pczt: pczt::Pczt,
    built: &Built,
    transparent: bool,
) -> Result<Transaction, String> {
    let mut signer = Signer::new(pczt).map_err(|e| format!("{e:?}"))?;
    if transparent {
        signer
            .sign_transparent(0, &builder::sender_tkey())
            .map_err(|e| format!("{e:?}"))?;
    }
    let pczt = signer.finish();
    sign_and_extract(pczt, built)
}

fn foreign_orchard() -> orchard::Address {
    builder::foreign_orchard_address()
}

fn sender_orchard() -> orchard::Address {
    builder::sender_orchard().address_at(0u32, Scope::External)
}

#[test]
#[ignore = "needs zebrad and the Sapling parameters; run by the test:zcash-regtest lane"]
fn regtest_pczts_match_parser_and_chain() {
    let node = Node::start();
    let mut h = Harness {
        node,
        wallet: Wallet::default(),
        mined: BTreeMap::new(),
    };
    // Coinbase from the first blocks funds every transaction below; the
    // subsidy halves every few hundred blocks on regtest.
    h.mine(COINBASE_MATURITY + 6);
    h.mine_to_unscanned(ZIP212_ON);

    // v5 (NU6.2): shield coinbase into Orchard and Sapling, then spend both.
    h.honest(Plan {
        name: "v5 coinbase to Orchard",
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 50_000_000, "first payment")],
        change: Some(Change::Orchard),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v5 coinbase to Sapling",
        coinbase: true,
        pays: vec![Pay::Sapling(
            builder::foreign_sapling_address(),
            40_000_000,
            "sapling memo",
        )],
        change: Some(Change::Sapling),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v5 Orchard to transparent, Sapling and Orchard",
        orchard_note: true,
        pays: vec![
            Pay::Transparent(sender_taddr(), 30_000_000),
            Pay::Sapling(builder::foreign_sapling_address(), 20_000_000, ""),
            Pay::Orchard(foreign_orchard(), 10_000_000, "caf\u{e9} \u{2615}"),
        ],
        change: Some(Change::Orchard),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v5 Sapling to Orchard",
        sapling_note: true,
        pays: vec![Pay::Orchard(sender_orchard(), 5_000_000, "self")],
        change: Some(Change::Sapling),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v5 transparent only",
        utxo: true,
        pays: vec![Pay::Transparent(builder::foreign_taddr(), 1_000_000)],
        change: Some(Change::Transparent),
        ..Plan::default()
    });

    adversarial_v5(&mut h);

    // v6 (NU6.3): Ironwood.
    h.mine_to_unscanned(NU6_3_HEIGHT - 1);
    h.honest(Plan {
        name: "v6 Orchard to Ironwood",
        orchard_note: true,
        pays: vec![Pay::Ironwood(foreign_orchard(), 2_000_000, "ironwood")],
        change: Some(Change::Ironwood),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v6 coinbase to Ironwood",
        coinbase: true,
        pays: vec![Pay::Ironwood(foreign_orchard(), 3_000_000, "")],
        change: Some(Change::Ironwood),
        ..Plan::default()
    });
    h.honest(Plan {
        name: "v6 transparent only",
        utxo: true,
        pays: vec![Pay::Transparent(builder::foreign_taddr(), 1_000_000)],
        change: Some(Change::Transparent),
        ..Plan::default()
    });
    println!("mined: {:#?}", h.mined);
}

/// Prove an honest PCZT, then apply `tamper` to its note fields. Each
/// action's `cmx` and ciphertext are first written out explicitly, so the
/// edit changes what the PCZT says but not the transaction it encodes (the
/// v2 encoding can otherwise re-derive them from the note fields).
fn tamper_after_proving(
    h: &mut Harness,
    plan: &Plan,
    tamper: impl FnOnce(&mut Value),
) -> (Vec<u8>, Built) {
    let target = h.tip() + 1;
    let coins = Coins::take(plan, &mut h.wallet, target - 1);
    let built = build(plan, &coins, &h.wallet, target);
    let proven = prove(built.pczt.clone());
    let effects = proven.clone().into_effects().unwrap();
    let actions = effects.orchard_bundle().unwrap().actions();
    let pinned = edit_pczt(&proven.serialize().unwrap(), |v| {
        for (a, effect) in v["orchard"]["actions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(actions)
        {
            a["output"]["cmx"] = json!(effect.cmx().to_bytes());
            let enc = &effect.encrypted_note().enc_ciphertext;
            a["output"]["enc_ciphertext"] = json!({ "Encrypted": enc.as_ref() });
        }
    });
    (edit_pczt(&pinned, tamper), built)
}

fn adversarial_v5(h: &mut Harness) {
    // Fee hiding through a transparent amount: the coordinator understates
    // the coinbase input by 1 ZEC, so the parser shows the small fee the
    // builder computed. ZIP 244 commits the signature to the real amount,
    // so the node must refuse the signed transaction.
    let plan = Plan {
        name: "understated transparent input",
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 10_000_000, "")],
        change: Some(Change::Orchard),
        understate_input: 100_000_000,
        ..Plan::default()
    };
    let target = h.tip() + 1;
    let coins = Coins::take(&plan, &mut h.wallet, target - 1);
    let built = build(&plan, &coins, &h.wallet, target);
    let r = render(&built.pczt.clone().serialize().unwrap()).unwrap();
    assert_eq!(r.fee, built.fee);
    let tx = sign_and_extract_all(prove(built.pczt.clone()), &built, true).unwrap();
    let refused = h
        .node
        .send(&tx)
        .expect_err("node accepted a signature over an understated amount");
    println!(
        "understated transparent input: parser fee {}, node: {refused}",
        r.fee
    );
    // Return the coin: the signed transaction never made it on chain.
    h.wallet.utxos.push((
        coins.transparent.clone().unwrap().0,
        coins.transparent.unwrap().1,
        true,
        1,
    ));

    // Recipient swapped after proving, cmx untouched: the parser refuses.
    // A signer that trusted the field would sign a transaction the node
    // accepts and that pays the original recipient.
    let plan = Plan {
        name: "recipient swapped after proving",
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 7_000_000, "to foreign")],
        change: Some(Change::Orchard),
        ..Plan::default()
    };
    let shown = sender_orchard().to_raw_address_bytes();
    let (tampered, built) = tamper_after_proving(h, &plan, |v| {
        for a in v["orchard"]["actions"].as_array_mut().unwrap() {
            if a["output"]["value"] == 7_000_000 {
                a["output"]["recipient"] = json!(shown.to_vec());
            }
        }
    });
    let err = render(&tampered).expect_err("swapped recipient rendered");
    assert!(err.to_string().contains("note commitment"), "{err}");
    let tx = sign_and_extract_all(pczt::Pczt::parse(&tampered).unwrap(), &built, true).unwrap();
    let txid = h
        .node
        .send(&tx)
        .expect("the swapped field is not part of the transaction");
    h.mine(1);
    let (raw, _) = h.node.raw_tx(&txid);
    let mined = Transaction::read(&raw[..], BranchId::Nu6_2).unwrap();
    let (rows, _) = mined_rows(&mined);
    assert!(
        rows.iter()
            .any(|r| r.1 == shown_orchard(foreign_orchard()) && r.2 == 7_000_000)
    );
    assert!(
        !rows
            .iter()
            .any(|r| r.1 == shown_orchard(sender_orchard()) && r.2 == 7_000_000)
    );
    println!(
        "recipient swapped after proving: parser refused ({err}); signed anyway, node mined {txid} paying the original recipient"
    );

    // Garbage ciphertext on a nonzero output: the parser refuses; the node
    // would accept it and the recipient could never decrypt the note.
    let plan = Plan {
        name: "unrecoverable output",
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 6_000_000, "lost")],
        change: Some(Change::Orchard),
        ..Plan::default()
    };
    // The coordinator writes garbage before the IO Finalizer signs the dummy
    // spends, so the transaction stays consistent.
    let target = h.tip() + 1;
    let coins = Coins::take(&plan, &mut h.wallet, target - 1);
    let built = build_with(&plan, &coins, &h.wallet, target, |v| {
        for a in v["orchard"]["actions"].as_array_mut().unwrap() {
            if a["output"]["value"] == 6_000_000 {
                a["output"]["enc_ciphertext"] = json!({"Encrypted": vec![0u8; 580]});
            }
        }
    });
    let tampered = prove(built.pczt.clone()).serialize().unwrap();
    let err = render(&tampered).expect_err("unrecoverable output rendered");
    assert!(err.to_string().contains("recover"), "{err}");
    let tx = sign_and_extract_all(pczt::Pczt::parse(&tampered).unwrap(), &built, true).unwrap();
    let txid = h
        .node
        .send(&tx)
        .expect("ciphertexts are not consensus-checked");
    h.mine(1);
    let (raw, _) = h.node.raw_tx(&txid);
    let mined = Transaction::read(&raw[..], BranchId::Nu6_2).unwrap();
    let (rows, _) = mined_rows(&mined);
    assert!(
        !rows.iter().any(|r| r.2 == 6_000_000),
        "the lost note decrypted: {rows:?}"
    );
    println!(
        "unrecoverable output: parser refused ({err}); signed anyway, node mined {txid} and no key decrypts the note"
    );

    // Oversized action count at a fee that pays for two actions: flagged.
    let plan = Plan {
        name: "oversized action count",
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 1_000_000, "")],
        change: Some(Change::Orchard),
        orchard_pad_to: Some(64),
        fixed_fee: Some(10_000),
        ..Plan::default()
    };
    let target = h.tip() + 1;
    let coins = Coins::take(&plan, &mut h.wallet, target - 1);
    let built = build(&plan, &coins, &h.wallet, target);
    let bytes = built.pczt.clone().serialize().unwrap();
    let r = render(&bytes).unwrap();
    assert_eq!(r.padding, 62);
    assert!(
        r.fee_check
            .starts_with("below the ZIP-317 conventional fee"),
        "{}",
        r.fee_check
    );
    let tx = sign_and_extract_all(prove(built.pczt.clone()), &built, true).unwrap();
    match h.node.send(&tx) {
        Ok(txid) => println!(
            "oversized action count: flagged by the parser ({}); node accepted {txid}",
            r.fee_check
        ),
        Err(e) => println!(
            "oversized action count: flagged by the parser ({}); node refused: {e}",
            r.fee_check
        ),
    }
    h.mine(1);

    // Wrong network: the same PCZT declared as mainnet. The sighash does not
    // commit to the network, so the parser can only flag it.
    let plan = Plan {
        name: "declared mainnet",
        utxo: false,
        coinbase: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 1_000_000, "")],
        change: Some(Change::Orchard),
        ..Plan::default()
    };
    let target = h.tip() + 1;
    let coins = Coins::take(&plan, &mut h.wallet, target - 1);
    let built = build(&plan, &coins, &h.wallet, target);
    let bytes = built.pczt.serialize().unwrap();
    let mainnet = edit_pczt(&bytes, |v| v["global"]["coin_type"] = json!(133));
    let (honest, relabeled) = (render(&bytes).unwrap(), render(&mainnet).unwrap());
    assert_eq!(honest.network, "Zcash testnet");
    assert_eq!(relabeled.network, "Zcash mainnet");
    assert_eq!(honest.sighash, relabeled.sighash);
    assert!(
        relabeled
            .outputs
            .iter()
            .all(|o| o.address.starts_with("u1"))
    );
    // A testnet user_address on a mainnet-declared PCZT is refused.
    let mixed = edit_pczt(&mainnet, |v| {
        v["orchard"]["actions"][0]["output"]["user_address"] =
            json!(shown_orchard(foreign_orchard()));
    });
    assert!(matches!(
        render(&mixed),
        Err(VisualSignError::ValidationError(_))
    ));
    h.wallet.utxos.push((
        coins.transparent.clone().unwrap().0,
        coins.transparent.unwrap().1,
        true,
        1,
    ));

    // Fee hiding in the shielded pool: spend values redacted and the Orchard
    // value sum lowered by 5 000 zatoshis. The parser shows the smaller spend
    // and fee; no binding signature exists for the edited value balance.
    let plan = Plan {
        name: "shielded value sum lowered",
        orchard_note: true,
        pays: vec![Pay::Orchard(foreign_orchard(), 1_000_000, "")],
        change: Some(Change::Orchard),
        ..Plan::default()
    };
    let target = h.tip() + 1;
    let coins = Coins::take(&plan, &mut h.wallet, target - 1);
    let built = build(&plan, &coins, &h.wallet, target);
    let proven = prove(built.pczt.clone()).serialize().unwrap();
    let honest = render(&proven).unwrap();
    let lowered = edit_pczt(&proven, |v| {
        for a in v["orchard"]["actions"].as_array_mut().unwrap() {
            a["spend"]["value"] = Value::Null;
        }
        let sum = v["orchard"]["value_sum"][0].as_u64().unwrap();
        assert_eq!(v["orchard"]["value_sum"][1], false);
        // value_sum is spends minus outputs, i.e. the Orchard value balance.
        v["orchard"]["value_sum"][0] = json!(sum - 5_000);
    });
    match render(&lowered) {
        Ok(r) => {
            assert_eq!(r.fee + 5_000, honest.fee);
            assert_ne!(r.sighash, honest.sighash);
            let result = sign_and_extract_all(pczt::Pczt::parse(&lowered).unwrap(), &built, false);
            match result {
                Ok(tx) => {
                    let e = h
                        .node
                        .send(&tx)
                        .expect_err("node accepted an edited value balance");
                    println!("shielded value sum lowered: node refused: {e}");
                }
                Err(e) => println!("shielded value sum lowered: extraction refused: {e}"),
            }
        }
        Err(e) => println!("shielded value sum lowered: parser refused: {e}"),
    }
    h.wallet.orchard_notes.insert(0, coins.orchard.unwrap());
}
