//! Caller-supplied wallet metadata for a PSBT (prototype): the caller's
//! accounts, labels for the coins it spends and the outputs it creates, and
//! previous transactions it already holds.
//!
//! Requests reach the parser signed by the backend, so the metadata is treated
//! as attested by it, but it is still only context. It never replaces a fact
//! taken from the transaction (amounts, scripts, addresses), and everything it
//! contributes is shown in the payload and attributed to "wallet metadata".
//!
//! Ownership is never taken from the metadata alone. An input or output is
//! [`Ownership::OwnVerified`] only when the PSBT's `bip32_derivation` /
//! `tap_key_origins` names one of the account fingerprints, the path lies under
//! that account's origin, and the script regenerated from the account xpub at
//! that path is the output's actual script. A derivation naming our
//! fingerprint whose script does not match is [`Ownership::ClaimedOwnUnverified`]
//! (the fake-change attack) and counts as external. Anything else is
//! [`Ownership::External`].
//!
//! Metadata that does not fit the transaction (a label or previous
//! transaction for an outpoint the PSBT does not spend, a previous transaction
//! that does not hash to its outpoint's txid, a label for an output that does
//! not exist) rejects the whole request rather than being ignored.
//!
//! [`MetadataSource`] says where the metadata came from, and every row it
//! contributes names that source. Metadata read from the signer's own node
//! ([`crate::wallet_source`]) also carries the node's own `ismine` answer for
//! each address, which is checked against the parser's re-derivation
//! ([`NodeCheck`]).
//!
//! Proto wiring is not done yet. The intended shape is a `BitcoinMetadata
//! bitcoin = 4;` entry in the `ChainMetadata` oneof of
//! `proto/parser/parser.proto`, converted into [`BitcoinMetadata`] when
//! `VisualSignOptions::metadata` carries it.

use crate::analysis::BitcoinParserError;
use std::collections::{BTreeMap, BTreeSet};

use bitcoin::bip32::{ChildNumber, DerivationPath, Fingerprint, KeySource, Xpub};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{PublicKey, Secp256k1, Verification};
use bitcoin::{
    Amount, CompressedPublicKey, Network, NetworkKind, OutPoint, Script, ScriptBuf, Transaction,
};

/// Longest accepted account name or label, in bytes.
pub const MAX_LABEL_LEN: usize = 64;

/// An own script at an address index above this is shown with a warning. This
/// is not BIP-44's gap limit (a run of unused addresses), which the parser
/// cannot see; it only catches indexes no wallet plausibly reached.
pub const MAX_EXPECTED_INDEX: u32 = 10_000;

/// Wallet context supplied with a signing request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BitcoinMetadata {
    /// Must match the converter's network.
    pub network: Network,
    pub accounts: Vec<Account>,
    pub utxos: Vec<UtxoMetadata>,
    pub output_labels: Vec<OutputLabel>,
}

/// Script type of a single-key account.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AccountScriptType {
    P2pkh,
    P2shP2wpkh,
    P2wpkh,
    /// BIP-86 key path only: no script tree.
    P2tr,
}

/// An account xpub and where it sits under the master key.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccountKey {
    /// Fingerprint of the master key, as PSBT derivations carry it.
    pub master_fingerprint: Fingerprint,
    /// Path from the master key to `xpub`, e.g. `m/84'/0'/0'`.
    pub origin: DerivationPath,
    pub xpub: Xpub,
}

/// How an account's scripts are built from its keys. Only single-sig is
/// implemented; a multisig variant would carry a threshold and several
/// [`AccountKey`]s.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AccountPolicy {
    SingleSig {
        key: AccountKey,
        script_type: AccountScriptType,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Account {
    /// Display name, e.g. "Savings".
    pub name: String,
    pub policy: AccountPolicy,
    /// A full output descriptor. Not supported by the prototype: a request
    /// that carries one is rejected rather than half-checked.
    pub descriptor: Option<String>,
}

/// Context for a coin the transaction spends.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UtxoMetadata {
    pub outpoint: OutPoint,
    /// Display text only, e.g. "Deposit #1234".
    pub label: Option<String>,
    /// Consensus-serialized transaction that created the outpoint. Must hash
    /// to the outpoint's txid.
    pub prev_tx: Option<Vec<u8>>,
}

