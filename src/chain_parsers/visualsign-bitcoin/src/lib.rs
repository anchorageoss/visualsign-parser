//! Bitcoin PSBT (BIP-174) parser for VisualSign.
//!
//! Decodes a base64 or hex PSBT into inputs, outputs and fee. The parser holds
//! no keys: it cannot tell change from payments. A fee whose input amounts or
//! sighash commitments cannot be trusted is shown as unknown, alongside the
//! claimed value. See [`analysis`] for the exact rules.

mod analysis;
#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;
mod scripts;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{BitcoinArgs, BitcoinPlugin};

pub use analysis::BitcoinParserError;

use analysis::{AmountSource, Analysis, Fee, InputInfo, OutputInfo};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use bitcoin::psbt::Psbt;
use bitcoin::{Amount, Network};
use visualsign::encodings::SupportedEncodings;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_preview_layout, create_text_field,
};
use visualsign::vsptrait::{
    ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
    VisualSignConverterFromString, VisualSignError, VisualSignOptions,
};
use visualsign::{AnnotatedPayloadField, SignablePayload};

/// Upper bound on the encoded input. Real PSBTs with full previous
/// transactions are well below this.
const MAX_PSBT_CHARS: usize = 4 * 1024 * 1024;

/// Parse a network name as accepted by `--bitcoin-network`.
pub fn parse_network(name: &str) -> Result<Network, String> {
    match name.to_ascii_lowercase().as_str() {
        "bitcoin" | "mainnet" => Ok(Network::Bitcoin),
        "testnet" | "testnet3" => Ok(Network::Testnet),
        "testnet4" => Ok(Network::Testnet4),
        "signet" => Ok(Network::Signet),
        "regtest" => Ok(Network::Regtest),
        other => Err(format!(
            "unknown Bitcoin network '{other}' (expected bitcoin, testnet, testnet4, signet or regtest)"
        )),
    }
}

fn network_label(network: Network) -> String {
    match network {
        Network::Bitcoin => "Bitcoin Mainnet".to_string(),
        Network::Testnet => "Bitcoin Testnet".to_string(),
        Network::Testnet4 => "Bitcoin Testnet4".to_string(),
        Network::Signet => "Bitcoin Signet".to_string(),
        Network::Regtest => "Bitcoin Regtest".to_string(),
    }
}

fn decode_psbt(data: &str) -> Result<Psbt, BitcoinParserError> {
    let data = data.trim();
    if data.len() > MAX_PSBT_CHARS {
        return Err(BitcoinParserError::FailedToDecodeTransaction(
            "PSBT is too large".into(),
        ));
    }
    let bytes = match SupportedEncodings::detect(data) {
        SupportedEncodings::Hex => visualsign::encodings::decode_hex(data).map_err(|e| {
            BitcoinParserError::FailedToDecodeTransaction(format!("Failed to decode hex: {e}"))
        })?,
        SupportedEncodings::Base64 => b64.decode(data).map_err(|e| {
            BitcoinParserError::FailedToDecodeTransaction(format!("Failed to decode base64: {e}"))
        })?,
    };
    let mut remaining = bytes.as_slice();
    let psbt = Psbt::deserialize_from_reader(&mut remaining).map_err(|e| {
        BitcoinParserError::FailedToDecodeTransaction(format!("Failed to parse PSBT: {e}"))
    })?;
    if !remaining.is_empty() {
        return Err(BitcoinParserError::FailedToDecodeTransaction(
            "trailing bytes after PSBT".into(),
        ));
    }
    Ok(psbt)
}

/// Wrapper for a parsed PSBT.
#[derive(Debug, Clone)]
pub struct BitcoinTransactionWrapper {
    psbt: Psbt,
}

impl Transaction for BitcoinTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        let psbt =
            decode_psbt(data).map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;
        Ok(Self { psbt })
    }

    fn transaction_type(&self) -> String {
        "Bitcoin".to_string()
    }
}

impl BitcoinTransactionWrapper {
    pub fn new(psbt: Psbt) -> Self {
        Self { psbt }
    }

    pub fn inner(&self) -> &Psbt {
        &self.psbt
    }
}

/// Converter for Bitcoin PSBTs. A PSBT does not name its network, so the
/// network (used for address encoding) is part of the converter.
#[derive(Debug, Clone, Copy)]
pub struct BitcoinVisualSignConverter {
    network: Network,
}

impl BitcoinVisualSignConverter {
    #[must_use]
    pub fn new(network: Network) -> Self {
        Self { network }
    }
}

impl Default for BitcoinVisualSignConverter {
    fn default() -> Self {
        Self::new(Network::Bitcoin)
    }
}

impl VisualSignConverter<BitcoinTransactionWrapper> for BitcoinVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: BitcoinTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let analysis = analysis::analyze(transaction_wrapper.inner(), self.network)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?;
        let payload = build_payload(&analysis, self.network, options)?;
        Ok(ConversionResult::new(payload))
    }
}

impl VisualSignConverterFromString<BitcoinTransactionWrapper> for BitcoinVisualSignConverter {}

/// Format satoshis as a fixed 8-decimal BTC string (valid for `create_amount_field`).
fn btc(amount: Amount) -> String {
    let sat = amount.to_sat();
    format!("{}.{:08}", sat / 100_000_000, sat % 100_000_000)
}

