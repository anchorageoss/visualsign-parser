//! Shared parse handler. Used by both /visualsign/api/v1/parse (open, Turnkey)
//! and /visualsign/api/v2/parse (x402-gated).

use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use base64::Engine as _;
use generated::parser::{
    Chain, ChainMetadata, ParseRequest, ParsedTransactionPayload, SignatureScheme,
};
use generated::tonic;
use host_primitives::turnkey::{
    TurnkeyBootProof, TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponseWrapper,
    TurnkeySignature, error_response as turnkey_error_response,
    success_response as turnkey_success_response,
};
use qos_crypto::sha_256;
use std::time::Duration;

/// Stable mock used in every gateway response. The base64 sentinels decode to
/// "TURNKEY_GATEWAY_MOCK_BOOT_PROOF" and "TURNKEY_GATEWAY_MOCK_QOS_MANIFEST*" --
/// pure placeholders, not signed attestation. Real attestation verifiers will
/// reject them. Kept stable so downstream test fixtures can pin against them.
const MOCK_BOOT_PROOF_AWS_DOC: &str = "VFVSTktFWV9HQVRFV0FZX01PQ0tfQk9PVF9QUk9PRg==";
const MOCK_BOOT_PROOF_QOS_MANIFEST: &str = "VFVSTktFWV9HQVRFV0FZX01PQ0tfUU9TX01BTklGRVNU";
const MOCK_BOOT_PROOF_QOS_MANIFEST_ENV: &str =
    "VFVSTktFWV9HQVRFV0FZX01PQ0tfUU9TX01BTklGRVNUX0VOVkVMT1BF";
const MOCK_BOOT_PROOF_EPHEMERAL_PK: &str =
    "020000000000000000000000000000000000000000000000000000000000000001";

fn mock_boot_proof() -> TurnkeyBootProof {
    TurnkeyBootProof {
        aws_attestation_doc_b64: MOCK_BOOT_PROOF_AWS_DOC.to_string(),
        qos_manifest_b64: MOCK_BOOT_PROOF_QOS_MANIFEST.to_string(),
        qos_manifest_envelope_b64: MOCK_BOOT_PROOF_QOS_MANIFEST_ENV.to_string(),
        ephemeral_public_key_hex: MOCK_BOOT_PROOF_EPHEMERAL_PK.to_string(),
        enclave_app: "visualsign-parser".to_string(),
        deployment_label: "local-mock".to_string(),
    }
}

/// Gateway-local error envelope: always the stable mock boot proof. The gateway
/// only runs in non-TEE local dev and CI, so it never has a real one.
fn error_response(msg: String) -> TurnkeyResponseWrapper {
    turnkey_error_response(msg, mock_boot_proof())
}

const PARSE_TIMEOUT: Duration = Duration::from_secs(30);

/// True if `payload`'s digests match what parser_app would compute for
/// `unsigned_payload` and `metadata_bytes` -- i.e. the response actually
/// answers this request, not a different one. Mirrors the digest
/// computation in `parser/app/src/routes/parse.rs::parse_with_registry`
/// byte-for-byte; keep the two in sync.
///
/// A valid signature over `payload` (checked separately by
/// `AttestationVerifier::verify`) only proves the pinned key produced it, not
/// that it answers this request -- without this check, a compromised gRPC
/// hop could replay a validly-signed response captured for a different
/// request and it would still verify.
fn response_matches_request(
    payload: &ParsedTransactionPayload,
    unsigned_payload: &str,
    metadata_bytes: &[u8],
) -> bool {
    let expected_input_digest = qos_hex::encode(&sha_256(unsigned_payload.as_bytes()));
    let expected_metadata_digest = qos_hex::encode(&sha_256(metadata_bytes));
    payload.input_payload_digest == expected_input_digest
        && payload.metadata_digest == expected_metadata_digest
}