/// Which output a label refers to.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum OutputSelector {
    Index(u32),
    /// Every output paying this script.
    Script(ScriptBuf),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OutputLabel {
    pub output: OutputSelector,
    /// Display text only.
    pub label: String,
}

/// Where the metadata came from. Every row it contributes names this source.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum MetadataSource {
    /// Supplied with the signing request.
    #[default]
    Request,
    /// Read from the signer's own Bitcoin Core node.
    LocalNode(LocalNode),
}

/// The signer's own node, and its independent answer on ownership.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LocalNode {
    /// Shown after "your local node", e.g. "wallet airgap, no chain sync".
    pub detail: String,
    /// `getaddressinfo(..).ismine` for each input's prevout address. `None`
    /// when the input's script is unknown or has no address.
    pub inputs_ismine: Vec<Option<bool>>,
    /// The same for each output.
    pub outputs_ismine: Vec<Option<bool>>,
}

/// The node's `ismine` answer for one input or output, against the parser.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum NodeCheck {
    /// No node answer: request metadata, or no address to ask about.
    #[default]
    NotChecked,
    /// The node agrees with the parser's re-derivation.
    Agrees { ismine: bool },
    /// The node contradicts the parser's re-derivation or the PSBT's claim.
    /// An output is counted as sending.
    Disagrees { ismine: bool },
}

/// Whether an input or output belongs to one of the caller's accounts.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Ownership {
    /// The script was regenerated from the account xpub at `path`.
    OwnVerified {
        account: String,
        path: DerivationPath,
    },
    /// The PSBT claims one of our fingerprints, but the script does not
    /// belong to the account. Treated as external.
    ClaimedOwnUnverified {
        reason: String,
    },
    External,
}

/// Classification of one input or output.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Classified {
    pub ownership: Ownership,
    /// Label from the metadata.
    pub label: Option<String>,
    /// Unusual but verified derivations (branch, index, depth).
    pub warnings: Vec<String>,
    /// Inputs only: the previous transaction came from the metadata, not the
    /// PSBT.
    pub prev_tx_from_metadata: bool,
    /// The local node's own answer, when the metadata came from one.
    pub node: NodeCheck,
}

/// What the metadata says about a transaction, for display and for policy
/// engines.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OwnershipReport {
    pub inputs: Vec<Classified>,
    pub outputs: Vec<Classified>,
    /// Outputs not verified as ours, including false claims and outputs the
    /// local node disowns.
    pub external_total: Amount,
    pub external_count: usize,
    /// Outputs verified as ours.
    pub own_total: Amount,
    pub own_count: usize,
}

/// The PSBT with previous transactions from the metadata filled in, plus the
/// resolved labels. Analysis runs on `psbt`, so a metadata previous
/// transaction goes through the same txid and `witness_utxo` checks as one
/// carried by the PSBT.
pub(crate) struct Prepared {
    pub psbt: Psbt,
    input_labels: Vec<Option<String>>,
    output_labels: Vec<Option<String>>,
    prev_tx_from_metadata: Vec<bool>,
}

fn invalid(msg: impl Into<String>) -> BitcoinParserError {
    BitcoinParserError::Metadata(msg.into())
}

/// Labels and names are shown verbatim, so they must be short printable ASCII.
fn check_text(text: &str, what: &str) -> Result<(), BitcoinParserError> {
    if text.is_empty() || text.len() > MAX_LABEL_LEN {
        return Err(invalid(format!(
            "{what} must be 1 to {MAX_LABEL_LEN} characters"
        )));
    }
    if !text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(invalid(format!("{what} must be printable ASCII")));
    }
    Ok(())
}

