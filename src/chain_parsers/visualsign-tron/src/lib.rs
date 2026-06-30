#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{TronArgs, TronPlugin};

use anychain_tron::protocol::Tron::{Transaction as TronTransaction, transaction};
use anychain_tron::protocol::balance_contract::{
    DelegateResourceContract, FreezeBalanceV2Contract, TransferContract,
    UnDelegateResourceContract, UnfreezeBalanceV2Contract, WithdrawBalanceContract,
    WithdrawExpireUnfreezeContract,
};
use anychain_tron::protocol::common::ResourceCode;
use anychain_tron::protocol::witness_contract::VoteWitnessContract;
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use protobuf::Message;
use visualsign::field_builders::{create_address_field, create_amount_field, create_text_field};
use visualsign::time_fmt::{format_relative_ms, format_timestamp_ms};
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum TronParserError {
    #[error("Failed to decode transaction: {0}")]
    FailedToDecodeTransaction(String),
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<transaction::Raw, TronParserError> {
    let bytes = match encodings {
        SupportedEncodings::Hex => {
            visualsign::encodings::decode_hex(raw_transaction).map_err(|e| {
                TronParserError::FailedToDecodeTransaction(format!("Failed to decode hex: {e}"))
            })?
        }
        SupportedEncodings::Base64 => b64.decode(raw_transaction).map_err(|e| {
            TronParserError::FailedToDecodeTransaction(format!("Failed to decode base64: {e}"))
        })?,
    };

    parse_tron_bytes(&bytes)
}

// Tron tooling emits two on-the-wire forms:
//   1. Bare `transaction::Raw` bytes (the "raw_data_hex" returned by trongrid).
//   2. The wrapped `Transaction { raw_data, signature, ret }` (what gets broadcast).
//
// The two share field-1 tag (`Raw.ref_block_bytes` and `Transaction.raw_data` are both
// wire-type-2 at tag 1), so a bare Raw whose `ref_block_bytes` payload happens to be a
// parseable sub-message will *also* parse as a wrapped Transaction with `raw_data =
// Some(near-empty Raw)`. Picking the wrapped form unconditionally hides the real bare-Raw
// content. Try both parses and pick the form that decoded a real transaction (at least
// one contract); when neither produced a contract, tie-break to bare — that's the safer
// choice because an accidental wrapped re-parse usually yields an empty inner Raw with
// the original ref_block_bytes lost.
fn parse_tron_bytes(bytes: &[u8]) -> Result<transaction::Raw, TronParserError> {
    let bare_result = transaction::Raw::parse_from_bytes(bytes);
    let wrapped_raw = TronTransaction::parse_from_bytes(bytes)
        .ok()
        .and_then(|w| w.raw_data.into_option());

    match (bare_result, wrapped_raw) {
        (Ok(bare), Some(wrapped)) => {
            if !bare.contract.is_empty() {
                Ok(bare)
            } else if !wrapped.contract.is_empty() {
                Ok(wrapped)
            } else {
                Ok(bare)
            }
        }
        (Ok(bare), None) => Ok(bare),
        (Err(_), Some(wrapped)) => Ok(wrapped),
        (Err(e), None) => Err(TronParserError::FailedToDecodeTransaction(format!(
            "Failed to parse Tron transaction: {e}"
        ))),
    }
}

// This module provides a parser and wrapper for Tron blockchain transactions,
// enabling their decoding and integration with the VisualSign framework.
/// Wrapper for Tron transactions
#[derive(Debug, Clone)]
pub struct TronTransactionWrapper {
    transaction: transaction::Raw,
}

impl Transaction for TronTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        // detect() recognizes an optional 0x/0X prefix as hex.
        let format = SupportedEncodings::detect(data);
        let transaction = decode_transaction(data, format)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;
        Ok(Self { transaction })
    }

    fn transaction_type(&self) -> String {
        "Tron".to_string()
    }
}

impl TronTransactionWrapper {
    pub fn new(transaction: transaction::Raw) -> Self {
        Self { transaction }
    }

    pub fn inner(&self) -> &transaction::Raw {
        &self.transaction
    }
}

/// Converter for Tron transactions
pub struct TronVisualSignConverter;

impl VisualSignConverter<TronTransactionWrapper> for TronVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: TronTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        convert_to_visual_sign_payload(transaction_wrapper.inner().clone(), options)
    }
}

fn convert_to_visual_sign_payload(
    raw_data: transaction::Raw,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();

    fields.push(create_text_field("Network", "Tron")?);

    let now_ms = chrono::Utc::now().timestamp_millis();
    fields.push(create_text_field(
        "Timestamp",
        &render_time_field(raw_data.timestamp, now_ms),
    )?);
    fields.push(create_text_field(
        "Expiration",
        &render_time_field(raw_data.expiration, now_ms),
    )?);

    fields.push(create_amount_field(
        "Fee Limit",
        &sun_to_trx_string(raw_data.fee_limit),
        "TRX",
    )?);

    fields.push(create_text_field(
        "Ref Block",
        &hex::encode(&raw_data.ref_block_bytes),
    )?);

    fields.push(create_text_field(
        "Ref Block Hash",
        &hex::encode(&raw_data.ref_block_hash),
    )?);

    for contract in raw_data.contract.iter() {
        match contract.parameter.as_ref() {
            Some(parameter) => {
                decode_contract(&parameter.type_url, &parameter.value, &mut fields)?;
            }
            None => {
                // Make malformed/incomplete transactions visible to the signer rather than
                // silently dropping the contract entry.
                fields.push(create_text_field("Contract Type", "<missing parameter>")?);
            }
        }
    }

    let title = options
        .transaction_name
        .unwrap_or_else(|| "Tron Transaction".to_string());

    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields
            .into_iter()
            .map(|af| af.signable_payload_field)
            .collect(),
        "TronTx".to_string(),
    ))
}

