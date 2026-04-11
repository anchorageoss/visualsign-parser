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
use generated::grpc::health::v1::{
    HealthCheckRequest, health_check_response::ServingStatus, health_client::HealthClient,
};
use generated::parser::{
    Chain, ParseRequest, SignatureScheme, parser_service_client::ParserServiceClient,
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

#[derive(Deserialize)]
struct TurnkeyRequest {
    unsigned_payload: String,
    chain: String,
}

#[derive(Serialize)]
struct TurnkeyResponseWrapper {
    response: TurnkeyResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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
        chain_metadata: None,
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
            response: TurnkeyResponse {
                parsed_transaction: TurnkeyParsedTransaction {
                    payload: TurnkeyPayload {
                        signable_payload: payload.parsed_payload,
                    },
                    signature,
                },
            },
            error: None,
        }),
    )
}

fn error_response(msg: String) -> TurnkeyResponseWrapper {
    TurnkeyResponseWrapper {
        response: TurnkeyResponse {
            parsed_transaction: TurnkeyParsedTransaction {
                payload: TurnkeyPayload {
                    signable_payload: String::new(),
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

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/visualsign/api/v1/parse", post(parse_handler))
        .layer(DefaultBodyLimit::max(GRPC_MAX_RECV_MSG_SIZE))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("Gateway listening on {addr}");
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