fn check_source(source: &MetadataSource, tx: &Transaction) -> Result<(), BitcoinParserError> {
    let MetadataSource::LocalNode(node) = source else {
        return Ok(());
    };
    check_text(&node.detail, "node description")?;
    if node.inputs_ismine.len() != tx.input.len() || node.outputs_ismine.len() != tx.output.len() {
        return Err(invalid(
            "node answers do not match the transaction's inputs and outputs",
        ));
    }
    Ok(())
}

fn check_account(account: &Account, network: Network) -> Result<(), BitcoinParserError> {
    check_text(&account.name, "account name")?;
    if account.descriptor.is_some() {
        return Err(invalid(format!(
            "account '{}': descriptors are not supported yet",
            account.name
        )));
    }
    let AccountPolicy::SingleSig { key, .. } = &account.policy;
    if key.xpub.network != NetworkKind::from(network) {
        return Err(invalid(format!(
            "account '{}': xpub is for another network",
            account.name
        )));
    }
    let depth_matches = usize::from(key.xpub.depth) == key.origin.len();
    let child_matches = key
        .origin
        .as_ref()
        .last()
        .is_none_or(|c| *c == key.xpub.child_number);
    if !depth_matches || !child_matches {
        return Err(invalid(format!(
            "account '{}': origin path does not match the xpub",
            account.name
        )));
    }
    Ok(())
}

/// Validate `metadata` against `psbt` and fill in its previous transactions.
pub(crate) fn prepare(
    psbt: &Psbt,
    network: Network,
    metadata: &BitcoinMetadata,
    source: &MetadataSource,
) -> Result<Prepared, BitcoinParserError> {
    if metadata.network != network {
        return Err(invalid(format!(
            "metadata is for {}, the request for {network}",
            metadata.network
        )));
    }
    for account in &metadata.accounts {
        check_account(account, network)?;
    }
    check_source(source, &psbt.unsigned_tx)?;

    let tx = &psbt.unsigned_tx;
    let mut out = psbt.clone();
    let mut input_labels = vec![None; tx.input.len()];
    let mut prev_tx_from_metadata = vec![false; tx.input.len()];
    let mut seen = BTreeSet::new();
    for utxo in &metadata.utxos {
        let outpoint = utxo.outpoint;
        if !seen.insert(outpoint) {
            return Err(invalid(format!("outpoint {outpoint} listed twice")));
        }
        let index = tx
            .input
            .iter()
            .position(|i| i.previous_output == outpoint)
            .ok_or_else(|| {
                invalid(format!(
                    "outpoint {outpoint} is not spent by this transaction"
                ))
            })?;
        if let Some(label) = &utxo.label {
            check_text(label, "label")?;
            input_labels[index] = Some(label.clone());
        }
        if let Some(bytes) = &utxo.prev_tx {
            let prev: Transaction = bitcoin::consensus::deserialize(bytes).map_err(|e| {
                invalid(format!(
                    "previous transaction for {outpoint} does not decode: {e}"
                ))
            })?;
            if prev.compute_txid() != outpoint.txid {
                return Err(invalid(format!(
                    "previous transaction for {outpoint} does not hash to its txid"
                )));
            }
            // A PSBT `non_witness_utxo` with this txid is the same transaction,
            // so only fill the gap.
            if out.inputs[index].non_witness_utxo.is_none() {
                out.inputs[index].non_witness_utxo = Some(prev);
                prev_tx_from_metadata[index] = true;
            }
        }
    }

    let mut output_labels = vec![None; tx.output.len()];
    for label in &metadata.output_labels {
        check_text(&label.label, "label")?;
        let targets: Vec<usize> = match &label.output {
            OutputSelector::Index(i) => usize::try_from(*i)
                .ok()
                .filter(|i| *i < tx.output.len())
                .into_iter()
                .collect(),
            OutputSelector::Script(script) => tx
                .output
                .iter()
                .enumerate()
                .filter(|(_, o)| o.script_pubkey == *script)
                .map(|(i, _)| i)
                .collect(),
        };
        if targets.is_empty() {
            return Err(invalid(format!(
                "label '{}' refers to an output this transaction does not have",
                label.label
            )));
        }
        for i in targets {
            if output_labels[i].replace(label.label.clone()).is_some() {
                return Err(invalid(format!("output {i} is labelled twice")));
            }
        }
    }

    Ok(Prepared {
        psbt: out,
        input_labels,
        output_labels,
        prev_tx_from_metadata,
    })
}

