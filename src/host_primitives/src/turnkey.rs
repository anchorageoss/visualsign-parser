//! Turnkey-compatible request/response envelope for parse endpoints.

use generated::parser::{
    ChainMetadata, EthereumMetadata, NearMetadata, SolanaMetadata, chain_metadata,
};
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
    /// Opt-in for the chain-specific `intermediate_output` blob. Defaults to
    /// false so existing REST callers that omit it behave exactly as before.
    #[serde(default)]
    pub include_intermediate_output: bool,
}

// Serialize is derived on the request types (not just Deserialize) for a future
// caller that builds this envelope symmetrically to what parses it; no in-tree
// consumer serializes a request today.

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
    #[serde(rename = "CHAIN_NEAR")]
    Near(NearMetadata),
}

impl From<ChainMetadataInput> for ChainMetadata {
    fn from(input: ChainMetadataInput) -> Self {
        let metadata = match input {
            ChainMetadataInput::Ethereum(eth) => chain_metadata::Metadata::Ethereum(eth),
            ChainMetadataInput::Solana(sol) => chain_metadata::Metadata::Solana(sol),
            ChainMetadataInput::Near(near) => chain_metadata::Metadata::Near(near),
        };
        ChainMetadata {
            metadata: Some(metadata),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeyResponseWrapper {
    /// Top-level boot proof, matching the production Turnkey visualsign API
    /// response shape that wallet integrators consume. Injected by the caller
    /// because the value is deployment-specific: `parser_gateway` supplies a
    /// stable local-dev mock, `parser_http_server` supplies the real attested
    /// one. See issue #337.
    pub boot_proof: TurnkeyBootProof,
    pub response: TurnkeyResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Boot proof object shape, matching the production Turnkey visualsign API
/// that wallet integrators consume. The reference Go client uses the same
/// field names — see [visualsign-turnkeyclient/api/types.go::TurnkeyBootProof][types].
///
/// [types]: https://github.com/anchorageoss/visualsign-turnkeyclient/blob/main/api/types.go#L128
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeyBootProof {
    pub aws_attestation_doc_b64: String,
    pub qos_manifest_b64: String,
    pub qos_manifest_envelope_b64: String,
    pub ephemeral_public_key_hex: String,
    pub enclave_app: String,
    pub deployment_label: String,
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
    /// Chain-specific, borsh-serialized structured decode, base64-encoded (proto
    /// `bytes` JSON convention). Empty and omitted from the response when the
    /// request did not opt in or the chain has no intermediate output, so
    /// responses to existing consumers stay byte-identical.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub intermediate_output: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnkeySignature {
    pub message: String,
    pub public_key: String,
    pub scheme: String,
    pub signature: String,
}

/// Error envelope. `boot_proof` is injected by the caller because the value is
/// deployment-specific: `parser_gateway` supplies a stable local-dev mock,
/// `parser_http_server` supplies the real attested one.
pub fn error_response(msg: String, boot_proof: TurnkeyBootProof) -> TurnkeyResponseWrapper {
    TurnkeyResponseWrapper {
        boot_proof,
        response: TurnkeyResponse {
            parsed_transaction: TurnkeyParsedTransaction {
                payload: TurnkeyPayload {
                    signable_payload: String::new(),
                    metadata_digest: EMPTY_SHA256.to_string(),
                    input_payload_digest: EMPTY_SHA256.to_string(),
                    intermediate_output: String::new(),
                },
                signature: None,
            },
        },
        error: Some(msg),
    }
}

/// Success envelope. `boot_proof` is injected by the caller for the same reason as
/// in [`error_response`].
pub fn success_response(
    boot_proof: TurnkeyBootProof,
    payload: TurnkeyPayload,
    signature: Option<TurnkeySignature>,
) -> TurnkeyResponseWrapper {
    TurnkeyResponseWrapper {
        boot_proof,
        response: TurnkeyResponse {
            parsed_transaction: TurnkeyParsedTransaction { payload, signature },
        },
        error: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn probe_boot_proof() -> TurnkeyBootProof {
        TurnkeyBootProof {
            aws_attestation_doc_b64: "doc".to_string(),
            qos_manifest_b64: "man".to_string(),
            qos_manifest_envelope_b64: "env".to_string(),
            ephemeral_public_key_hex: "02ab".to_string(),
            enclave_app: "visualsign-parser".to_string(),
            deployment_label: "test".to_string(),
        }
    }

    #[test]
    fn boot_proof_wire_shape_is_exactly_six_camel_case_keys() {
        // Wallet contract, issue #337. Field set is fixed by
        // visualsign-turnkeyclient/api/types.go::TurnkeyBootProof.
        let value = serde_json::to_value(probe_boot_proof()).unwrap();
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let expected: std::collections::BTreeSet<&str> = [
            "awsAttestationDocB64",
            "qosManifestB64",
            "qosManifestEnvelopeB64",
            "ephemeralPublicKeyHex",
            "enclaveApp",
            "deploymentLabel",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys, expected,
            "bootProof field set must match production wire shape exactly"
        );
    }

    #[test]
    fn error_response_carries_the_supplied_boot_proof() {
        // Strict consumers reject any response without bootProof, including errors.
        let resp = error_response("oops".to_string(), probe_boot_proof());
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            value.get("bootProof").unwrap().get("enclaveApp").unwrap(),
            "visualsign-parser"
        );
        assert_eq!(value.get("error").unwrap(), "oops");
        assert_eq!(
            value
                .get("response")
                .unwrap()
                .get("parsedTransaction")
                .unwrap()
                .get("payload")
                .unwrap()
                .get("metadataDigest")
                .unwrap(),
            EMPTY_SHA256
        );
    }

    #[test]
    fn intermediate_output_empty_is_omitted() {
        // Byte-identical responses for callers that did not opt in (#414).
        let payload = TurnkeyPayload {
            signable_payload: "sp".to_string(),
            metadata_digest: "md".to_string(),
            input_payload_digest: "ipd".to_string(),
            intermediate_output: String::new(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert!(value.get("intermediateOutput").is_none());
    }

    #[test]
    fn chain_metadata_input_solana_not_misread_as_ethereum() {
        let json = r#"{"chain":"CHAIN_SOLANA","networkId":"solana-mainnet"}"#;
        let parsed: ChainMetadataInput = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, ChainMetadataInput::Solana(_)));
    }

    #[test]
    fn response_wrapper_round_trips_when_intermediate_output_is_omitted() {
        // A response with no intermediate_output must still deserialize back into
        // TurnkeyResponseWrapper, not just serialize cleanly: the omitted key needs
        // a default on the way back in, since this is the shape every existing
        // caller produces.
        let resp = error_response("oops".to_string(), probe_boot_proof());
        let json = serde_json::to_string(&resp).unwrap();
        let round_tripped: TurnkeyResponseWrapper = serde_json::from_str(&json).unwrap();
        assert!(
            round_tripped
                .response
                .parsed_transaction
                .payload
                .intermediate_output
                .is_empty()
        );
    }
}
