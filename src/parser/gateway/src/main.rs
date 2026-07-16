// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use base64::Engine as _;
use generated::grpc::health::v1::{
    HealthCheckRequest, health_check_response::ServingStatus, health_client::HealthClient,
};
use generated::parser::{
    Chain, ChainMetadata, EthereumMetadata, ParseRequest, SignatureScheme, SolanaMetadata,
    chain_metadata, parser_service_client::ParserServiceClient,
};
use generated::tonic;
use host_primitives::GRPC_MAX_RECV_MSG_SIZE;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Deserialize)]
struct TurnkeyRequestWrapper {
    request: TurnkeyRequest,
}

/// Tagged representation of chain metadata for unambiguous JSON deserialization.
///
/// The generated `ChainMetadata` uses `serde(untagged)` on the inner oneof enum, which means
/// serde tries Ethereum first. A Solana payload with only `networkId` would be silently
/// decoded as `EthereumMetadata`. This wrapper uses an explicit `chain` discriminator.
#[derive(Deserialize)]
#[serde(tag = "chain", rename_all = "camelCase")]
enum ChainMetadataInput {
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

#[derive(Deserialize)]
struct TurnkeyRequest {
    unsigned_payload: String,
    chain: String,
    chain_metadata: Option<ChainMetadataInput>,
    /// Opt-in for the chain-specific `intermediate_output` blob. Defaults to
    /// false so existing REST callers that omit it behave exactly as before.
    #[serde(default)]
    include_intermediate_output: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeyResponseWrapper {
    /// Top-level boot proof, matching the production Turnkey visualsign API
    /// response shape that wallet integrators consume. parser_gateway always
    /// emits a stable mock here — the gateway is only used in non-TEE local
    /// dev/CI, never wrapping a real enclave, so production deployments never
    /// see these values. Downstream consumers that perform real attestation
    /// verification will reject the mock, which is correct: this is for
    /// contract-shape testing (the field must be present), not for letting
    /// signing actually succeed. See issue #337.
    boot_proof: TurnkeyBootProof,
    response: TurnkeyResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Boot proof object shape, matching the production Turnkey visualsign API
/// that wallet integrators consume. The reference Go client uses the same
/// field names — see [visualsign-turnkeyclient/api/types.go::TurnkeyBootProof][types].
///
/// [types]: https://github.com/anchorageoss/visualsign-turnkeyclient/blob/main/api/types.go#L128
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeyBootProof {
    aws_attestation_doc_b64: String,
    qos_manifest_b64: String,
    qos_manifest_envelope_b64: String,
    ephemeral_public_key_hex: String,
    enclave_app: String,
    deployment_label: String,
}

/// Stable mock used in every gateway response. The base64 sentinels decode to
/// "TURNKEY_GATEWAY_MOCK_BOOT_PROOF" and "TURNKEY_GATEWAY_MOCK_QOS_MANIFEST*" —
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeyResponse {
    parsed_transaction: TurnkeyParsedTransaction,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeyParsedTransaction {
    payload: TurnkeyPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<TurnkeySignature>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeyPayload {
    signable_payload: String,
    metadata_digest: String,
    input_payload_digest: String,
    /// Chain-specific, borsh-serialized structured decode, base64-encoded (proto
    /// `bytes` JSON convention). Empty and omitted from the response when the
    /// request did not opt in or the chain has no intermediate output, so
    /// responses to existing consumers stay byte-identical.
    #[serde(skip_serializing_if = "String::is_empty")]
    intermediate_output: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnkeySignature {
    message: String,
    public_key: String,
    scheme: String,
    signature: String,
}

type GrpcClient = ParserServiceClient<tonic::transport::Channel>;

#[derive(Clone)]
struct AppState {
    grpc_client: GrpcClient,
    health_client: HealthClient<tonic::transport::Channel>,
}

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const PARSE_TIMEOUT: Duration = Duration::from_secs(30);

async fn health_handler(
    State(AppState {
        mut health_client, ..
    }): State<AppState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let request = tonic::Request::new(HealthCheckRequest {
        service: health_check::DEFAULT_SERVICE.to_string(),
    });
    match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, health_client.check(request)).await {
        Ok(Ok(resp)) => {
            let status = resp.into_inner().status;
            if status == ServingStatus::Serving as i32 {
                (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
            } else {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(
                        serde_json::json!({"status": "unhealthy", "reason": "grpc service not serving"}),
                    ),
                )
            }
        }
        Ok(Err(e)) => {
            eprintln!("health check failed: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"status": "unhealthy", "reason": "backend unavailable"})),
            )
        }
        Err(_) => {
            eprintln!("health check timed out after {HEALTH_CHECK_TIMEOUT:?}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(
                    serde_json::json!({"status": "unhealthy", "reason": "health check timed out"}),
                ),
            )
        }
    }
}

async fn parse_handler(
    State(AppState {
        mut grpc_client, ..
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

    let request = tonic::Request::new(ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
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

    let signature = parsed_tx.signature.map(|sig| {
        let scheme = match sig.scheme {
            x if x == SignatureScheme::TurnkeyP256EphemeralKey as i32 => {
                SignatureScheme::TurnkeyP256EphemeralKey
            }
            _ => SignatureScheme::Unspecified,
        };
        let scheme_str = scheme.as_str_name();
        TurnkeySignature {
            message: sig.message,
            public_key: sig.public_key,
            scheme: scheme_str.to_string(),
            signature: sig.signature,
        }
    });

    (
        StatusCode::OK,
        Json(TurnkeyResponseWrapper {
            boot_proof: mock_boot_proof(),
            response: TurnkeyResponse {
                parsed_transaction: TurnkeyParsedTransaction {
                    payload: TurnkeyPayload {
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
                },
            },
            error: None,
        }),
    )
}

// SHA-256 of empty input: used as the canonical "no data" sentinel for digest fields.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn error_response(msg: String) -> TurnkeyResponseWrapper {
    TurnkeyResponseWrapper {
        boot_proof: mock_boot_proof(),
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = match std::env::var("GATEWAY_PORT") {
        Ok(val) => val.parse().unwrap_or_else(|_| {
            eprintln!("WARNING: invalid GATEWAY_PORT value '{val}', falling back to 8080");
            8080
        }),
        Err(_) => 8080,
    };

    let grpc_addr =
        std::env::var("GRPC_ADDR").unwrap_or_else(|_| "http://127.0.0.1:44020".to_string());

    let endpoint = tonic::transport::Endpoint::from_shared(grpc_addr.clone())
        .map_err(|e| format!("invalid gRPC address {grpc_addr}: {e}"))?;
    let channel = endpoint.connect_lazy();
    let grpc_client = ParserServiceClient::new(channel.clone())
        .max_decoding_message_size(GRPC_MAX_RECV_MSG_SIZE)
        .max_encoding_message_size(GRPC_MAX_RECV_MSG_SIZE);
    let health_client = HealthClient::new(channel);

    let state = AppState {
        grpc_client,
        health_client,
    };

    // Mount the same handler under both v1 and v2 URLs for local parity with
    // the production Turnkey visualsign API, which serves /api/v1/parse and
    // /api/v2/parse from the same backend. The response shape already carries
    // both v1 fields (signablePayload) and v2 fields (metadataDigest,
    // inputPayloadDigest) since #287, so a single handler covers both.
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/visualsign/api/v1/parse", post(parse_handler))
        .route("/visualsign/api/v2/parse", post(parse_handler))
        .layer(DefaultBodyLimit::max(GRPC_MAX_RECV_MSG_SIZE))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("parser_gateway {} listening on {addr}", env!("VERSION"));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await.expect("failed to listen for ctrl-c");

    println!("Shutting down gateway");
}

#[cfg(test)]
mod tests {
    use super::*;
    use generated::parser::{Abi, AbiType, EthereumMetadata, SolanaMetadata};

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
    fn intermediate_output_empty_is_omitted() {
        // Existing consumers and existing-parser responses must see byte-identical
        // JSON when there is no intermediate output: the key is absent, not "".
        let payload = TurnkeyPayload {
            signable_payload: "sp".to_string(),
            metadata_digest: "md".to_string(),
            input_payload_digest: "ipd".to_string(),
            intermediate_output: String::new(),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert!(
            value.get("intermediateOutput").is_none(),
            "empty intermediate output must be omitted from the response"
        );
    }

    #[test]
    fn error_response_carries_mock_boot_proof() {
        // The wallet-integration contract (see issue #337) requires bootProof
        // be present on every response, including parse errors — strict
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
        // Wire-shape parity with the production response that wallet
        // integrators consume. Field set and JSON keys mirror
        // visualsign-turnkeyclient/api/types.go: TurnkeyVisualSignResponse
        // (bootProof at top level) and TurnkeyBootProof (six camelCase keys).
        let resp = error_response("x".to_string());
        let value: serde_json::Value = serde_json::to_value(&resp).unwrap();

        let top_keys: std::collections::BTreeSet<_> =
            value.as_object().unwrap().keys().cloned().collect();
        // Top-level: bootProof, response, error (error only present when set).
        assert!(
            top_keys.contains("bootProof"),
            "missing top-level bootProof"
        );
        assert!(top_keys.contains("response"), "missing top-level response");

        let bp = value.get("bootProof").unwrap().as_object().unwrap();
        let bp_keys: std::collections::BTreeSet<&str> = bp.keys().map(String::as_str).collect();
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
            bp_keys, expected,
            "bootProof field set must match production wire shape exactly"
        );
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