fn build_payload(
    analysis: &Analysis,
    network: Network,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let mut fields: Vec<AnnotatedPayloadField> = vec![create_text_field(
        "Address Encoding",
        &format!("{} (transaction network unknown)", network_label(network)),
    )?];

    for (i, input) in analysis.inputs.iter().enumerate() {
        fields.push(input_field(i, input, analysis.version)?);
    }
    for (i, output) in analysis.outputs.iter().enumerate() {
        fields.push(output_field(i, output)?);
    }

    match &analysis.fee {
        Fee::Verified(fee) => fields.push(create_amount_field("Fee", &btc(*fee), "BTC")?),
        Fee::Unverified { fee, reason } => fields.push(create_text_field(
            "Fee",
            &format!("unknown (claimed {} BTC; unverified: {reason})", btc(*fee)),
        )?),
        Fee::Unknown => fields.push(create_text_field("Fee", "unknown (input amounts missing)")?),
    }

    fields.push(create_text_field("Version", &analysis.version.to_string())?);
    if analysis.lock_time != 0 {
        fields.push(create_text_field(
            "Lock Time",
            &if !analysis.lock_time_active {
                format!(
                    "{} - inactive (all input sequences final)",
                    analysis.lock_time
                )
            } else if analysis.lock_time < 500_000_000 {
                format!("block height {}", analysis.lock_time)
            } else {
                format!("Unix timestamp {}", analysis.lock_time)
            },
        )?);
    }

    let title = options
        .transaction_name
        .unwrap_or_else(|| "Bitcoin Transaction".to_string());

    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields
            .into_iter()
            .map(|af| af.signable_payload_field)
            .collect(),
        "BitcoinTx".to_string(),
    ))
}

fn input_field(
    index: usize,
    input: &InputInfo,
    version: i32,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let mut expanded = vec![create_text_field(
        "Outpoint",
        &format!("{}:{}", input.outpoint.txid, input.outpoint.vout),
    )?];
    let sequence = input.sequence.to_consensus_u32();
    let rbf = if input.sequence.is_rbf() {
        "signals replaceability"
    } else {
        "does not signal replaceability"
    };
    let relative = if !input.sequence.is_relative_lock_time() {
        "relative lock disabled".to_string()
    } else if version < 2 {
        "relative lock inactive (version < 2)".to_string()
    } else if sequence & (1 << 22) != 0 {
        format!("relative lock: {} seconds", (sequence & 0xffff) * 512)
    } else {
        format!("relative lock: {} blocks", sequence & 0xffff)
    };
    expanded.push(create_text_field(
        "Sequence",
        &format!("0x{sequence:08x} ({rbf}; {relative})"),
    )?);
    if let Some(addr) = &input.address {
        expanded.push(create_address_field(
            "Address", addr, None, None, None, None,
        )?);
    }
    if let Some(script_type) = input.script_type {
        expanded.push(create_text_field("Script Type", script_type)?);
    }
    let amount_text = match input.amount {
        Some(amount) => {
            expanded.push(create_amount_field("Amount", &btc(amount), "BTC")?);
            let marker = if input.source == AmountSource::Claimed {
                " (claimed amount, unverified)"
            } else {
                ""
            };
            format!("{} BTC{marker}", btc(amount))
        }
        None => {
            expanded.push(create_text_field("Amount", "unknown (no UTXO data)")?);
            "amount unknown".to_string()
        }
    };
    expanded.push(create_text_field(
        "Amount Source",
        match input.source {
            AmountSource::Verified => "verified against previous transaction",
            AmountSource::Claimed => "claimed by witness_utxo only",
            AmountSource::Missing => "missing",
        },
    )?);
    expanded.extend(script_fields(&input.scripts)?);
    if let Some(warning) = &input.sighash_warning {
        expanded.push(create_text_field("Sighash Type", warning)?);
    }
    let subtitle = match &input.address {
        Some(addr) => format!("{amount_text} from {addr}"),
        None => amount_text,
    };
    Ok(create_preview_layout(
        &format!("Input {index}"),
        subtitle,
        expanded,
    ))
}

fn output_field(
    index: usize,
    output: &OutputInfo,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let mut expanded = Vec::new();
    match &output.address {
        Some(addr) => expanded.push(create_address_field(
            "Address", addr, None, None, None, None,
        )?),
        None => expanded.push(create_text_field("Script", &output.script_hex)?),
    }
    expanded.push(create_text_field("Script Type", output.script_type)?);
    expanded.push(create_amount_field("Amount", &btc(output.amount), "BTC")?);
    expanded.extend(script_fields(&output.scripts)?);
    let target = match &output.address {
        Some(addr) => addr.clone(),
        None => format!("script {}", output.script_hex),
    };
    let subtitle = format!("{target} {} BTC", btc(output.amount));
    Ok(create_preview_layout(
        &format!("Output {index}"),
        subtitle,
        expanded,
    ))
}

fn script_fields(
    details: &[scripts::ScriptDetail],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = Vec::new();
    for detail in details {
        match detail {
            scripts::ScriptDetail::Multisig(m) => {
                fields.push(create_text_field(
                    "Multisig",
                    &format!("{} of {} ({})", m.threshold, m.cosigners.len(), m.kind),
                )?);
                for (i, c) in m.cosigners.iter().enumerate() {
                    let origin = c.origin.as_deref().unwrap_or("no origin");
                    fields.push(create_text_field(
                        &format!("Cosigner {}", i + 1),
                        &format!("{} ({origin})", c.key_hex),
                    )?);
                }
                if let Some((key, nums)) = &m.internal_key {
                    fields.push(create_text_field("Internal Key", key)?);
                    if !nums {
                        fields.push(create_text_field(
                            "Internal Key Warning",
                            "internal key can spend alone",
                        )?);
                    }
                }
            }
            scripts::ScriptDetail::Unrecognized { script_hex, reason } => {
                fields.push(create_text_field("Script", script_hex)?);
                fields.push(create_text_field("Script Status", reason)?);
            }
        }
    }
    Ok(fields)
}
