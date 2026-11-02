//! Build [`BitcoinMetadata`] for one PSBT from a wallet the signer controls
//! (prototype).
//!
//! A [`WalletSource`] is anything that knows the signer's accounts, the
//! previous transactions it has seen, and its address labels: a watch-only
//! Bitcoin Core wallet ([`crate::bitcoind_rpc`]), or [`InMemorySource`] in
//! tests. [`build_wallet_check`] asks it only about this PSBT:
//!
//! - the accounts whose master fingerprints the PSBT's `bip32_derivation` /
//!   `tap_key_origins` name;
//! - the previous transactions it has for the outpoints spent;
//! - the labels of the input and output addresses.
//!
//! It also records the source's own `ismine` answer for every input and output
//! address. The parser still re-derives every script from the account xpubs;
//! `ismine` is a second, independent opinion that is shown next to it, and a
//! disagreement is a warning ([`crate::NodeCheck`]).
//!
//! Descriptors are parsed narrowly ([`accounts_from_descriptors`]): single-key
//! `pkh`, `sh(wpkh)`, `wpkh` and key-path-only `tr`, with a key origin and a
//! `/0/*`, `/1/*` or `/<0;1>/*` range. Anything else is skipped and counted.

use crate::metadata::{
    Account, AccountKey, AccountPolicy, AccountScriptType, BitcoinMetadata, LocalNode,
    MAX_LABEL_LEN, MetadataSource, OutputLabel, OutputSelector, UtxoMetadata,
};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use bitcoin::bip32::{DerivationPath, Fingerprint, Xpub};
use bitcoin::psbt::Psbt;
use bitcoin::{Network, Script, ScriptBuf, Transaction, Txid};

/// A failure reading from a wallet source.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct WalletSourceError(pub String);

/// The accounts a source can offer, and what it had to skip.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct SourceAccounts {
    pub accounts: Vec<Account>,
    /// Entries that could not be used, by reason, e.g. `{"multisig": 2}`.
    pub unsupported: BTreeMap<String, usize>,
    /// Entries read, used or not.
    pub read: usize,
}

/// What a source knows about one address.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct AddressInfo {
    /// The source's own ownership answer (Core: `getaddressinfo(..).ismine`).
    pub ismine: Option<bool>,
    pub label: Option<String>,
}

/// A wallet the signer controls.
pub trait WalletSource {
    /// Network the wallet is on.
    fn network(&self) -> Network;
    /// Shown after "your local node", e.g. "wallet airgap, no chain sync".
    fn description(&self) -> String;
    /// Every account the wallet has.
    fn accounts(&self) -> Result<SourceAccounts, WalletSourceError>;
    /// The transaction with `txid`, if the wallet has it.
    fn transaction(&self, txid: Txid) -> Result<Option<Transaction>, WalletSourceError>;
    /// What the wallet knows about the address paying to `script`.
    fn address_info(&self, script: &Script) -> Result<AddressInfo, WalletSourceError>;
}

/// Metadata for one PSBT, with its provenance.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WalletCheck {
    pub metadata: BitcoinMetadata,
    /// [`MetadataSource::LocalNode`] with the node's `ismine` answers.
    pub source: MetadataSource,
    /// Accounts the source has, before selecting the ones this PSBT names.
    pub accounts_available: usize,
    /// Account entries the source skipped, by reason.
    pub unsupported: BTreeMap<String, usize>,
    /// Account entries the source read.
    pub entries_read: usize,
}

/// The metadata `source` has for `psbt`. See [`build_wallet_check`].
pub fn build_metadata(
    psbt: &Psbt,
    source: &dyn WalletSource,
) -> Result<BitcoinMetadata, WalletSourceError> {
    build_wallet_check(psbt, source).map(|check| check.metadata)
}

