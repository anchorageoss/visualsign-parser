//! JSON form of [`BitcoinMetadata`], for `parser_cli bitcoin wallet-check
//! --compare` and for exporting what a node said.
//!
//! Every Bitcoin value is a string in its usual text form (xpub, `m/...`
//! path, `txid:vout`, hex), so the file can be read and written by hand:
//!
//! ```json
//! {
//!   "network": "regtest",
//!   "accounts": [{
//!     "name": "Savings", "script_type": "p2wpkh", "master_fingerprint": "8c2d7a1e",
//!     "origin": "m/84'/1'/0'", "xpub": "tpubDC..."
//!   }],
//!   "utxos": [{ "outpoint": "<txid>:0", "label": "Deposit #1234", "prev_tx": "0200..." }],
//!   "output_labels": [{ "index": 0, "label": "Invoice 117" }]
//! }
//! ```
//!
//! An output label takes `index` or `script` (hex), not both.

use crate::metadata::{
    Account, AccountKey, AccountPolicy, AccountScriptType, BitcoinMetadata, OutputLabel,
    OutputSelector, UtxoMetadata,
};
use std::str::FromStr;

use bitcoin::bip32::{DerivationPath, Fingerprint, Xpub};
use bitcoin::hex::{DisplayHex, FromHex};
use bitcoin::{OutPoint, ScriptBuf};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataJson {
    network: String,
    #[serde(default)]
    accounts: Vec<AccountJson>,
    #[serde(default)]
    utxos: Vec<UtxoJson>,
    #[serde(default)]
    output_labels: Vec<OutputLabelJson>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountJson {
    name: String,
    script_type: String,
    master_fingerprint: String,
    origin: String,
    xpub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    descriptor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UtxoJson {
    outpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prev_tx: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputLabelJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    label: String,
}

const SCRIPT_TYPES: [(&str, AccountScriptType); 4] = [
    ("p2pkh", AccountScriptType::P2pkh),
    ("p2sh-p2wpkh", AccountScriptType::P2shP2wpkh),
    ("p2wpkh", AccountScriptType::P2wpkh),
    ("p2tr", AccountScriptType::P2tr),
];

fn field<T: FromStr>(text: &str, what: &str) -> Result<T, String> {
    T::from_str(text).map_err(|_| format!("invalid {what} '{text}'"))
}

impl BitcoinMetadata {
    /// Parse the JSON form. Values are only decoded here; they are checked
    /// against the PSBT when the metadata is used.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let m: MetadataJson =
            serde_json::from_str(json).map_err(|e| format!("metadata JSON: {e}"))?;
        let accounts = m
            .accounts
            .into_iter()
            .map(|a| {
                let script_type = SCRIPT_TYPES
                    .iter()
                    .find(|(name, _)| *name == a.script_type)
                    .map(|(_, t)| *t)
                    .ok_or_else(|| format!("unknown script_type '{}'", a.script_type))?;
                Ok(Account {
                    name: a.name,
                    policy: AccountPolicy::SingleSig {
                        key: AccountKey {
                            master_fingerprint: field::<Fingerprint>(
                                &a.master_fingerprint,
                                "master_fingerprint",
                            )?,
                            origin: field::<DerivationPath>(&a.origin, "origin")?,
                            xpub: field::<Xpub>(&a.xpub, "xpub")?,
                        },
                        script_type,
                    },
                    descriptor: a.descriptor,
                })
            })
            .collect::<Result<_, String>>()?;
        let utxos = m
            .utxos
            .into_iter()
            .map(|u| {
                Ok(UtxoMetadata {
                    outpoint: field::<OutPoint>(&u.outpoint, "outpoint")?,
                    label: u.label,
                    prev_tx: u
                        .prev_tx
                        .map(|h| Vec::<u8>::from_hex(&h).map_err(|_| "invalid prev_tx hex"))
                        .transpose()?,
                })
            })
            .collect::<Result<_, String>>()?;
        let output_labels = m
            .output_labels
            .into_iter()
            .map(|l| {
                let output = match (l.index, l.script) {
                    (Some(i), None) => OutputSelector::Index(i),
                    (None, Some(hex)) => OutputSelector::Script(
                        ScriptBuf::from_hex(&hex).map_err(|_| "invalid output label script")?,
                    ),
                    _ => return Err("an output label needs index or script".to_string()),
                };
                Ok(OutputLabel {
                    output,
                    label: l.label,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            network: crate::parse_network(&m.network)?,
            accounts,
            utxos,
            output_labels,
        })
    }

    /// The JSON form, pretty-printed.
    pub fn to_json(&self) -> String {
        let m = MetadataJson {
            network: self.network.to_string(),
            accounts: self
                .accounts
                .iter()
                .map(|a| {
                    let AccountPolicy::SingleSig { key, script_type } = &a.policy;
                    let script_type = SCRIPT_TYPES
                        .iter()
                        .find(|(_, t)| t == script_type)
                        .map_or("", |(name, _)| name);
                    AccountJson {
                        name: a.name.clone(),
                        script_type: script_type.to_string(),
                        master_fingerprint: key.master_fingerprint.to_string(),
                        origin: format!("m/{}", key.origin),
                        xpub: key.xpub.to_string(),
                        descriptor: a.descriptor.clone(),
                    }
                })
                .collect(),
            utxos: self
                .utxos
                .iter()
                .map(|u| UtxoJson {
                    outpoint: u.outpoint.to_string(),
                    label: u.label.clone(),
                    prev_tx: u.prev_tx.as_ref().map(|b| b.to_lower_hex_string()),
                })
                .collect(),
            output_labels: self
                .output_labels
                .iter()
                .map(|l| {
                    let (index, script) = match &l.output {
                        OutputSelector::Index(i) => (Some(*i), None),
                        OutputSelector::Script(s) => (None, Some(s.to_hex_string())),
                    };
                    OutputLabelJson {
                        index,
                        script,
                        label: l.label.clone(),
                    }
                })
                .collect(),
        };
        // Plain strings and numbers only, so serialization cannot fail.
        serde_json::to_string_pretty(&m).unwrap_or_default()
    }
}
