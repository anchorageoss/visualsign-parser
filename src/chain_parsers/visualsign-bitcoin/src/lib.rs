//! Bitcoin PSBT (BIP-174) parser for VisualSign.
//!
//! Decodes a base64 or hex PSBT into inputs, outputs and fee. The parser holds
//! no keys: it cannot tell change from payments. A fee whose input amounts or
//! sighash commitments cannot be trusted is shown as unknown, alongside the
//! claimed value. See [`analysis`] for the exact rules.
//!
//! Wallet metadata supplied with the request ([`BitcoinMetadata`]) adds an
//! ownership summary and per-row status lines; see [`metadata`]. Without it
//! the payload is unchanged. The same metadata can be read from the signer's
//! own watch-only node instead; see [`wallet_source`].

mod analysis;
#[cfg(feature = "bitcoind-rpc")]
pub mod bitcoind_rpc;
#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;
pub mod metadata;
#[cfg(feature = "json")]
mod metadata_json;
mod scripts;
pub mod wallet_source;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{BitcoinArgs, BitcoinPlugin};
#[cfg(all(feature = "cli-plugin", feature = "bitcoind-rpc"))]
pub use cli_plugin::{BitcoinCommand, WalletCheckArgs};

pub use analysis::BitcoinParserError;
pub use metadata::{
    BitcoinMetadata, Classified, LocalNode, MetadataSource, NodeCheck, Ownership, OwnershipReport,
};

use analysis::{AmountSource, Analysis, Fee, InputInfo, OutputInfo};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use bitcoin::bip32::ChildNumber;
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

impl BitcoinVisualSignConverter {
    /// Classify `psbt`'s inputs and outputs against `metadata`, for a policy
    /// engine. Fails as conversion would on metadata that does not fit.
    pub fn ownership_report(
        &self,
        psbt: &Psbt,
        metadata: &BitcoinMetadata,
    ) -> Result<OwnershipReport, BitcoinParserError> {
        self.ownership_report_from(psbt, metadata, &MetadataSource::Request)
    }

    /// [`Self::ownership_report`] for metadata from `source`.
    pub fn ownership_report_from(
        &self,
        psbt: &Psbt,
        metadata: &BitcoinMetadata,
        source: &MetadataSource,
    ) -> Result<OwnershipReport, BitcoinParserError> {
        self.analyze_with_metadata(psbt, metadata, source)
            .map(|(_, report)| report)
    }

    /// Convert with wallet metadata. Until `ChainMetadata` has a Bitcoin entry
    /// (see [`metadata`]), callers pass it here instead of through
    /// `VisualSignOptions::metadata`.
    pub fn to_visual_sign_payload_with_metadata(
        &self,
        transaction_wrapper: BitcoinTransactionWrapper,
        options: VisualSignOptions,
        metadata: &BitcoinMetadata,
    ) -> Result<ConversionResult, VisualSignError> {
        self.to_visual_sign_payload_with_metadata_from(
            transaction_wrapper,
            options,
            metadata,
            &MetadataSource::Request,
        )
    }

    /// Convert with wallet metadata from `source`, which every row the
    /// metadata contributes names.
    pub fn to_visual_sign_payload_with_metadata_from(
        &self,
        transaction_wrapper: BitcoinTransactionWrapper,
        options: VisualSignOptions,
        metadata: &BitcoinMetadata,
        source: &MetadataSource,
    ) -> Result<ConversionResult, VisualSignError> {
        let (analysis, report) = self
            .analyze_with_metadata(transaction_wrapper.inner(), metadata, source)
            .map_err(|e| VisualSignError::ConversionError(e.to_string()))?;
        let annotation = Annotation {
            report: &report,
            attribution: Attribution::new(source),
        };
        let payload = build_payload(&analysis, self.network, options, Some(&annotation))?;
        Ok(ConversionResult::new(payload))
    }

