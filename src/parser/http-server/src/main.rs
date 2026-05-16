// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! HTTP+JSON server wrapping `parser_app::routes::parse::parse` — the
//! single-binary variant intended for Turnkey TVC deployment.
//!
//! Turnkey's TVC public ingress accepts HTTP only — Cloudflare in front of
//! `app-<uuid>.turnkey.cloud` rejects gRPC with 403 (verified 2026-05-16).
//! So the binary deployed as the TVC pivot must speak HTTP+JSON natively.
//! parser_app's gRPC interface remains how vsock IPC happens internally;
//! this binary is the public face.
//!
//! Routes:
//! - `GET /health` — 200 OK for Turnkey's HTTP health check
//!   (`healthCheckType: TVC_HEALTH_CHECK_TYPE_HTTP`).
//! - `POST /visualsign/api/v1/parse` — Turnkey-envelope JSON in/out.
//!   Mirrors `parser_gateway`'s v1 route exactly; the Turnkey wire
//!   envelope types are reused from `host_primitives::turnkey` so the Go
//!   visualsign-turnkey-client (and any HTTP-only client) keeps working
//!   byte-for-byte.
//! - `POST /visualsign/api/v2/parse` — same payload; additionally
//!   enforces `GATEWAY_SIGNING_PUBKEY_HEX` when set (TVC-enforced mode
//!   from plan v3). When unset, behaves the same as v1.
//!
//! Env vars:
//! - `HTTP_PORT` (default: 3000) — Turnkey TVC public ingress default.
//! - `EPHEMERAL_FILE` (required) — path to the parser_app ephemeral key.
//! - `GATEWAY_SIGNING_PUBKEY_HEX` (optional) — pinned gateway P256 sign
//!   pubkey for VPM verification on v2.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use generated::parser::{Chain, ChainMetadata, SignatureScheme};
use host_primitives::turnkey::{
    TurnkeyParsedTransaction, TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponse,
    TurnkeyResponseWrapper, TurnkeySignature, error_response,
};
use parser_app::payment_verify::PaymentPolicy;
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    ephemeral_key: Arc<P256Pair>,
    /// Disabled when GATEWAY_SIGNING_PUBKEY_HEX is unset. The v1 route
    /// always passes `Disabled`; the v2 route uses this policy.
    policy: Arc<PaymentPolicy>,
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn parse_v1(
    State(state): State<AppState>,
    Json(wrapper): Json<TurnkeyRequestWrapper>,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    handle_parse(&state, wrapper, &PaymentPolicy::Disabled)
}

async fn parse_v2(
    State(state): State<AppState>,
    Json(wrapper): Json<TurnkeyRequestWrapper>,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    handle_parse(&state, wrapper, state.policy.as_ref())
}

fn handle_parse(
    state: &AppState,
    wrapper: TurnkeyRequestWrapper,
    policy: &PaymentPolicy,
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

    // TVC-enforced mode threads the VPM through the proto field. Open
    // v1 callers never set this, so empty vec is fine.
    let proto_req = generated::parser::ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
        payment_marker: Vec::new(),
    };

    let proto_resp = match parse(&proto_req, &state.ephemeral_key, policy) {
        Ok(r) => r,
        Err(e) => {
            let http_status = match e.code {
                generated::google::rpc::Code::InvalidArgument => StatusCode::BAD_REQUEST,
                generated::google::rpc::Code::NotFound => StatusCode::NOT_FOUND,
                generated::google::rpc::Code::FailedPrecondition => StatusCode::PAYMENT_REQUIRED,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return (http_status, Json(error_response(e.message)));
        }
    };

    let parsed_tx = match proto_resp.parsed_transaction {
        Some(tx) => tx,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(
                    "parser_app returned no parsed_transaction".to_string(),
                )),
            );
        }
    };
    let payload = match parsed_tx.payload {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response("parser_app returned no payload".to_string())),
            );
        }
    };
    let signature = parsed_tx.signature.map(|sig| {
        let scheme = SignatureScheme::from_i32(sig.scheme).unwrap_or(SignatureScheme::Unspecified);
        TurnkeySignature {
            message: sig.message,
            public_key: sig.public_key,
            scheme: scheme.as_str_name().to_string(),
            signature: sig.signature,
        }
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("HTTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);

    let ephemeral_file = std::env::var("EPHEMERAL_FILE")
        .unwrap_or_else(|_| "integration/fixtures/ephemeral.secret".to_string());
    let handle = EphemeralKeyHandle::new(ephemeral_file.clone());
    let ephemeral_key = handle
        .get_ephemeral_key()
        .expect("failed to load ephemeral key");
    eprintln!(
        "parser_http_server {} loaded ephemeral key from {ephemeral_file}",
        env!("VERSION")
    );

    let policy =
        PaymentPolicy::from_env().expect("invalid GATEWAY_SIGNING_PUBKEY_HEX configuration");
    if matches!(policy, PaymentPolicy::Required { .. }) {
        eprintln!("v2 route is TVC-enforced (GATEWAY_SIGNING_PUBKEY_HEX is set)");
    } else {
        eprintln!("v2 route is open (GATEWAY_SIGNING_PUBKEY_HEX unset)");
    }

    let state = AppState {
        ephemeral_key: Arc::new(ephemeral_key),
        policy: Arc::new(policy),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/visualsign/api/v1/parse", post(parse_v1))
        .route("/visualsign/api/v2/parse", post(parse_v2))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    eprintln!("parser_http_server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
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
    eprintln!("parser_http_server shutting down");
}