fn decode_contract(
    type_url: &str,
    value: &[u8],
    fields: &mut Vec<AnnotatedPayloadField>,
) -> Result<(), VisualSignError> {
    match type_url {
        "type.googleapis.com/protocol.TransferContract" => {
            let transfer = TransferContract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode TransferContract: {e}"))
            })?;
            fields.push(create_text_field(
                "Contract Type",
                "TransferContract (TRX Transfer)",
            )?);
            fields.push(create_address_field(
                "From",
                &address_to_base58(&transfer.owner_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_address_field(
                "To",
                &address_to_base58(&transfer.to_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_amount_field(
                "Amount",
                &sun_to_trx_string(transfer.amount),
                "TRX",
            )?);
        }
        "type.googleapis.com/protocol.FreezeBalanceV2Contract" => {
            let freeze = FreezeBalanceV2Contract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode FreezeBalanceV2Contract: {e}"))
            })?;
            fields.push(create_text_field(
                "Contract Type",
                "FreezeBalanceV2 (Stake)",
            )?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&freeze.owner_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_amount_field(
                "Frozen Balance",
                &sun_to_trx_string(freeze.frozen_balance),
                "TRX",
            )?);
            fields.push(create_text_field(
                "Resource",
                &resource_label(freeze.resource),
            )?);
        }
        "type.googleapis.com/protocol.UnfreezeBalanceV2Contract" => {
            let unfreeze = UnfreezeBalanceV2Contract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode UnfreezeBalanceV2Contract: {e}"))
            })?;
            fields.push(create_text_field(
                "Contract Type",
                "UnfreezeBalanceV2 (Unstake)",
            )?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&unfreeze.owner_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_amount_field(
                "Unfreeze Balance",
                &sun_to_trx_string(unfreeze.unfreeze_balance),
                "TRX",
            )?);
            fields.push(create_text_field(
                "Resource",
                &resource_label(unfreeze.resource),
            )?);
        }
        "type.googleapis.com/protocol.WithdrawExpireUnfreezeContract" => {
            let withdraw =
                WithdrawExpireUnfreezeContract::parse_from_bytes(value).map_err(|e| {
                    VisualSignError::ConversionError(format!(
                        "decode WithdrawExpireUnfreezeContract: {e}"
                    ))
                })?;
            fields.push(create_text_field(
                "Contract Type",
                "WithdrawExpireUnfreeze (Claim Unfrozen)",
            )?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&withdraw.owner_address),
                None,
                None,
                None,
                None,
            )?);
        }
        "type.googleapis.com/protocol.WithdrawBalanceContract" => {
            // Claims accumulated voting / Super Representative rewards to the owner's
            // balance. The amount is computed by the chain at execution time, so the
            // contract itself carries only the owner address.
            let withdraw = WithdrawBalanceContract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode WithdrawBalanceContract: {e}"))
            })?;
            fields.push(create_text_field(
                "Contract Type",
                "WithdrawBalance (Claim Rewards)",
            )?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&withdraw.owner_address),
                None,
                None,
                None,
                None,
            )?);
        }
        "type.googleapis.com/protocol.DelegateResourceContract" => {
            let delegate = DelegateResourceContract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode DelegateResourceContract: {e}"))
            })?;
            fields.push(create_text_field("Contract Type", "DelegateResource")?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&delegate.owner_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_address_field(
                "Receiver",
                &address_to_base58(&delegate.receiver_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_text_field(
                "Resource",
                &resource_label(delegate.resource),
            )?);
            fields.push(create_amount_field(
                "Balance",
                &sun_to_trx_string(delegate.balance),
                "TRX",
            )?);
            fields.push(create_text_field(
                "Lock",
                if delegate.lock { "true" } else { "false" },
            )?);
            // When delegate.lock is true, the lock_period (in blocks) is meaningful and
            // must be shown — including when it's the protobuf default 0 (lock-until-manual-undelegate).
            // When delegate.lock is false, only surface a non-zero lock_period as an informational
            // signal that something unusual was set; a zero is the silent default.
            if delegate.lock || delegate.lock_period != 0 {
                fields.push(create_text_field(
                    "Lock Period",
                    &delegate.lock_period.to_string(),
                )?);
            }
        }
        "type.googleapis.com/protocol.UnDelegateResourceContract" => {
            let undelegate = UnDelegateResourceContract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode UnDelegateResourceContract: {e}"))
            })?;
            fields.push(create_text_field("Contract Type", "UnDelegateResource")?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&undelegate.owner_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_address_field(
                "Receiver",
                &address_to_base58(&undelegate.receiver_address),
                None,
                None,
                None,
                None,
            )?);
            fields.push(create_text_field(
                "Resource",
                &resource_label(undelegate.resource),
            )?);
            fields.push(create_amount_field(
                "Balance",
                &sun_to_trx_string(undelegate.balance),
                "TRX",
            )?);
        }
        "type.googleapis.com/protocol.VoteWitnessContract" => {
            let vote = VoteWitnessContract::parse_from_bytes(value).map_err(|e| {
                VisualSignError::ConversionError(format!("decode VoteWitnessContract: {e}"))
            })?;
            fields.push(create_text_field("Contract Type", "Vote Witness")?);
            fields.push(create_address_field(
                "Owner",
                &address_to_base58(&vote.owner_address),
                None,
                None,
                None,
                None,
            )?);

            let mut detail_fields: Vec<AnnotatedPayloadField> = Vec::new();
            for (i, v) in vote.votes.iter().enumerate() {
                let n = i + 1;
                detail_fields.push(create_address_field(
                    &format!("Vote {n} (SR)"),
                    &address_to_base58(&v.vote_address),
                    None,
                    None,
                    None,
                    None,
                )?);
                detail_fields.push(create_text_field(
                    &format!("Vote {n} (Count)"),
                    &v.vote_count.to_string(),
                )?);
            }

            // i64 sum may overflow only for adversarial inputs; saturating keeps the
            // summary readable rather than panicking. Per-vote counts are still shown
            // verbatim in the expanded list.
            let total: i64 = vote
                .votes
                .iter()
                .map(|v| v.vote_count)
                .fold(0i64, i64::saturating_add);
            let subtitle = format!("{} votes across {} SRs", total, vote.votes.len());
            let fallback = format!("Vote Witness: {subtitle}");

            // Condensed view targets hardware-wallet screens with limited room:
            // only the totals plus the owner are echoed. Expanded carries the full
            // per-vote breakdown so signers can audit every SR before signing.
            let condensed_fields = vec![create_text_field("Summary", &subtitle)?];

            fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: fallback.clone(),
                        label: "Votes".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "Vote Witness".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 { text: subtitle }),
                        condensed: Some(SignablePayloadFieldListLayout {
                            fields: condensed_fields,
                        }),
                        expanded: Some(SignablePayloadFieldListLayout {
                            fields: detail_fields,
                        }),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }
        other => {
            fields.push(create_text_field(
                "Contract Type",
                &format!("{other} (not fully decoded)"),
            )?);
        }
    }
    Ok(())
}