/// Ask `source` about `psbt` only: the accounts its derivations name, the
/// previous transactions it spends, the labels and `ismine` of its addresses.
pub fn build_wallet_check(
    psbt: &Psbt,
    source: &dyn WalletSource,
) -> Result<WalletCheck, WalletSourceError> {
    let network = source.network();
    let tx = &psbt.unsigned_tx;

    let named: BTreeSet<Fingerprint> = psbt
        .inputs
        .iter()
        .flat_map(|i| {
            i.bip32_derivation
                .values()
                .chain(i.tap_key_origins.values().map(|(_, s)| s))
        })
        .chain(psbt.outputs.iter().flat_map(|o| {
            o.bip32_derivation
                .values()
                .chain(o.tap_key_origins.values().map(|(_, s)| s))
        }))
        .map(|(fingerprint, _)| *fingerprint)
        .collect();
    let all = source.accounts()?;
    let accounts_available = all.accounts.len();
    let accounts = all
        .accounts
        .into_iter()
        .filter(|a| {
            let AccountPolicy::SingleSig { key, .. } = &a.policy;
            named.contains(&key.master_fingerprint)
        })
        .collect();

    let mut cache: BTreeMap<ScriptBuf, AddressInfo> = BTreeMap::new();
    let mut lookup = |script: &Script| -> Result<AddressInfo, WalletSourceError> {
        if let Some(info) = cache.get(script) {
            return Ok(info.clone());
        }
        let info = source.address_info(script)?;
        cache.insert(script.to_owned(), info.clone());
        Ok(info)
    };

    let mut utxos = Vec::new();
    let mut inputs_ismine = Vec::with_capacity(tx.input.len());
    let mut seen = BTreeSet::new();
    for (i, txin) in tx.input.iter().enumerate() {
        let outpoint = txin.previous_output;
        let prev = source.transaction(outpoint.txid)?;
        // The PSBT's script first (analysis checks it against the txid), then
        // the source's transaction.
        let script = crate::metadata::prevout_script(psbt, i)
            .map(Script::to_owned)
            .or_else(|| {
                let vout = usize::try_from(outpoint.vout).ok()?;
                prev.as_ref()?
                    .output
                    .get(vout)
                    .map(|o| o.script_pubkey.clone())
            });
        let info = match &script {
            Some(script) => lookup(script)?,
            None => AddressInfo::default(),
        };
        inputs_ismine.push(info.ismine);
        let label = info.label.as_deref().and_then(display_text);
        if (label.is_some() || prev.is_some()) && seen.insert(outpoint) {
            utxos.push(UtxoMetadata {
                outpoint,
                label,
                prev_tx: prev.as_ref().map(bitcoin::consensus::serialize),
            });
        }
    }

    let mut output_labels = Vec::new();
    let mut outputs_ismine = Vec::with_capacity(tx.output.len());
    for (i, txout) in tx.output.iter().enumerate() {
        let info = lookup(&txout.script_pubkey)?;
        outputs_ismine.push(info.ismine);
        if let (Some(label), Ok(index)) = (
            info.label.as_deref().and_then(display_text),
            u32::try_from(i),
        ) {
            output_labels.push(OutputLabel {
                output: OutputSelector::Index(index),
                label,
            });
        }
    }

    let detail = display_text(&source.description()).unwrap_or_else(|| "unnamed".to_string());
    Ok(WalletCheck {
        metadata: BitcoinMetadata {
            network,
            accounts,
            utxos,
            output_labels,
        },
        source: MetadataSource::LocalNode(LocalNode {
            detail,
            inputs_ismine,
            outputs_ismine,
        }),
        accounts_available,
        unsupported: all.unsupported,
        entries_read: all.read,
    })
}

/// Make wallet text displayable: printable ASCII, at most [`MAX_LABEL_LEN`]
/// bytes. Labels are free text in Core, and one odd label should not fail the
/// whole check. `None` for empty text.
pub fn display_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut out: String = text
        .chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .collect();
    if out.len() > MAX_LABEL_LEN {
        out.truncate(MAX_LABEL_LEN - 3);
        out.push_str("...");
    }
    Some(out)
}

