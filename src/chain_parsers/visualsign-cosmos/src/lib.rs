//! Cosmos SDK chain parser for VisualSign.
//!
//! Decodes the bytes a Cosmos signer is asked to sign, in both sign modes:
//!
//! - `SIGN_MODE_DIRECT`: the protobuf `SignDoc` (hex or base64).
//! - `SIGN_MODE_LEGACY_AMINO_JSON`: the canonical `StdSignDoc` JSON.
//!
//! The input is the exact sign bytes, not a broadcast `TxRaw`: the signer
//! approves what it signs, so that is what is displayed. A `TxRaw` is not a
//! `SignDoc` and is rejected.
//!
//! Supported messages: bank `MsgSend`; staking `MsgDelegate`,
//! `MsgUndelegate` and `MsgBeginRedelegate`; distribution
//! `MsgWithdrawDelegatorReward`; gov `MsgVote` (v1beta1 and v1); authz
//! `MsgExec` (recursive, depth-capped at [`MAX_EXEC_DEPTH`]); IBC
//! `MsgTransfer`; and wasm `MsgExecuteContract` (with its JSON message).
//! Any other message is shown with its type URL and marked as not fully
//! decoded. The wire messages are hand-written `prost` structs, so the crate
//! does not depend on `cosmrs` or the Cosmos SDK protos.

mod amino;
#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;
mod direct;
mod out;
pub mod proto;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{CosmosArgs, CosmosPlugin};
pub use direct::ML_DSA_65_PUBKEY_TYPE_URL;
pub use out::MAX_EXEC_DEPTH;

use amino::AminoDoc;
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use direct::DirectDoc;
use visualsign::{
    SignablePayload,
    encodings::SupportedEncodings,
    vsptrait::{
        ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
        VisualSignConverterFromString, VisualSignError, VisualSignOptions,
    },
};

/// Largest sign doc accepted, in bytes (before hex or base64 decoding).
const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
enum SignDocument {
    Direct(Box<DirectDoc>),
    Amino(AminoDoc),
}

/// A Cosmos sign doc, ready to render.
#[derive(Debug, Clone)]
pub struct CosmosTransaction {
    document: SignDocument,
}

impl Transaction for CosmosTransaction {
    /// Accepts the canonical amino JSON text, or a protobuf `SignDoc` as hex
    /// (optional `0x` prefix) or base64.
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        if data.len() > MAX_INPUT_BYTES {
            return Err(TransactionParseError::InvalidFormat(
                "sign doc too large".to_string(),
            ));
        }
        let data = data.trim();
        let document = if data.starts_with('{') {
            SignDocument::Amino(AminoDoc::parse(data)?)
        } else {
            let bytes = match SupportedEncodings::detect(data) {
                SupportedEncodings::Hex => visualsign::encodings::decode_hex(data)
                    .map_err(|e| TransactionParseError::DecodeError(format!("hex: {e}")))?,
                SupportedEncodings::Base64 => b64
                    .decode(data)
                    .map_err(|e| TransactionParseError::DecodeError(format!("base64: {e}")))?,
            };
            SignDocument::Direct(Box::new(DirectDoc::parse(&bytes)?))
        };
        Ok(Self { document })
    }

    fn transaction_type(&self) -> String {
        "Cosmos".to_string()
    }
}

/// Converter for Cosmos sign docs.
pub struct CosmosVisualSignConverter;

impl VisualSignConverter<CosmosTransaction> for CosmosVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: CosmosTransaction,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let fields = match &transaction.document {
            SignDocument::Direct(doc) => doc.render()?,
            SignDocument::Amino(doc) => doc.render()?,
        };
        let title = options
            .transaction_name
            .unwrap_or_else(|| "Cosmos Transaction".to_string());
        // Cosmos has no intermediate_output schema yet.
        Ok(ConversionResult::new(SignablePayload::new(
            0,
            title,
            None,
            fields,
            "CosmosTx".to_string(),
        )))
    }
}

impl VisualSignConverterFromString<CosmosTransaction> for CosmosVisualSignConverter {}