    fn analyze_with_metadata(
        &self,
        psbt: &Psbt,
        metadata: &BitcoinMetadata,
        source: &MetadataSource,
    ) -> Result<(Analysis, OwnershipReport), BitcoinParserError> {
        let prepared = metadata::prepare(psbt, self.network, metadata, source)?;
        let analysis = analysis::analyze(&prepared.psbt, self.network)?;
        let report = metadata::classify(&prepared, metadata, source);
        Ok((analysis, report))
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
        let payload = build_payload(&analysis, self.network, options, None)?;
        Ok(ConversionResult::new(payload))
    }
}

impl VisualSignConverterFromString<BitcoinTransactionWrapper> for BitcoinVisualSignConverter {}

/// Format satoshis as a fixed 8-decimal BTC string (valid for `create_amount_field`).
fn btc(amount: Amount) -> String {
    let sat = amount.to_sat();
    format!("{}.{:08}", sat / 100_000_000, sat % 100_000_000)
}

/// The fee as displayed. A verified fee is a bare 8-decimal amount.
fn fee_text(fee: &Fee) -> String {
    match fee {
        Fee::Verified(fee) => btc(*fee),
        Fee::SignatureBound(fee) => {
            format!("{} BTC (claimed; enforced by BIP-341 signature)", btc(*fee))
        }
        Fee::Unverified { fee, reason } => {
            format!("unknown (claimed {} BTC; unverified: {reason})", btc(*fee))
        }
        Fee::Unknown => "unknown (input amounts missing)".to_string(),
    }
}

