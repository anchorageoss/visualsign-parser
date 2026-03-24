use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    extract::State,
    routing::{get, post},
};
use generated::parser::{
    Chain, ParseRequest, SignatureScheme, parser_service_client::ParserServiceClient,
};
use generated::tonic;
use host_primitives::GRPC_MAX_RECV_MSG_SIZE;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

// --- Turnkey JSON types (matching Go client's format) ---

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

// --- Handler ---

type GrpcClient = ParserServiceClient<tonic::transport::Channel>;

#[derive(Clone)]
struct AppState {
    grpc_client: GrpcClient,
    grpc_addr: Arc<str>,
}

async fn health_handler(
    State(state): State<AppState>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let tcp_addr = match state.grpc_addr.parse::<axum::http::Uri>() {
        Ok(uri) => uri
            .authority()
            .map(|a| a.to_string())
            .unwrap_or_else(|| state.grpc_addr.to_string()),
        Err(_) => state.grpc_addr.to_string(),
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(tcp_addr),
    )
    .await
    {
        Ok(Ok(_)) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"status": "ok"})),
        ),
        _ => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "unhealthy", "reason": "grpc backend unreachable"})),
        ),
    }
}

async fn parse_handler(
    State(AppState {
        mut grpc_client, ..
    }): State<AppState>,
    Json(wrapper): Json<TurnkeyRequestWrapper>,
) -> (axum::http::StatusCode, Json<TurnkeyResponseWrapper>) {
    let chain = match Chain::from_str_name(&wrapper.request.chain) {
        Some(c) => c as i32,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
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

    let response = match grpc_client.parse(request).await {
        Ok(r) => r.into_inner(),
        Err(e) => {
            let http_status = match e.code() {
                tonic::Code::InvalidArgument => axum::http::StatusCode::BAD_REQUEST,
                tonic::Code::NotFound => axum::http::StatusCode::NOT_FOUND,
                _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            };
            return (
                http_status,
                Json(error_response(format!("gRPC error: {e}"))),
            );
        }
    };

    let parsed_tx = match response.parsed_transaction {
        Some(tx) => tx,
        None => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
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
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
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
        axum::http::StatusCode::OK,
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

// --- Server startup ---

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

    let client = ParserServiceClient::connect(grpc_addr.clone())
        .await
        .map_err(|e| format!("failed to connect to gRPC server at {grpc_addr}: {e}"))?
        .max_decoding_message_size(GRPC_MAX_RECV_MSG_SIZE)
        .max_encoding_message_size(GRPC_MAX_RECV_MSG_SIZE);

    let state = AppState {
        grpc_client: client,
        grpc_addr: Arc::from(grpc_addr.as_str()),
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
        .await?;

    Ok(())
}
