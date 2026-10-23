//! Reads the parser's rendered payload back into plain values, so validation
//! tests can compare it with an independent source (rust-bitcoin's own PSBT
//! accessors, a block explorer, or Bitcoin Core).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bitcoin::Network;
use serde_json::Value;
use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignOptions};
use visualsign_bitcoin::BitcoinVisualSignConverter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedInput {
    pub address: Option<String>,
    pub amount_sat: Option<u64>,
    pub source: String,
    pub sighash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOutput {
    pub address: Option<String>,
    pub script_hex: Option<String>,
    pub amount_sat: u64,
    /// A `Change` field is shown. #577 no longer infers change from
    /// derivation metadata, so validation asserts this is always false.
    pub claims_change: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderedFee {
    Verified(u64),
    Unverified { sat: u64, reason: String },
    Unknown,
}

impl RenderedFee {
    pub fn sat(&self) -> Option<u64> {
        match self {
            Self::Verified(s) | Self::Unverified { sat: s, .. } => Some(*s),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub inputs: Vec<RenderedInput>,
    pub outputs: Vec<RenderedOutput>,
    pub fee: RenderedFee,
}

/// "1.23456789 BTC" (or a leading "1.23456789") to satoshis.
pub fn btc_to_sat(text: &str) -> u64 {
    let amount = text.split_whitespace().next().unwrap();
    let (whole, frac) = amount.split_once('.').unwrap();
    assert_eq!(frac.len(), 8, "amount {text:?} is not 8-decimal BTC");
    whole.parse::<u64>().unwrap() * 100_000_000 + frac.parse::<u64>().unwrap()
}

fn fallback(field: &Value) -> &str {
    field["FallbackText"].as_str().unwrap_or_default()
}

fn expanded(field: &Value) -> Vec<&Value> {
    field["PreviewLayout"]["Expanded"]["Fields"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn child<'a>(fields: &[&'a Value], label: &str) -> Option<&'a Value> {
    fields.iter().copied().find(|f| f["Label"] == label)
}

/// Run the parser exactly as the CLI and servers do, then read the payload back.
pub fn render(psbt: &str, network: Network) -> Result<Rendered, String> {
    let payload = BitcoinVisualSignConverter::new(network)
        .to_visual_sign_payload_from_string(psbt, VisualSignOptions::default())
        .map_err(|e| e.to_string())?
        .payload;
    let json: Value = serde_json::from_str(&payload.to_json().unwrap()).unwrap();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut fee = None;
    for field in json["Fields"].as_array().unwrap() {
        let label = field["Label"].as_str().unwrap_or_default();
        if label.starts_with("Input ") {
            let kids = expanded(field);
            let amount = child(&kids, "Amount").unwrap();
            inputs.push(RenderedInput {
                address: child(&kids, "Address").map(|f| fallback(f).to_string()),
                amount_sat: (amount["Type"] == "amount_v2").then(|| btc_to_sat(fallback(amount))),
                source: fallback(child(&kids, "Amount Source").unwrap()).to_string(),
                sighash: child(&kids, "Sighash Type").map(|f| fallback(f).to_string()),
            });
        } else if label.starts_with("Output ") {
            let kids = expanded(field);
            outputs.push(RenderedOutput {
                address: child(&kids, "Address").map(|f| fallback(f).to_string()),
                script_hex: child(&kids, "Script").map(|f| fallback(f).to_string()),
                amount_sat: btc_to_sat(fallback(child(&kids, "Amount").unwrap())),
                claims_change: child(&kids, "Change").is_some(),
            });
        } else if label == "Fee" {
            let text = fallback(field);
            fee = Some(if field["Type"] == "amount_v2" {
                RenderedFee::Verified(btc_to_sat(text))
            } else if let Some(rest) = text.strip_prefix("unknown (claimed ") {
                // "unknown (claimed X BTC; unverified: reason)"
                let (_, reason) = rest
                    .split_once("; unverified: ")
                    .unwrap_or_else(|| panic!("unexpected fee text {text:?}"));
                RenderedFee::Unverified {
                    sat: btc_to_sat(rest),
                    reason: reason.trim_end_matches(')').to_string(),
                }
            } else {
                assert!(text.starts_with("unknown"), "unexpected fee text {text:?}");
                RenderedFee::Unknown
            });
        }
    }
    Ok(Rendered {
        inputs,
        outputs,
        fee: fee.expect("payload has no Fee field"),
    })
}

/// `render`, with any panic turned into an error naming the input.
pub fn render_no_panic(psbt: &str, network: Network) -> Result<Rendered, String> {
    let owned = psbt.to_string();
    std::panic::catch_unwind(move || render(&owned, network))
        .unwrap_or_else(|_| panic!("parser panicked on {psbt}"))
}