pub async fn parse_handler(
    State(AppState {
        mut grpc_client,
        attestation,
        ..
    }): State<AppState>,
    Json(wrapper): Json<TurnkeyRequestWrapper>,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    let chain = match Chain::from_str_name(&wrapper.request.chain) {
        Some(c) => c as i32,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(format!(
                    "unknown chain: {}",
                    wrapper.request.chain
                ))),
            );
        }
    };

    let chain_metadata = wrapper.request.chain_metadata.map(ChainMetadata::from);
    // Bytes parser_app hashes into `metadata_digest` for this exact request
    // (see parser/app/src/routes/parse.rs). Recomputed after the response
    // comes back so the verified payload can be bound to *this* request
    // rather than just to itself -- see the attestation check below.
    let metadata_bytes = match chain_metadata.as_ref() {
        Some(metadata) => match borsh::to_vec(metadata) {
            Ok(bytes) => bytes,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(format!(
                        "failed to serialize chain metadata: {e}"
                    ))),
                );
            }
        },
        None => Vec::new(),
    };
    let unsigned_payload = wrapper.request.unsigned_payload;

    let request = tonic::Request::new(ParseRequest {
        unsigned_payload: unsigned_payload.clone(),
        chain,
        chain_metadata,
        include_intermediate_output: wrapper.request.include_intermediate_output,
    });

    let response = match tokio::time::timeout(PARSE_TIMEOUT, grpc_client.parse(request)).await {
        Ok(Ok(r)) => r.into_inner(),
        Ok(Err(e)) => {
            let (http_status, msg) = match e.code() {
                tonic::Code::InvalidArgument => (StatusCode::BAD_REQUEST, e.message().to_string()),
                tonic::Code::NotFound => (StatusCode::NOT_FOUND, e.message().to_string()),
                _ => {
                    eprintln!("gRPC error: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal error".to_string(),
                    )
                }
            };
            return (http_status, Json(error_response(msg)));
        }
        Err(_) => {
            eprintln!("parse RPC timed out after {PARSE_TIMEOUT:?}");
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(error_response("request timed out".to_string())),
            );
        }
    };

    let parsed_tx = match response.parsed_transaction {
        Some(tx) => tx,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(
                    "missing parsed_transaction in response".to_string(),
                )),
            );
        }
    };

    let payload = match parsed_tx.payload {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response("missing payload in response".to_string())),
            );
        }
    };

    // Missing signature from parser_app is the same class of trust failure
    // as a bad signature: surface 502 + don't settle. (502 makes x402-axum's
    // settle-on-success contract treat this as "do not charge".)
    let proto_signature = match parsed_tx.signature {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(error_response("missing signature in response".to_string())),
            );
        }
    };

    // TVC attestation: only forward responses that verifiably came from the
    // pinned enclave key. A 502 here causes x402-axum's settle-on-success
    // contract to skip /settle so payment is not charged for an unattested
    // response.
    if let Some(verifier) = attestation.as_ref() {
        if let Err(e) = verifier.verify(&proto_signature, &payload) {
            eprintln!("attestation verification failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(error_response(format!("attestation failed: {e}"))),
            );
        }

        // Bind the verified response to *this* request; see
        // `response_matches_request` for why.
        if !response_matches_request(&payload, &unsigned_payload, &metadata_bytes) {
            eprintln!("response digest does not match this request");
            return (
                StatusCode::BAD_GATEWAY,
                Json(error_response(
                    "response does not match request".to_string(),
                )),
            );
        }
    }

    let scheme = match proto_signature.scheme {
        x if x == SignatureScheme::TurnkeyP256EphemeralKey as i32 => {
            SignatureScheme::TurnkeyP256EphemeralKey
        }
        _ => SignatureScheme::Unspecified,
    };
    let signature = Some(TurnkeySignature {
        message: proto_signature.message,
        public_key: proto_signature.public_key,
        scheme: scheme.as_str_name().to_string(),
        signature: proto_signature.signature,
    });

    (
        StatusCode::OK,
        Json(turnkey_success_response(
            mock_boot_proof(),
            TurnkeyPayload {
                signable_payload: payload.parsed_payload,
                metadata_digest: payload.metadata_digest,
                input_payload_digest: payload.input_payload_digest,
                // base64 of an empty Vec is "", which serde omits (see
                // skip_serializing_if) so the non-intermediate response
                // is unchanged.
                intermediate_output: base64::engine::general_purpose::STANDARD
                    .encode(&payload.intermediate_output),
            },
            signature,
        )),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{Abi, AbiType, EthereumMetadata, SolanaMetadata};
    use host_primitives::turnkey::{ChainMetadataInput, EMPTY_SHA256};

    fn sample_response_payload(
        input_payload_digest: String,
        metadata_digest: String,
    ) -> ParsedTransactionPayload {
        ParsedTransactionPayload {
            parsed_payload: "{}".to_string(),
            input_payload_digest,
            metadata_digest,
            signable_payload: "{}".to_string(),
            intermediate_output: Vec::new(),
        }
    }

    #[test]
    fn response_matches_request_accepts_matching_digests() {
        let unsigned_payload = "0xdeadbeef";
        let metadata_bytes = b"some-metadata";
        let payload = sample_response_payload(
            qos_hex::encode(&sha_256(unsigned_payload.as_bytes())),
            qos_hex::encode(&sha_256(metadata_bytes)),
        );
        assert!(response_matches_request(
            &payload,
            unsigned_payload,
            metadata_bytes
        ));
    }

    #[test]
    fn response_matches_request_rejects_replay_of_a_different_requests_payload() {
        // Simulates a compromised gRPC hop returning a validly-signed
        // response that was actually produced for a different request's
        // unsigned_payload -- the core replay this check closes.
        let unsigned_payload = "0xdeadbeef";
        let metadata_bytes = b"some-metadata";
        let payload = sample_response_payload(
            qos_hex::encode(&sha_256(b"0xsomeoneelsesrequest")),
            qos_hex::encode(&sha_256(metadata_bytes)),
        );
        assert!(!response_matches_request(
            &payload,
            unsigned_payload,
            metadata_bytes
        ));
    }

    #[test]
    fn response_matches_request_rejects_metadata_mismatch() {
        let unsigned_payload = "0xdeadbeef";
        let payload = sample_response_payload(
            qos_hex::encode(&sha_256(unsigned_payload.as_bytes())),
            qos_hex::encode(&sha_256(b"someone-elses-metadata")),
        );
        assert!(!response_matches_request(
            &payload,
            unsigned_payload,
            b"some-metadata"
        ));
    }

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
    fn intermediate_output_present_serializes_as_camelcase_base64() {
        let payload = TurnkeyPayload {
            signable_payload: "sp".to_string(),
            metadata_digest: "md".to_string(),
            input_payload_digest: "ipd".to_string(),
            intermediate_output: "AQID".to_string(), // base64 of [1,2,3]
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            value.get("intermediateOutput").and_then(|v| v.as_str()),
            Some("AQID"),
            "non-empty intermediate output must serialize under the camelCase key"
        );
    }

    #[test]
    fn error_response_carries_mock_boot_proof() {
        // The wallet-integration contract (see issue #337) requires bootProof
        // be present on every response, including parse errors -- strict
        // consumers reject responses missing the field outright.
        let resp = error_response("oops".to_string());
        assert_eq!(
            resp.boot_proof.aws_attestation_doc_b64,
            MOCK_BOOT_PROOF_AWS_DOC
        );
        assert_eq!(resp.boot_proof.enclave_app, "visualsign-parser");
        assert_eq!(resp.boot_proof.deployment_label, "local-mock");
    }

    #[test]
    fn mock_boot_proof_matches_production_wire_shape() {
        // Top-level wire parity: bootProof must sit alongside response, not
        // nested. bootProof's own field set is covered by
        // host_primitives::turnkey::boot_proof_wire_shape_is_exactly_six_camel_case_keys.
        let resp = error_response("x".to_string());
        let value: serde_json::Value = serde_json::to_value(&resp).unwrap();

        let top_keys: std::collections::BTreeSet<_> =
            value.as_object().unwrap().keys().cloned().collect();
        assert!(
            top_keys.contains("bootProof"),
            "missing top-level bootProof"
        );
        assert!(top_keys.contains("response"), "missing top-level response");
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

    #[test]
    fn abi_type_deserializes_from_string_name() {
        let json = r#"{"value":"[]","abiType":"ABI_TYPE_PROXY","implementationAddress":"0x2222222222222222222222222222222222222222"}"#;
        let abi: Abi = serde_json::from_str(json).unwrap();
        assert_eq!(abi.abi_type, Some(AbiType::Proxy as i32));
        assert_eq!(
            abi.implementation_address.as_deref(),
            Some("0x2222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn abi_type_serializes_as_string_name() {
        let abi = Abi {
            value: "[]".to_string(),
            signature: None,
            abi_type: Some(AbiType::Proxy as i32),
            implementation_address: None,
        };
        let value = serde_json::to_value(&abi).unwrap();
        assert_eq!(value.get("abiType").unwrap(), "ABI_TYPE_PROXY");
    }

    #[test]
    fn abi_type_defaults_to_none_when_omitted() {
        let abi: Abi = serde_json::from_str(r#"{"value":"[]"}"#).unwrap();
        assert_eq!(abi.abi_type, None);
    }

    #[test]
    fn abi_type_rejects_unknown_string() {
        let result: Result<Abi, _> =
            serde_json::from_str(r#"{"value":"[]","abiType":"ABI_TYPE_BOGUS"}"#);
        assert!(
            result.is_err(),
            "expected deserialization to fail for unknown AbiType variant"
        );
    }
}