/// Turn output descriptors into accounts named `name`. The receive and change
/// descriptors of one account become one [`Account`]; unsupported
/// descriptors are counted by reason.
pub fn accounts_from_descriptors<'a>(
    descriptors: impl IntoIterator<Item = &'a str>,
    name: &str,
) -> SourceAccounts {
    let mut out = SourceAccounts::default();
    for descriptor in descriptors {
        out.read += 1;
        match parse_descriptor(descriptor) {
            Ok((script_type, key)) => {
                let account = Account {
                    name: name.to_string(),
                    policy: AccountPolicy::SingleSig { key, script_type },
                    descriptor: None,
                };
                if !out.accounts.contains(&account) {
                    out.accounts.push(account);
                }
            }
            Err(reason) => *out.unsupported.entry(reason.to_string()).or_default() += 1,
        }
    }
    out
}

/// Parse a single-key descriptor, or say why it is not supported. The
/// checksum is not verified: the descriptor comes from the signer's own node.
pub fn parse_descriptor(descriptor: &str) -> Result<(AccountScriptType, AccountKey), &'static str> {
    let body = descriptor
        .split_once('#')
        .map_or(descriptor, |(body, _)| body)
        .trim();
    if body.contains("multi(") || body.contains("multi_a(") {
        return Err("multisig");
    }
    let (script_type, inner) = [
        ("sh(wpkh(", "))", AccountScriptType::P2shP2wpkh),
        ("wpkh(", ")", AccountScriptType::P2wpkh),
        ("pkh(", ")", AccountScriptType::P2pkh),
        ("tr(", ")", AccountScriptType::P2tr),
    ]
    .into_iter()
    .find_map(|(open, close, t)| Some((t, body.strip_prefix(open)?.strip_suffix(close)?)))
    .ok_or("unsupported script type")?;
    if script_type == AccountScriptType::P2tr && inner.contains(',') {
        return Err("taproot script tree");
    }
    let (origin, key) = inner
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .ok_or("no key origin")?;
    let (fingerprint, origin_path) = origin.split_once('/').unwrap_or((origin, ""));
    let master_fingerprint = Fingerprint::from_str(fingerprint).map_err(|_| "bad key origin")?;
    let origin = DerivationPath::from_str(origin_path).map_err(|_| "bad key origin")?;
    let mut steps = key.split('/');
    let xpub = steps
        .next()
        .and_then(|k| Xpub::from_str(k).ok())
        .ok_or("not an xpub")?;
    match (steps.next(), steps.next(), steps.next()) {
        (Some("0" | "1" | "<0;1>"), Some("*"), None) => {}
        (_, Some("*'" | "*h"), None) => return Err("hardened wildcard"),
        (None, ..) | (Some(_), None, None) => return Err("not ranged"),
        _ => return Err("unsupported derivation"),
    }
    Ok((
        script_type,
        AccountKey {
            master_fingerprint,
            origin,
            xpub,
        },
    ))
}

/// A wallet held in memory, for tests and for producers that already have the
/// data at hand.
#[derive(Debug, Clone)]
pub struct InMemorySource {
    pub network: Network,
    pub description: String,
    pub accounts: SourceAccounts,
    pub transactions: Vec<Transaction>,
    /// Scripts the wallet owns (`ismine = true`). Every other script with an
    /// address answers `ismine = false`.
    pub owned: BTreeSet<ScriptBuf>,
    pub labels: BTreeMap<ScriptBuf, String>,
}

impl WalletSource for InMemorySource {
    fn network(&self) -> Network {
        self.network
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn accounts(&self) -> Result<SourceAccounts, WalletSourceError> {
        Ok(self.accounts.clone())
    }

    fn transaction(&self, txid: Txid) -> Result<Option<Transaction>, WalletSourceError> {
        Ok(self
            .transactions
            .iter()
            .find(|t| t.compute_txid() == txid)
            .cloned())
    }

    fn address_info(&self, script: &Script) -> Result<AddressInfo, WalletSourceError> {
        // As Core, which has no answer for a script without an address.
        if bitcoin::Address::from_script(script, self.network).is_err() {
            return Ok(AddressInfo::default());
        }
        Ok(AddressInfo {
            ismine: Some(self.owned.contains(script)),
            label: self.labels.get(script).cloned(),
        })
    }
}
