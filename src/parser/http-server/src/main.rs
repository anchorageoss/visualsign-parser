// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! HTTP+JSON server wrapping `parser_app::routes::parse::parse` - the
//! single-binary variant intended for Turnkey TVC deployment.
//!
//! Turnkey's TVC public ingress accepts HTTP only - Cloudflare in front of
//! `app-<uuid>.turnkey.cloud` rejects gRPC with 403 (verified 2026-05-16).
//! So the binary deployed as the TVC pivot must speak HTTP+JSON natively.
//! parser_app's gRPC interface remains how vsock IPC happens internally;
//! this binary is the public face.
//!
//! Routes:
//! - `GET /health` - 200 OK for Turnkey's HTTP health check
//!   (`healthCheckType: TVC_HEALTH_CHECK_TYPE_HTTP`).
//! - `POST /visualsign/api/v1/parse` - Turnkey-envelope JSON in/out.
//!   Mirrors `parser_gateway`'s v1 route exactly; the Turnkey wire
//!   envelope types are reused from `host_primitives::turnkey` so the Go
//!   visualsign-turnkey-client (and any HTTP-only client) keeps working
//!   byte-for-byte.
//! - `POST /visualsign/api/v2/parse` - same payload. Open in this PR; a
//!   later PR adds X-Stamp enforcement and payment enforcement here.
//!
//! Configuration (CLI args; env vars listed are clap fallbacks):
//! - `--port <u16>` / `HTTP_PORT` (default 3000) - Turnkey TVC public ingress.
//! - `--enclave-app <name>` / `ENCLAVE_APP` (default `visualsign-parser`).
//! - `--deployment-label <label>` / `DEPLOYMENT_LABEL`.
//!
//! The ephemeral key is read from `qos_core::EPHEMERAL_KEY_FILE` (provisioned
//! by QOS inside the enclave). No override flag - if a deployment ever needs
//! a non-canonical path, bind-mount it instead.

mod boot_proof;
mod stamp;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::Engine as _;
use boot_proof::{BootProofSource, StaticBootProof};
use clap::Parser;
use generated::parser::{Chain, ChainMetadata, SignatureScheme};
use host_primitives::turnkey::{
    TurnkeyParsedTransaction, TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponse,
    TurnkeyResponseWrapper, TurnkeySignature, error_response,
};
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
use stamp::Allowlist;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(version = env!("VERSION"))]
struct Args {
    /// HTTP port to listen on.
    #[arg(long, env = "HTTP_PORT", default_value_t = 3000)]
    port: u16,

    /// Enclave app identifier reported in every response's `bootProof`.
    #[arg(long, env = "ENCLAVE_APP", default_value = "visualsign-parser")]
    enclave_app: String,

    /// Deployment label reported in every response's `bootProof`.
    #[arg(long, env = "DEPLOYMENT_LABEL", default_value = "")]
    deployment_label: String,

    /// Comma-separated compressed SEC1 hex pubkeys allowed to call the parse
    /// routes. Absent means the routes stay open (today's behavior);
    /// present means every request must carry a valid X-Stamp from a listed
    /// key. Delivered via `pivotArgs` at deploy time.
    #[arg(long, env = "ALLOWED_STAMP_PUBKEYS_HEX")]
    allowed_stamp_pubkeys_hex: Option<String>,
}

#[derive(Clone)]
struct AppState {
    ephemeral_key: Arc<P256Pair>,
    boot_proof: Arc<dyn BootProofSource + Send + Sync>,
    allowlist: Option<Arc<Allowlist>>,
}

async fn health() -> StatusCode {
    StatusCode::OK
}

// Handlers take raw bytes, never `Json<T>`. The X-Stamp signature is verified
// against the exact request bytes; a `Json<T>` extractor re-serializes
// before the handler body runs, changing key order / whitespace / unicode
// escaping and invalidating every signature. Both routes share one body.
// `headers` comes before `body` because axum requires body-consuming
// extractors last.
async fn parse_v1(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    handle_parse(&state, &headers, &body)
}

/// v2 is byte-identical to v1 in this PR. Registering it now keeps the
/// deployed URL stable across the stack as later PRs add payment enforcement here.
async fn parse_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    handle_parse(&state, &headers, &body)
}

/// Deserialize the envelope from the original bytes. Kept separate so the
/// caller still owns the untouched slice (see the X-Stamp seam test).
fn parse_envelope(body: &[u8]) -> Result<TurnkeyRequestWrapper, serde_json::Error> {
    serde_json::from_slice(body)
}

