//! Turnkey-compatible request/response envelope for parse endpoints.

use generated::parser::{ChainMetadata, EthereumMetadata, SolanaMetadata, chain_metadata};
use serde::{Deserialize, Serialize};

/// SHA-256 of empty input: used as the canonical "no data" sentinel for digest fields
/// in error responses, where we have no real payload to digest.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Deserialize, Serialize)]
pub struct TurnkeyRequestWrapper {
    pub request: TurnkeyRequest,
}

#[derive(Deserialize, Serialize)]
pub struct TurnkeyRequest {
    pub unsigned_payload: String,
    pub chain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_metadata: Option<ChainMetadataInput>,
    /// Optional borsh-serialized `SignedVerifiedPaymentMarker`, base64-
    /// encoded. Set by `parser_gateway` when it hand-rolls the
    /// verify→settle→sign-VPM flow and POSTs to an HTTP backend
    /// (`parser_http_server`). The receiver base64-decodes and forwards
    /// the raw bytes to `parser_app::routes::parse::parse` as
    /// `ParseRequest.payment_marker`. Open v1 callers leave it None.
    ///
    /// Wire name: `payment_marker_b64` (snake_case, matching the rest of
    /// this envelope — kept compatible with the Go visualsign-turnkey-client).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_marker_b64: Option<String>,
}

// ChainMetadataInput needs Serialize too so parser_gateway can build the
// outbound HTTP body symmetrically with what parser_http_server parses.

/// Tagged representation of chain metadata for unambiguous JSON deserialization.
///
/// The generated `ChainMetadata` uses `serde(untagged)` on the inner oneof enum, which means
/// serde tries Ethereum first. A Solana payload with only `networkId` would be silently
/// decoded as `EthereumMetadata`. This wrapper uses an explicit `chain` discriminator.
#[derive(Deserialize, Serialize)]
#[serde(tag = "chain", rename_all = "camelCase")]
pub enum ChainMetadataInput {
    #[serde(rename = "CHAIN_ETHEREUM")]
    Ethereum(EthereumMetadata),
    #[serde(rename = "CHAIN_SOLANA")]
    Solana(SolanaMetadata),
}

impl From<ChainMetadataInput> for ChainMetadata {
    fn from(input: ChainMetadataInput) -> Self {
        let metadata = match input {
            ChainMetadataInput::Ethereum(eth) => chain_metadata::Metadata::Ethereum(eth),
            ChainMetadataInput::Solana(sol) => chain_metadata::Metadata::Solana(sol),
        };
        ChainMetadata {
            metadata: Some(metadata),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct TurnkeyResponseWrapper {
    pub response: TurnkeyResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeyResponse {
    pub parsed_transaction: TurnkeyParsedTransaction,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeyParsedTransaction {
    pub payload: TurnkeyPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<TurnkeySignature>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeyPayload {
    pub signable_payload: String,
    pub metadata_digest: String,
    pub input_payload_digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeySignature {
    pub message: String,
    pub public_key: String,
    pub scheme: String,
    pub signature: String,
}

pub fn error_response(msg: String) -> TurnkeyResponseWrapper {
    TurnkeyResponseWrapper {
        response: TurnkeyResponse {
            parsed_transaction: TurnkeyParsedTransaction {
                payload: TurnkeyPayload {
                    signable_payload: String::new(),
                    metadata_digest: EMPTY_SHA256.to_string(),
                    input_payload_digest: EMPTY_SHA256.to_string(),
                },
                signature: None,
            },
        },
        error: Some(msg),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_response_has_empty_sha256_digests() {
        let resp = error_response("something broke".to_string());
        let payload = &resp.response.parsed_transaction.payload;
        assert_eq!(payload.metadata_digest, EMPTY_SHA256);
        assert_eq!(payload.input_payload_digest, EMPTY_SHA256);
        assert!(payload.signable_payload.is_empty());
        assert_eq!(resp.error.as_deref(), Some("something broke"));
    }

    #[test]
    fn chain_metadata_input_solana_not_misread_as_ethereum() {
        let json = r#"{"chain":"CHAIN_SOLANA","networkId":"solana-mainnet"}"#;
        let parsed: ChainMetadataInput = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, ChainMetadataInput::Solana(_)));
    }

    #[test]
    fn chain_metadata_input_ethereum_deserializes() {
        let json = r#"{"chain":"CHAIN_ETHEREUM","networkId":"ETHEREUM_MAINNET"}"#;
        let parsed: ChainMetadataInput = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, ChainMetadataInput::Ethereum(_)));
    }

    #[test]
    fn ethereum_metadata_abi_mappings_defaults_when_omitted() {
        let json = r#"{"networkId":"ETHEREUM_MAINNET"}"#;
        let parsed: EthereumMetadata = serde_json::from_str(json).unwrap();
        assert!(parsed.abi_mappings.is_empty());
    }

    #[test]
    fn solana_metadata_idl_mappings_defaults_when_omitted() {
        let json = r#"{"networkId":"SOLANA_MAINNET"}"#;
        let parsed: SolanaMetadata = serde_json::from_str(json).unwrap();
        assert!(parsed.idl_mappings.is_empty());
    }
}