fn script_for<C: Verification>(
    secp: &Secp256k1<C>,
    script_type: AccountScriptType,
    key: PublicKey,
) -> ScriptBuf {
    let compressed = CompressedPublicKey(key);
    match script_type {
        AccountScriptType::P2pkh => ScriptBuf::new_p2pkh(&compressed.pubkey_hash()),
        AccountScriptType::P2shP2wpkh => {
            ScriptBuf::new_p2sh(&ScriptBuf::new_p2wpkh(&compressed.wpubkey_hash()).script_hash())
        }
        AccountScriptType::P2wpkh => ScriptBuf::new_p2wpkh(&compressed.wpubkey_hash()),
        AccountScriptType::P2tr => ScriptBuf::new_p2tr(secp, key.x_only_public_key().0, None),
    }
}

fn path_text(path: &DerivationPath) -> String {
    format!("m/{path}")
}

/// Why a verified path is unusual, if it is. Standard wallets derive
/// `<origin>/<0 or 1>/<index>`.
fn path_warnings(rest: &[ChildNumber]) -> Vec<String> {
    let mut warnings = Vec::new();
    match rest {
        [branch, index] => {
            let branch = u32::from(*branch);
            let index = u32::from(*index);
            if branch > 1 {
                warnings.push(format!(
                    "branch {branch} is neither receive (0) nor change (1)"
                ));
            }
            if index > MAX_EXPECTED_INDEX {
                warnings.push(format!(
                    "address index {index} is above {MAX_EXPECTED_INDEX}"
                ));
            }
        }
        _ => warnings.push(format!(
            "{} steps below the account instead of 2",
            rest.len()
        )),
    }
    warnings
}

/// Classify a script against the accounts, using the derivations the PSBT
/// attaches to it.
fn classify_script<C: Verification>(
    secp: &Secp256k1<C>,
    script: Option<&Script>,
    claims: &[KeySource],
    accounts: &[Account],
) -> (Ownership, Vec<String>) {
    let mut reason = None;
    // "Outside account X" is the least specific reason: with several accounts
    // under one fingerprint, the account the path lies under explains more.
    let mut outside = None;
    for (fingerprint, path) in claims {
        for account in accounts {
            let AccountPolicy::SingleSig { key, script_type } = &account.policy;
            if key.master_fingerprint != *fingerprint {
                continue;
            }
            let shown = path_text(path);
            let why = match path.as_ref().strip_prefix(key.origin.as_ref()) {
                None => {
                    outside.get_or_insert_with(|| {
                        format!(
                            "path {shown} is outside account {} ({})",
                            account.name,
                            path_text(&key.origin)
                        )
                    });
                    continue;
                }
                Some([]) => format!("path {shown} is the account key itself"),
                Some(rest) if rest.iter().any(ChildNumber::is_hardened) => {
                    format!("path {shown} has hardened steps below the account")
                }
                Some(rest) => {
                    let Some(script) = script else {
                        reason.get_or_insert_with(|| "its script is unknown".to_string());
                        continue;
                    };
                    let Ok(child) = key.xpub.derive_pub(secp, &rest) else {
                        continue;
                    };
                    if script_for(secp, *script_type, child.public_key) == *script {
                        return (
                            Ownership::OwnVerified {
                                account: account.name.clone(),
                                path: path.clone(),
                            },
                            path_warnings(rest),
                        );
                    }
                    format!("script is not account {} at {shown}", account.name)
                }
            };
            reason.get_or_insert(why);
        }
    }
    match reason.or(outside) {
        Some(reason) => (Ownership::ClaimedOwnUnverified { reason }, Vec::new()),
        None => (Ownership::External, Vec::new()),
    }
}