fn handle_parse(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    if let Some(allowlist) = state.allowlist.as_deref() {
        if let Err(e) = stamp::verify(headers, body, allowlist) {
            eprintln!("rejected request: {e:?}");
            // Deliberately coarse: the client learns "not authenticated", not
            // which check failed, so the error text cannot be used to probe
            // the allowlist.
            return (
                StatusCode::UNAUTHORIZED,
                Json(error_response(
                    "invalid or missing X-Stamp".to_string(),
                    state.boot_proof.boot_proof(),
                )),
            );
        }
    }

    let wrapper = match parse_envelope(body) {
        Ok(w) => w,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(
                    format!("invalid request body: {e}"),
                    state.boot_proof.boot_proof(),
                )),
            );
        }
    };

    let chain = match Chain::from_str_name(&wrapper.request.chain) {
        Some(c) => c as i32,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_response(
                    format!("unknown chain: {}", wrapper.request.chain),
                    state.boot_proof.boot_proof(),
                )),
            );
        }
    };

    let proto_req = generated::parser::ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
        include_intermediate_output: wrapper.request.include_intermediate_output,
    };

    let proto_resp = match parse(&proto_req, &state.ephemeral_key) {
        Ok(r) => r,
        Err(e) => {
            let http_status = match e.code {
                generated::google::rpc::Code::InvalidArgument => StatusCode::BAD_REQUEST,
                generated::google::rpc::Code::NotFound => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return (
                http_status,
                Json(error_response(e.message, state.boot_proof.boot_proof())),
            );
        }
    };

    let parsed_tx = match proto_resp.parsed_transaction {
        Some(tx) => tx,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(
                    "parser_app returned no parsed_transaction".to_string(),
                    state.boot_proof.boot_proof(),
                )),
            );
        }
    };
    let payload = match parsed_tx.payload {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(
                    "parser_app returned no payload".to_string(),
                    state.boot_proof.boot_proof(),
                )),
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
            boot_proof: state.boot_proof.boot_proof(),
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let handle = EphemeralKeyHandle::new(qos_core::EPHEMERAL_KEY_FILE.to_string());
    let ephemeral_key = handle
        .get_ephemeral_key()
        .expect("failed to load ephemeral key");
    eprintln!(
        "parser_http_server {} loaded ephemeral key from {}",
        env!("VERSION"),
        qos_core::EPHEMERAL_KEY_FILE,
    );

    let boot_proof = StaticBootProof::from_enclave_files(
        &ephemeral_key,
        args.enclave_app,
        args.deployment_label,
    );

    // Absent means the routes stay open (today's behavior); present means
    // every request must carry a valid X-Stamp from a listed key.
    let allowlist = args
        .allowed_stamp_pubkeys_hex
        .map(|csv| Allowlist::from_hex_list(&csv).expect("invalid --allowed-stamp-pubkeys-hex"))
        .map(Arc::new);

    let state = AppState {
        ephemeral_key: Arc::new(ephemeral_key),
        boot_proof: Arc::new(boot_proof),
        allowlist,
    };

    // 64 KiB caps every parse-request body the TVC pivot will accept.
    // axum's default is 2 MiB; a real parse envelope is hundreds of bytes,
    // and accepting more lets an attacker force expensive sync parsing
    // (block_in_place) on the enclave's CPU per call. Same cap as the
    // gateway in front of us, so a properly-formed request that passes the
    // gateway can't be rejected here.
    const PIVOT_BODY_LIMIT_BYTES: usize = 64 * 1024;
    let app = Router::new()
        .route("/health", get(health))
        .route("/visualsign/api/v1/parse", post(parse_v1))
        .route("/visualsign/api/v2/parse", post(parse_v2))
        .layer(axum::extract::DefaultBodyLimit::max(PIVOT_BODY_LIMIT_BYTES))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn envelope_is_parsed_from_raw_bytes_not_reserialized() {
        // A later PR verifies an X-Stamp signature over the exact request
        // bytes. If a handler ever takes `Json<T>` and re-serializes, the
        // bytes change (key order, whitespace, unicode escaping) and every
        // stamp fails. Locking the seam here means that PR adds one call and
        // no signature churn.
        let raw = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02","include_intermediate_output":false}}"#;
        let parsed = parse_envelope(raw).unwrap();
        assert_eq!(parsed.request.chain, "CHAIN_ETHEREUM");
        // Re-serializing must NOT be how we get bytes back: prove they differ,
        // so a future refactor that leans on serde output gets caught.
        let reserialized = serde_json::to_vec(&parsed).unwrap();
        assert_ne!(
            reserialized.as_slice(),
            raw.as_slice(),
            "serde round-trip changes the bytes; verification must use the original slice"
        );
    }

    #[test]
    fn static_boot_proof_has_the_six_keys_and_a_real_ephemeral_pubkey() {
        let pair = qos_p256::P256Pair::generate().unwrap();
        let expected_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let source = StaticBootProof::from_enclave_files(
            &pair,
            "visualsign-parser".to_string(),
            "test".to_string(),
        );
        let bp = source.boot_proof();
        assert_eq!(bp.ephemeral_public_key_hex, expected_hex);
        // A later PR fills the doc; until then it is explicitly empty, never a fake.
        assert!(bp.aws_attestation_doc_b64.is_empty());
        let value = serde_json::to_value(&bp).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 6);
    }
}