fn build_payload(
    analysis: &Analysis,
    network: Network,
    options: VisualSignOptions,
    annotation: Option<&Annotation>,
) -> Result<SignablePayload, VisualSignError> {
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();
    if let Some(a) = annotation {
        fields.extend(summary_fields(analysis, a.report, &a.attribution)?);
    }
    fields.push(create_text_field(
        "Address Encoding",
        &format!("{} (transaction network unknown)", network_label(network)),
    )?);

    for (i, input) in analysis.inputs.iter().enumerate() {
        let classified = annotation.map(|a| (&a.report.inputs[i], &a.attribution));
        fields.push(input_field(i, input, analysis.version, classified)?);
    }
    for (i, output) in analysis.outputs.iter().enumerate() {
        let classified = annotation.map(|a| (&a.report.outputs[i], &a.attribution));
        fields.push(output_field(i, output, classified)?);
    }

    match &analysis.fee {
        Fee::Verified(fee) => fields.push(create_amount_field("Fee", &btc(*fee), "BTC")?),
        other => fields.push(create_text_field("Fee", &fee_text(other))?),
    }
    if let Some(Annotation {
        report,
        attribution,
    }) = annotation
    {
        let from_metadata: Vec<String> = report
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.prev_tx_from_metadata)
            .map(|(i, _)| i.to_string())
            .collect();
        if !from_metadata.is_empty() {
            fields.push(create_text_field(
                "Fee Source",
                &format!(
                    "previous transactions for input {} {}",
                    from_metadata.join(", "),
                    attribution.from
                ),
            )?);
        }
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
    classified: Option<(&Classified, &Attribution)>,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let mut expanded = match classified {
        Some((c, attr)) => ownership_fields(c, false, attr)?,
        None => Vec::new(),
    };
    expanded.push(create_text_field(
        "Outpoint",
        &format!("{}:{}", input.outpoint.txid, input.outpoint.vout),
    )?);
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
    let from_metadata = classified
        .filter(|(c, _)| c.prev_tx_from_metadata)
        .map(|(_, attr)| attr);
    expanded.push(create_text_field(
        "Amount Source",
        &match (input.source, from_metadata) {
            (AmountSource::Verified, Some(attr)) => {
                format!("verified against previous transaction {}", attr.from)
            }
            (AmountSource::Verified, None) => "verified against previous transaction".to_string(),
            (AmountSource::Claimed, _) => "claimed by witness_utxo only".to_string(),
            (AmountSource::Missing, _) => "missing".to_string(),
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
    let subtitle = match classified {
        Some((c, attr)) => format!("{} | {subtitle}", status_line(c, false, attr)),
        None => subtitle,
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
    classified: Option<(&Classified, &Attribution)>,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let mut expanded = match classified {
        Some((c, attr)) => ownership_fields(c, true, attr)?,
        None => Vec::new(),
    };
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
    let subtitle = match classified {
        Some((c, attr)) => format!("{} | {subtitle}", status_line(c, true, attr)),
        None => subtitle,
    };
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

/// Attribution appended to everything request metadata contributes.
const FROM_METADATA: &str = "from wallet metadata";

/// An ownership report and how its rows name their source.
struct Annotation<'a> {
    report: &'a OwnershipReport,
    attribution: Attribution,
}

/// How rows contributed by the metadata name where it came from.
struct Attribution {
    /// "from wallet metadata", appended to verified and labelled rows.
    from: String,
    /// "in wallet metadata", for rows that found nothing.
    within: String,
    /// The summary's "Ownership Source" row.
    source: String,
}

impl Attribution {
    fn new(source: &MetadataSource) -> Self {
        match source {
            MetadataSource::Request => Self {
                from: FROM_METADATA.to_string(),
                within: "in wallet metadata".to_string(),
                source: "wallet metadata supplied with this request, checked against the PSBT"
                    .to_string(),
            },
            MetadataSource::LocalNode(node) => {
                let node = format!("your local node ({})", node.detail);
                Self {
                    from: format!("from {node}"),
                    within: format!("on {node}"),
                    source: format!("{node}: its public descriptors, re-derived against the PSBT"),
                }
            }
        }
    }
}

/// One-line ownership status of an input or output.
fn ownership_line(c: &Classified, is_output: bool, attr: &Attribution) -> String {
    let Attribution { from, within, .. } = attr;
    match &c.ownership {
        Ownership::OwnVerified { account, path } => {
            let change = path.as_ref().iter().rev().nth(1) == Some(&ChildNumber::from(1));
            let role = match (is_output, change) {
                (true, true) => "Change",
                (true, false) => "Returning to your account",
                (false, _) => "From your account",
            };
            format!("{role} - {account} (m/{path}) - verified {from}")
        }
        Ownership::ClaimedOwnUnverified { reason } if is_output => format!(
            "WARNING: claims to be your change but does not match your account ({reason}); counted as sending"
        ),
        Ownership::ClaimedOwnUnverified { reason } => {
            format!("WARNING: claims to be from your account but does not match it ({reason})")
        }
        Ownership::External if is_output => {
            format!("External - not one of your accounts {within}")
        }
        Ownership::External => format!("Not from your accounts {within}"),
    }
}

/// The local node's `ismine` answer, if it gave one.
fn node_line(c: &Classified, is_output: bool, attr: &Attribution) -> Option<String> {
    let counted = if is_output {
        "; counted as sending"
    } else {
        ""
    };
    match c.node {
        NodeCheck::NotChecked => None,
        NodeCheck::Agrees { ismine } => Some(format!("ismine={ismine}, agrees - {}", attr.from)),
        NodeCheck::Disagrees { ismine: false } => {
            let claim = match c.ownership {
                Ownership::OwnVerified { .. } => "it re-derives from your account",
                _ => "the PSBT claims it is yours",
            };
            Some(format!(
                "WARNING: your local node says this is not yours (ismine=false), but {claim}{counted}"
            ))
        }
        NodeCheck::Disagrees { ismine: true } => Some(format!(
            "WARNING: your local node says this is yours (ismine=true), but the PSBT does not prove it{counted}"
        )),
    }
}

fn label_line(label: &str, attr: &Attribution) -> String {
    format!("{label} - label {}", attr.from)
}

/// Ownership and label, for a preview subtitle.
fn status_line(c: &Classified, is_output: bool, attr: &Attribution) -> String {
    let mut line = ownership_line(c, is_output, attr);
    if let Some(label) = &c.label {
        line.push_str(&format!(" | {}", label_line(label, attr)));
    }
    if !c.warnings.is_empty() {
        line.push_str(" | WARNING: unusual derivation path");
    }
    if matches!(c.node, NodeCheck::Disagrees { .. }) {
        line.push_str(" | WARNING: your local node disagrees");
    }
    line
}

fn ownership_fields(
    c: &Classified,
    is_output: bool,
    attr: &Attribution,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![create_text_field(
        "Ownership",
        &ownership_line(c, is_output, attr),
    )?];
    if let Some(line) = node_line(c, is_output, attr) {
        fields.push(create_text_field("Node Check", &line)?);
    }
    if let Some(label) = &c.label {
        fields.push(create_text_field("Label", &label_line(label, attr))?);
    }
    for warning in &c.warnings {
        fields.push(create_text_field(
            "Derivation Warning",
            &format!(
                "WARNING: unusual derivation path: {warning} ({} check)",
                attr.from
            ),
        )?);
    }
    Ok(fields)
}

/// "Sending / Returning / Fee" summary, then one top-level warning per false
/// ownership claim and per disagreement with the local node.
fn summary_fields(
    analysis: &Analysis,
    report: &OwnershipReport,
    attr: &Attribution,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let claimed: Vec<usize> = report
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.ownership, Ownership::ClaimedOwnUnverified { .. }))
        .map(|(i, _)| i)
        .collect();
    let disowned = report
        .outputs
        .iter()
        .filter(|c| {
            matches!(c.ownership, Ownership::OwnVerified { .. })
                && matches!(c.node, NodeCheck::Disagrees { .. })
        })
        .count();
    let mut sending = format!(
        "{} BTC to {} output{} not verified as yours",
        btc(report.external_total),
        report.external_count,
        plural(report.external_count)
    );
    if !claimed.is_empty() {
        sending.push_str(&format!(
            " (includes {} falsely claiming to be yours)",
            claimed.len()
        ));
    }
    if disowned > 0 {
        sending.push_str(&format!(
            " (includes {disowned} your local node says {} not yours)",
            if disowned == 1 { "is" } else { "are" }
        ));
    }
    let returning = format!(
        "{} BTC to {} output{} - verified {}",
        btc(report.own_total),
        report.own_count,
        plural(report.own_count),
        attr.from
    );
    let fee = match &analysis.fee {
        Fee::Verified(fee) => format!("{} BTC", btc(*fee)),
        other => fee_text(other),
    };
    let subtitle = format!(
        "Sending {} BTC | Returning to your accounts {} BTC | Fee {fee}",
        btc(report.external_total),
        btc(report.own_total)
    );
    let mut fields = vec![create_preview_layout(
        "Summary",
        subtitle,
        vec![
            create_text_field("Sending", &sending)?,
            create_text_field("Returning to your accounts", &returning)?,
            create_text_field("Fee", &fee)?,
            create_text_field("Ownership Source", &attr.source)?,
        ],
    )];
    for i in claimed {
        fields.push(create_text_field(
            "Warning",
            &format!(
                "Output {i} claims to be your change but does not match your account; counted as sending"
            ),
        )?);
    }
    for (i, c) in report.inputs.iter().enumerate() {
        if matches!(c.ownership, Ownership::ClaimedOwnUnverified { .. }) {
            fields.push(create_text_field(
                "Warning",
                &format!("Input {i} claims to be from your account but does not match it"),
            )?);
        }
    }
    for (i, c) in report.outputs.iter().enumerate() {
        if let NodeCheck::Disagrees { ismine } = c.node {
            fields.push(create_text_field(
                "Warning",
                &format!(
                    "Output {i}: your local node disagrees with the PSBT about ownership (ismine={ismine}); counted as sending"
                ),
            )?);
        }
    }
    for (i, c) in report.inputs.iter().enumerate() {
        if let NodeCheck::Disagrees { ismine } = c.node {
            fields.push(create_text_field(
                "Warning",
                &format!(
                    "Input {i}: your local node disagrees with the PSBT about ownership (ismine={ismine})"
                ),
            )?);
        }
    }
    Ok(fields)
}
