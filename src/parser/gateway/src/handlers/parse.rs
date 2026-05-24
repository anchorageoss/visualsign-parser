//! Shared parse handler. Used by both /visualsign/api/v1/parse (open, Turnkey)
//! and /visualsign/api/v2/parse (x402-gated).

use crate::state::AppState;
use crate::turnkey::{
    TurnkeyParsedTransaction, TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponse,
    TurnkeyResponseWrapper, TurnkeySignature, error_response,
};
use axum::{Json, extract::State, http::StatusCode};
use generated::parser::{Chain, ChainMetadata, ParseRequest, SignatureScheme};
use generated::tonic;
use std::time::Duration;

const PARSE_TIMEOUT: Duration = Duration::from_secs(30);

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

    let request = tonic::Request::new(ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
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
    if let Some(verifier) = attestation.as_ref()
        && let Err(e) = verifier.verify(&proto_signature)
    {
        eprintln!("attestation verification failed: {e}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(error_response(format!("attestation failed: {e}"))),
        );
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
        Json(TurnkeyResponseWrapper {
            response: TurnkeyResponse {
                parsed_transaction: TurnkeyParsedTransaction {
                    payload: TurnkeyPayload {
                        signable_payload: payload.parsed_payload,
                        metadata_digest: payload.metadata_digest,
                        input_payload_digest: payload.input_payload_digest,
                    },
                    signature,
                },
            },
            error: None,
        }),
    )
}