impl VisualSignConverterFromString<TronTransactionWrapper> for TronVisualSignConverter {}

// Public API functions
pub fn transaction_to_visual_sign(
    transaction: transaction::Raw,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = TronTransactionWrapper::new(transaction);
    let converter = TronVisualSignConverter;
    converter.to_visual_sign_payload(wrapper, options)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = TronVisualSignConverter;
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

// Tron mainnet addresses are 21 bytes: 0x41 prefix + 20-byte hash, encoded as base58check
// (bs58 with double-SHA256 4-byte checksum). For malformed inputs we return a recognizable
// marker so the signer sees something obviously wrong instead of a confident-looking but
// fake base58 string (e.g. an empty input would otherwise render as the 6-char checksum).
const TRON_ADDRESS_LEN: usize = 21;
const TRON_MAINNET_PREFIX: u8 = 0x41;

fn address_to_base58(address_bytes: &[u8]) -> String {
    if address_bytes.len() != TRON_ADDRESS_LEN || address_bytes[0] != TRON_MAINNET_PREFIX {
        return format!("<invalid Tron address: {}>", hex::encode(address_bytes));
    }
    bs58::encode(address_bytes).with_check().into_string()
}

fn resource_label(resource: protobuf::EnumOrUnknown<ResourceCode>) -> String {
    match resource.enum_value() {
        Ok(ResourceCode::BANDWIDTH) => "BANDWIDTH".to_string(),
        Ok(ResourceCode::ENERGY) => "ENERGY".to_string(),
        Ok(ResourceCode::TRON_POWER) => "TRON_POWER".to_string(),
        Err(n) => format!("UNKNOWN({n})"),
    }
}

// Convert an i64 SUN amount to a TRX decimal string using integer math, so the displayed
// number is a byte-exact representation of the on-chain SUN value at any magnitude
// (f64-based division would round the trailing digits above 2^53 SUN). Output omits the
// fractional point when the value is a whole number of TRX and trims trailing zeros so
// e.g. 1_500_000 SUN -> "1.5", not "1.500000".
fn sun_to_trx_string(sun: i64) -> String {
    let (sign, magnitude) = if sun < 0 {
        ("-", sun.unsigned_abs())
    } else {
        ("", sun as u64)
    };
    let whole = magnitude / 1_000_000;
    let frac = magnitude % 1_000_000;
    if frac == 0 {
        format!("{sign}{whole}")
    } else {
        let mut frac_str = format!("{frac:06}");
        while frac_str.ends_with('0') {
            frac_str.pop();
        }
        format!("{sign}{whole}.{frac_str}")
    }
}

// Renders "<UTC> (<ms> ms[, <relative>])" — the relative tag is omitted when
// the timestamp is outside chrono's representable range so signers still see
// the raw bytes without a misleading "N years ago".
fn render_time_field(ms: i64, now_ms: i64) -> String {
    let abs = format_timestamp_ms(ms);
    match format_relative_ms(ms, now_ms) {
        Some(rel) => format!("{abs} ({ms} ms, {rel})"),
        None => format!("{abs} ({ms} ms)"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use anychain_tron::protocol::Tron::transaction::Contract as TxContract;
    use protobuf::MessageField;
    use protobuf::well_known_types::any::Any;
    use visualsign::{SignablePayloadField, vsptrait::VisualSignOptions};

    fn find_field<'a>(
        payload: &'a SignablePayload,
        label: &str,
    ) -> Option<&'a SignablePayloadField> {
        payload.fields.iter().find(|f| field_label(f) == label)
    }

    fn field_label(field: &SignablePayloadField) -> &str {
        match field {
            SignablePayloadField::TextV2 { common, .. }
            | SignablePayloadField::AmountV2 { common, .. }
            | SignablePayloadField::AddressV2 { common, .. }
            | SignablePayloadField::Number { common, .. }
            | SignablePayloadField::PreviewLayout { common, .. } => &common.label,
            _ => "",
        }
    }

    fn text_value(field: &SignablePayloadField) -> &str {
        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => &text_v2.text,
            _ => panic!("expected TextV2"),
        }
    }

    fn amount_value(field: &SignablePayloadField) -> (&str, &str) {
        match field {
            SignablePayloadField::AmountV2 { amount_v2, .. } => (
                amount_v2.amount.as_str(),
                amount_v2.abbreviation.as_deref().unwrap_or(""),
            ),
            _ => panic!("expected AmountV2"),
        }
    }

    fn address_value(field: &SignablePayloadField) -> &str {
        match field {
            SignablePayloadField::AddressV2 { address_v2, .. } => &address_v2.address,
            _ => panic!("expected AddressV2"),
        }
    }

    fn build_raw_with_contract(type_url: &str, value: Vec<u8>) -> transaction::Raw {
        let any = Any {
            type_url: type_url.to_string(),
            value,
            special_fields: Default::default(),
        };
        let contract = TxContract {
            parameter: MessageField::some(any),
            ..Default::default()
        };
        transaction::Raw {
            ref_block_bytes: vec![0x12, 0x34],
            ref_block_hash: vec![0xab, 0xcd],
            expiration: 1_700_000_000_000,
            timestamp: 1_699_999_999_000,
            fee_limit: 10_000_000,
            contract: vec![contract],
            ..Default::default()
        }
    }

    fn encode_hex(raw: &transaction::Raw) -> String {
        hex::encode(raw.write_to_bytes().unwrap())
    }

    // 21-byte Tron address: 0x41 prefix + 20 bytes. Use a deterministic test address.
    const OWNER_HEX: &str = "416a6ca578c7937e1bf6aea4be657f9d22716c424d";
    const RECEIVER_HEX: &str = "41a614f803b6fd780986a42c78ec9c7f77e6ded13c";

    fn owner_bytes() -> Vec<u8> {
        hex::decode(OWNER_HEX).unwrap()
    }

    fn receiver_bytes() -> Vec<u8> {
        hex::decode(RECEIVER_HEX).unwrap()
    }

    #[test]
    fn address_to_base58_matches_tron_protocol() {
        // Pin byte-equivalence with Tron's mainnet base58check (the encoding used by tronweb,
        // trongrid, every wallet). The 21-byte input 0x41 + 20-byte hash produces a fixed
        // mainnet-style 34-char string starting with 'T'. This test catches any future
        // regression in the bs58 base58check algorithm or in our 21-byte length check.
        let bytes = hex::decode(RECEIVER_HEX).unwrap();
        assert_eq!(
            address_to_base58(&bytes),
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
        );
    }

    #[test]
    fn address_to_base58_rejects_malformed_input() {
        // Empty / too-short / wrong-prefix bytes must NOT silently become a confident-looking
        // base58 string (e.g. the 4-byte checksum of empty input alone) — the signer needs to
        // see something obviously wrong.
        assert!(address_to_base58(&[]).starts_with("<invalid Tron address:"));
        assert!(address_to_base58(&[0x41, 0x00]).starts_with("<invalid Tron address:"));
        // Wrong prefix (mainnet is 0x41) — 0x30 is testnet/other.
        let mut wrong_prefix = vec![0x30];
        wrong_prefix.extend(std::iter::repeat_n(0u8, 20));
        assert!(address_to_base58(&wrong_prefix).starts_with("<invalid Tron address:"));
    }

    fn decode_hex(hex_tx: &str) -> SignablePayload {
        let wrapper = TronTransactionWrapper::from_string(hex_tx).expect("parse");
        TronVisualSignConverter
            .to_visual_sign_payload(wrapper, VisualSignOptions::default())
            .expect("convert")
    }

    // The five `real_onchain_*` tests below pin against actual mainnet transactions
    // pulled from trongrid `/wallet/getblockbynum`. Synthetic tests above exercise the
    // same code paths via hand-built bytes; these guard against library changes that
    // would silently break real-world hex.

    #[test]
    fn real_onchain_freeze_balance_v2() {
        // Block ~50M, 2023-05-09 — early Stake 2.0 freeze of 30 TRX for ENERGY.
        let hex_tx = "0a0232ae220836b3459b9dc805a34080d6d6f9ff305a5a083612560a34747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e467265657a6542616c616e63655632436f6e7472616374121e0a15413b3794ebed7168a9b4883468a45f74047bbf3f4a108087a70e1801709f86d3f9ff30";
        let payload = decode_hex(hex_tx);
        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "FreezeBalanceV2 (Stake)"
        );
        let (amount, abbrev) = amount_value(find_field(&payload, "Frozen Balance").unwrap());
        assert_eq!(amount, "30");
        assert_eq!(abbrev, "TRX");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "ENERGY"
        );
        assert_eq!(
            address_value(find_field(&payload, "Owner").unwrap()),
            "TFNKTGCp3EuZ9qsVXo7gEXwiJwSw1g55aX"
        );
    }

    #[test]
    fn real_onchain_unfreeze_balance_v2() {
        // 2025-08-20 unstake of 500 TRX BANDWIDTH.
        let hex_tx = "0a0268cb22085bb4c8c76b4de7814098a9ceab8c335a5b083712570a36747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e556e667265657a6542616c616e63655632436f6e7472616374121d0a1541ff1d8ee0291aad86b798121a4a697e567a18f2181080cab5ee0170aceecaab8c33";
        let payload = decode_hex(hex_tx);
        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "UnfreezeBalanceV2 (Unstake)"
        );
        let (amount, _) = amount_value(find_field(&payload, "Unfreeze Balance").unwrap());
        assert_eq!(amount, "500");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "BANDWIDTH"
        );
    }

    #[test]
    fn real_onchain_withdraw_expire_unfreeze() {
        // 2025-02-27 claim of expired unstake.
        let hex_tx = "0a021df22208ad699cca97c0673d4088d981b8d4325a5a083812560a3b747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e5769746864726177457870697265556e667265657a65436f6e747261637412170a15410a365e21aaf94b73c0bdf9ac1adaeaf298012a4c70898cfeb7d432";
        let payload = decode_hex(hex_tx);
        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "WithdrawExpireUnfreeze (Claim Unfrozen)"
        );
        assert_eq!(
            address_value(find_field(&payload, "Owner").unwrap()),
            "TAuCtWcdqWuJB63xJsZAopgvkjc1yCG5yj"
        );
    }

    #[test]
    fn real_onchain_delegate_resource() {
        // Block 82913500, 2026-05-22 — 300000 TRX ENERGY delegated, no lock.
        let hex_tx = "0a0228c82208c7d2e90410b55f7f4080d0b3e8e4335a76083912700a35747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e44656c65676174655265736f75726365436f6e747261637412370a154162d3558ee0914a31f3e087fab12f3d8120abd69f10011880f092cbdd08221541b057835a6cc6b296194c288161d5bd11f98d3e342803709d93b0e8e433";
        let payload = decode_hex(hex_tx);
        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "DelegateResource"
        );
        let (amount, abbrev) = amount_value(find_field(&payload, "Balance").unwrap());
        assert_eq!(amount, "300000");
        assert_eq!(abbrev, "TRX");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "ENERGY"
        );
        assert_eq!(text_value(find_field(&payload, "Lock").unwrap()), "false");
    }

    #[test]
    fn real_onchain_undelegate_resource() {
        // Block 82913510, 2026-05-22 — 7128 TRX ENERGY un-delegate.
        let hex_tx = "0a0228d22208aca8662d458ed2b2409992c7f9e4335a75083a12710a37747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e556e44656c65676174655265736f75726365436f6e747261637412360a1541208227e02dfe6742caa77d0e42eb96141e8c991310011880ccf2c61a2215415be6e6d2654d14eea3885da152947f134ce1909e7099f0b1e8e433";
        let payload = decode_hex(hex_tx);
        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "UnDelegateResource"
        );
        let (amount, _) = amount_value(find_field(&payload, "Balance").unwrap());
        assert_eq!(amount, "7128");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "ENERGY"
        );
        assert_eq!(
            address_value(find_field(&payload, "Owner").unwrap()),
            "TCw6YaWm3y6DvxY7M8hrCDnrJGeGMumzGJ"
        );
        assert_eq!(
            address_value(find_field(&payload, "Receiver").unwrap()),
            "TJM96qsBhi5CZpKwBLbuygWDseQQpaungN"
        );
    }

    #[test]
    fn user_fixture_decodes_freeze_balance_v2() {
        // The exact hex the user provided.
        let hex_tx = "0a730a02049d22080f1beff095be0cfd40a097a684e5335a55083612510a34747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e467265657a6542616c616e63655632436f6e747261637412190a15416a6ca578c7937e1bf6aea4be657f9d22716c424d100570a0df8cdbe433";

        let wrapper = TronTransactionWrapper::from_string(hex_tx).expect("parse");
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(wrapper, VisualSignOptions::default())
            .expect("convert");

        assert_eq!(payload.payload_type, "TronTx");
        assert_eq!(payload.title, "Tron Transaction");

        let contract_type = find_field(&payload, "Contract Type").expect("Contract Type field");
        assert_eq!(text_value(contract_type), "FreezeBalanceV2 (Stake)");

        let owner = find_field(&payload, "Owner").expect("Owner field");
        assert!(address_value(owner).starts_with('T'));

        let frozen = find_field(&payload, "Frozen Balance").expect("Frozen Balance field");
        let (amount, abbrev) = amount_value(frozen);
        // The inner contract sets frozen_balance=5 (SUN), which is 0.000005 TRX.
        assert_eq!(amount, "0.000005");
        assert_eq!(abbrev, "TRX");

        let resource = find_field(&payload, "Resource").expect("Resource field");
        // Hex doesn't set resource → default BANDWIDTH.
        assert_eq!(text_value(resource), "BANDWIDTH");
    }

    #[test]
    fn synthetic_freeze_balance_v2_with_energy_resource() {
        let inner = FreezeBalanceV2Contract {
            owner_address: owner_bytes(),
            frozen_balance: 100_000_000, // 100 TRX in SUN
            resource: ResourceCode::ENERGY.into(),
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.FreezeBalanceV2Contract",
            inner.write_to_bytes().unwrap(),
        );
        let hex_tx = encode_hex(&raw);

        let wrapper = TronTransactionWrapper::from_string(&hex_tx).expect("parse");
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(wrapper, VisualSignOptions::default())
            .expect("convert");

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "FreezeBalanceV2 (Stake)"
        );
        let (amount, abbrev) = amount_value(find_field(&payload, "Frozen Balance").unwrap());
        assert_eq!(amount, "100");
        assert_eq!(abbrev, "TRX");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "ENERGY"
        );
    }

    #[test]
    fn synthetic_unfreeze_balance_v2() {
        let inner = UnfreezeBalanceV2Contract {
            owner_address: owner_bytes(),
            unfreeze_balance: 50_000_000,
            resource: ResourceCode::BANDWIDTH.into(),
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.UnfreezeBalanceV2Contract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "UnfreezeBalanceV2 (Unstake)"
        );
        let (amount, _) = amount_value(find_field(&payload, "Unfreeze Balance").unwrap());
        assert_eq!(amount, "50");
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "BANDWIDTH"
        );
    }

    #[test]
    fn synthetic_withdraw_expire_unfreeze() {
        let inner = WithdrawExpireUnfreezeContract {
            owner_address: owner_bytes(),
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.WithdrawExpireUnfreezeContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "WithdrawExpireUnfreeze (Claim Unfrozen)"
        );
        assert!(address_value(find_field(&payload, "Owner").unwrap()).starts_with('T'));
    }

    #[test]
    fn synthetic_withdraw_balance() {
        let inner = WithdrawBalanceContract {
            owner_address: owner_bytes(),
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.WithdrawBalanceContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "WithdrawBalance (Claim Rewards)"
        );
        assert!(address_value(find_field(&payload, "Owner").unwrap()).starts_with('T'));
        // No amount/resource fields — the reward amount is chain-computed, not in the tx.
        assert!(find_field(&payload, "Amount").is_none());
        assert!(find_field(&payload, "Resource").is_none());
    }

    #[test]
    fn synthetic_delegate_resource_with_lock() {
        let inner = DelegateResourceContract {
            owner_address: owner_bytes(),
            receiver_address: receiver_bytes(),
            resource: ResourceCode::ENERGY.into(),
            balance: 1_500_000, // 1.5 TRX
            lock: true,
            lock_period: 86_400,
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.DelegateResourceContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "DelegateResource"
        );
        let (amount, abbrev) = amount_value(find_field(&payload, "Balance").unwrap());
        assert_eq!(amount, "1.5");
        assert_eq!(abbrev, "TRX");
        assert_eq!(text_value(find_field(&payload, "Lock").unwrap()), "true");
        assert_eq!(
            text_value(find_field(&payload, "Lock Period").unwrap()),
            "86400"
        );
        assert_eq!(
            text_value(find_field(&payload, "Resource").unwrap()),
            "ENERGY"
        );
    }

    #[test]
    fn synthetic_undelegate_resource() {
        let inner = UnDelegateResourceContract {
            owner_address: owner_bytes(),
            receiver_address: receiver_bytes(),
            resource: ResourceCode::BANDWIDTH.into(),
            balance: 2_000_000,
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.UnDelegateResourceContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "UnDelegateResource"
        );
        let (amount, _) = amount_value(find_field(&payload, "Balance").unwrap());
        assert_eq!(amount, "2");
        // Lock fields are absent on UnDelegate.
        assert!(find_field(&payload, "Lock").is_none());
    }

    #[test]
    fn unknown_contract_falls_through_to_text() {
        let raw = build_raw_with_contract("type.googleapis.com/protocol.UnknownContract", vec![]);
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        let contract_type = find_field(&payload, "Contract Type").unwrap();
        assert_eq!(
            text_value(contract_type),
            "type.googleapis.com/protocol.UnknownContract (not fully decoded)"
        );
    }

    fn build_vote_witness_bytes(owner: &[u8], votes: &[(&[u8], i64)]) -> Vec<u8> {
        use anychain_tron::protocol::witness_contract::vote_witness_contract::Vote;
        let mut contract = VoteWitnessContract {
            owner_address: owner.to_vec(),
            ..Default::default()
        };
        for (addr, count) in votes {
            contract.votes.push(Vote {
                vote_address: addr.to_vec(),
                vote_count: *count,
                ..Default::default()
            });
        }
        contract.write_to_bytes().unwrap()
    }

    fn preview_layout_subtitle(field: &SignablePayloadField) -> &str {
        match field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => preview_layout
                .subtitle
                .as_ref()
                .map(|t| t.text.as_str())
                .unwrap_or(""),
            _ => panic!("expected PreviewLayout"),
        }
    }

    fn preview_layout_expanded(field: &SignablePayloadField) -> &SignablePayloadFieldListLayout {
        match field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => preview_layout
                .expanded
                .as_ref()
                .expect("expanded must be Some"),
            _ => panic!("expected PreviewLayout"),
        }
    }

    fn preview_layout_condensed(field: &SignablePayloadField) -> &SignablePayloadFieldListLayout {
        match field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => preview_layout
                .condensed
                .as_ref()
                .expect("condensed must be Some"),
            _ => panic!("expected PreviewLayout"),
        }
    }

    #[test]
    fn vote_witness_decodes_owner_and_votes() {
        // 21-byte SR addresses (0x41 prefix + 20 bytes), deterministic.
        let sr1 = hex::decode("4100000000000000000000000000000000000001").unwrap();
        let sr2 = hex::decode("4100000000000000000000000000000000000002").unwrap();
        let owner = owner_bytes();
        let bytes = build_vote_witness_bytes(&owner, &[(&sr1, 1000), (&sr2, 500)]);
        let raw =
            build_raw_with_contract("type.googleapis.com/protocol.VoteWitnessContract", bytes);
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert_eq!(
            text_value(find_field(&payload, "Contract Type").unwrap()),
            "Vote Witness",
        );
        // Owner round-trips through base58check.
        assert_eq!(
            address_value(find_field(&payload, "Owner").unwrap()),
            address_to_base58(&owner),
        );

        let votes_field = find_field(&payload, "Votes").expect("Votes preview layout");
        assert_eq!(
            preview_layout_subtitle(votes_field),
            "1500 votes across 2 SRs"
        );

        // Condensed view: signers with constrained screens see only the summary line.
        let condensed = preview_layout_condensed(votes_field);
        assert_eq!(condensed.fields.len(), 1);
        assert_eq!(
            text_value(&condensed.fields[0].signable_payload_field),
            "1500 votes across 2 SRs",
        );

        // Expanded view: full per-vote breakdown, two fields per vote, in input order.
        let expanded = preview_layout_expanded(votes_field);
        assert_eq!(expanded.fields.len(), 4);
        let labels: Vec<&str> = expanded
            .fields
            .iter()
            .map(|f| field_label(&f.signable_payload_field))
            .collect();
        assert_eq!(
            labels,
            vec![
                "Vote 1 (SR)",
                "Vote 1 (Count)",
                "Vote 2 (SR)",
                "Vote 2 (Count)"
            ],
        );
        assert_eq!(
            address_value(&expanded.fields[0].signable_payload_field),
            address_to_base58(&sr1),
        );
        assert_eq!(
            text_value(&expanded.fields[1].signable_payload_field),
            "1000",
        );
        assert_eq!(
            text_value(&expanded.fields[3].signable_payload_field),
            "500",
        );
    }

    #[test]
    fn vote_witness_empty_votes_renders_zero_summary() {
        let owner = owner_bytes();
        let bytes = build_vote_witness_bytes(&owner, &[]);
        let raw =
            build_raw_with_contract("type.googleapis.com/protocol.VoteWitnessContract", bytes);
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        let votes_field = find_field(&payload, "Votes").expect("Votes preview layout");
        assert_eq!(preview_layout_subtitle(votes_field), "0 votes across 0 SRs");
        assert!(preview_layout_expanded(votes_field).fields.is_empty());
    }

    #[test]
    fn render_time_field_includes_relative_tag() {
        // Past timestamp -> "<ms> ms, N minutes ago".
        let rendered = render_time_field(1_700_000_000_000, 1_700_000_120_000);
        assert_eq!(
            rendered,
            "2023-11-14 22:13:20 UTC (1700000000000 ms, 2 minutes ago)",
        );

        // Future timestamp -> "<ms> ms, in about N hours".
        let rendered = render_time_field(1_700_000_000_000 + 3_600_000, 1_700_000_000_000);
        assert!(
            rendered.ends_with(", in about 1 hour)"),
            "unexpected render: {rendered}",
        );
    }

    #[test]
    fn render_time_field_omits_relative_tag_for_unrepresentable() {
        // i64::MAX is out of chrono's representable range; the helper must NOT include
        // a misleading relative tag and must not double the "(N ms)" suffix.
        assert_eq!(
            render_time_field(i64::MAX, 0),
            format!("invalid timestamp ({} ms)", i64::MAX),
        );
    }

    #[test]
    fn parse_tron_bytes_prefers_bare_when_wrapped_reparse_is_garbage() {
        // Regression for the wrapped-vs-bare ambiguity: a bare Raw whose ref_block_bytes
        // happens to be a parseable sub-Raw (e.g. [0x08, 0x00] = ref_block_num varint)
        // also parses as a wrapped Transaction with an empty inner raw_data. Picking
        // wrapped unconditionally would silently lose the real ref_block_bytes; the
        // contracts-first heuristic must keep the bare interpretation here.
        let bare = transaction::Raw {
            ref_block_bytes: vec![0x08, 0x00],
            ..Default::default()
        };
        let bytes = bare.write_to_bytes().unwrap();
        let parsed = parse_tron_bytes(&bytes).expect("parse");
        assert_eq!(parsed.ref_block_bytes, vec![0x08, 0x00]);
        assert!(parsed.contract.is_empty());
    }

    #[test]
    fn parse_tron_bytes_uses_wrapped_when_only_wrapped_has_contracts() {
        // The other direction: a wrapped Transaction whose bare-Raw reading would
        // yield zero contracts must end up using the wrapped form's inner Raw.
        // The user's real on-chain FreezeBalanceV2 hex hits exactly this case.
        let hex_tx = "0a730a02049d22080f1beff095be0cfd40a097a684e5335a55083612510a34747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e467265657a6542616c616e63655632436f6e747261637412190a15416a6ca578c7937e1bf6aea4be657f9d22716c424d100570a0df8cdbe433";
        let bytes = hex::decode(hex_tx).unwrap();
        let parsed = parse_tron_bytes(&bytes).expect("parse");
        assert_eq!(parsed.contract.len(), 1);
    }

    #[test]
    fn missing_parameter_surfaces_marker_rather_than_silently_dropping() {
        let contract = TxContract {
            parameter: MessageField::none(),
            ..Default::default()
        };
        let raw = transaction::Raw {
            contract: vec![contract],
            ..Default::default()
        };
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();
        let ct = find_field(&payload, "Contract Type").expect("Contract Type marker");
        assert_eq!(text_value(ct), "<missing parameter>");
    }

    #[test]
    fn delegate_resource_with_lock_zero_period_still_shows_lock_period() {
        // Regression for the Lock Period gating bug: when delegate.lock == true,
        // lock_period must be shown even if it equals the protobuf default 0
        // (semantically: lock-until-manual-undelegate).
        let inner = DelegateResourceContract {
            owner_address: owner_bytes(),
            receiver_address: receiver_bytes(),
            resource: ResourceCode::ENERGY.into(),
            balance: 1_000_000,
            lock: true,
            lock_period: 0,
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.DelegateResourceContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();
        assert_eq!(text_value(find_field(&payload, "Lock").unwrap()), "true");
        assert_eq!(
            text_value(find_field(&payload, "Lock Period").unwrap()),
            "0"
        );
    }

    #[test]
    fn sun_to_trx_string_is_byte_exact_at_any_magnitude() {
        // Whole numbers of TRX render without a decimal point.
        assert_eq!(sun_to_trx_string(0), "0");
        assert_eq!(sun_to_trx_string(7_000_000), "7");
        assert_eq!(sun_to_trx_string(100_000_000), "100");

        // Fractional amounts trim trailing zeros.
        assert_eq!(sun_to_trx_string(1), "0.000001");
        assert_eq!(sun_to_trx_string(5), "0.000005");
        assert_eq!(sun_to_trx_string(1_500_000), "1.5");
        assert_eq!(sun_to_trx_string(1_234_567), "1.234567");

        // Negative values keep the sign and the exact fractional digits.
        assert_eq!(sun_to_trx_string(-1), "-0.000001");
        assert_eq!(sun_to_trx_string(-1_500_000), "-1.5");

        // Past 2^53 SUN — where the old f64 path would round — the string is still exact.
        // 9_999_999_999_999_999 SUN = 9999999999.999999 TRX. f64 would round to "10000000000".
        assert_eq!(
            sun_to_trx_string(9_999_999_999_999_999),
            "9999999999.999999"
        );
        // i64::MAX = 9_223_372_036_854_775_807 SUN -> 9223372036854.775807 TRX (last digits preserved).
        assert_eq!(sun_to_trx_string(i64::MAX), "9223372036854.775807");
    }

    #[test]
    fn resource_label_surfaces_unknown_enum_values() {
        // protobuf wire values outside {0,1,2} must render as UNKNOWN(n), not silently
        // collapse to BANDWIDTH.
        let unknown: protobuf::EnumOrUnknown<ResourceCode> = protobuf::EnumOrUnknown::from_i32(99);
        assert_eq!(resource_label(unknown), "UNKNOWN(99)");
    }

    #[test]
    fn transfer_contract_still_uses_address_and_amount_fields() {
        // Regression: the legacy TransferContract path migrated to field_builders,
        // From/To should be AddressV2 and Amount should be AmountV2.
        let inner = TransferContract {
            owner_address: owner_bytes(),
            to_address: receiver_bytes(),
            amount: 7_000_000,
            ..Default::default()
        };
        let raw = build_raw_with_contract(
            "type.googleapis.com/protocol.TransferContract",
            inner.write_to_bytes().unwrap(),
        );
        let payload = TronVisualSignConverter
            .to_visual_sign_payload(
                TronTransactionWrapper::from_string(&encode_hex(&raw)).unwrap(),
                VisualSignOptions::default(),
            )
            .unwrap();

        assert!(address_value(find_field(&payload, "From").unwrap()).starts_with('T'));
        assert!(address_value(find_field(&payload, "To").unwrap()).starts_with('T'));
        let (amount, abbrev) = amount_value(find_field(&payload, "Amount").unwrap());
        assert_eq!(amount, "7");
        assert_eq!(abbrev, "TRX");
    }
}