fn claims_of(
    bip32: &BTreeMap<PublicKey, KeySource>,
    tap: &BTreeMap<bitcoin::XOnlyPublicKey, (Vec<bitcoin::TapLeafHash>, KeySource)>,
) -> Vec<KeySource> {
    bip32
        .values()
        .cloned()
        .chain(tap.values().map(|(_, source)| source.clone()))
        .collect()
}

/// Compare the node's `ismine` with the parser. A false ownership claim is a
/// disagreement whatever the node says: the PSBT and the node cannot both be
/// right.
fn node_check(ownership: &Ownership, ismine: Option<bool>) -> NodeCheck {
    match (ownership, ismine) {
        (_, None) => NodeCheck::NotChecked,
        (Ownership::OwnVerified { .. }, Some(true)) | (Ownership::External, Some(false)) => {
            NodeCheck::Agrees {
                ismine: ismine == Some(true),
            }
        }
        (_, Some(ismine)) => NodeCheck::Disagrees { ismine },
    }
}

/// The script an input spends, as the PSBT (after [`prepare`]) gives it.
pub(crate) fn prevout_script(psbt: &Psbt, index: usize) -> Option<&Script> {
    let input = &psbt.inputs[index];
    let vout = usize::try_from(psbt.unsigned_tx.input[index].previous_output.vout).ok()?;
    match &input.non_witness_utxo {
        Some(prev) => prev.output.get(vout).map(|o| o.script_pubkey.as_script()),
        None => input
            .witness_utxo
            .as_ref()
            .map(|o| o.script_pubkey.as_script()),
    }
}

/// Classify every input and output. `prepared.psbt` must already have passed
/// analysis, and `source` [`prepare`].
pub(crate) fn classify(
    prepared: &Prepared,
    metadata: &BitcoinMetadata,
    source: &MetadataSource,
) -> OwnershipReport {
    let secp = Secp256k1::verification_only();
    let psbt = &prepared.psbt;
    let (inputs_ismine, outputs_ismine) = match source {
        MetadataSource::LocalNode(node) => (&node.inputs_ismine[..], &node.outputs_ismine[..]),
        MetadataSource::Request => (&[][..], &[][..]),
    };
    let inputs = psbt
        .inputs
        .iter()
        .enumerate()
        .map(|(i, input)| {
            let claims = claims_of(&input.bip32_derivation, &input.tap_key_origins);
            let (ownership, warnings) =
                classify_script(&secp, prevout_script(psbt, i), &claims, &metadata.accounts);
            let node = node_check(&ownership, inputs_ismine.get(i).copied().flatten());
            Classified {
                ownership,
                label: prepared.input_labels[i].clone(),
                warnings,
                prev_tx_from_metadata: prepared.prev_tx_from_metadata[i],
                node,
            }
        })
        .collect();

    let mut report = OwnershipReport {
        inputs,
        outputs: Vec::new(),
        external_total: Amount::ZERO,
        external_count: 0,
        own_total: Amount::ZERO,
        own_count: 0,
    };
    for (i, (txout, output)) in psbt
        .unsigned_tx
        .output
        .iter()
        .zip(&psbt.outputs)
        .enumerate()
    {
        let claims = claims_of(&output.bip32_derivation, &output.tap_key_origins);
        let (ownership, warnings) = classify_script(
            &secp,
            Some(&txout.script_pubkey),
            &claims,
            &metadata.accounts,
        );
        let node = node_check(&ownership, outputs_ismine.get(i).copied().flatten());
        // Analysis has already bounded the output total by MAX_MONEY.
        let own = matches!(ownership, Ownership::OwnVerified { .. })
            && !matches!(node, NodeCheck::Disagrees { .. });
        if own {
            report.own_total += txout.value;
            report.own_count += 1;
        } else {
            report.external_total += txout.value;
            report.external_count += 1;
        }
        report.outputs.push(Classified {
            ownership,
            label: prepared.output_labels[i].clone(),
            warnings,
            prev_tx_from_metadata: false,
            node,
        });
    }
    report
}
