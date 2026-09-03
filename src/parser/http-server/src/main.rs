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
//! - `--accept-unsigned-abis` / `--accept-signatures-from-pubkey <hex>` (repeatable) -
//!   required, exactly one: the deploy-time trust posture for caller-supplied Ethereum
//!   ABI mappings, same posture and same requirement as `parser_app` (see
//!   `ParserConfig::abi_trust_from_options`). No env fallback: these land in this
//!   deployment's signed `pivotArgs`, and an env escape hatch would undermine that.
//!
//! The ephemeral key is read from `qos_core::EPHEMERAL_KEY_FILE` (provisioned
//! by QOS inside the enclave). No override flag - if a deployment ever needs
//! a non-canonical path, bind-mount it instead.

mod boot_proof;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use boot_proof::{BootProofSource, StaticBootProof};
use clap::Parser;
use generated::parser::{Chain, ChainMetadata, SignatureScheme};
use host_primitives::turnkey::{
    TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponseWrapper, TurnkeySignature,
    error_response, success_response,
};
use parser_app::config::ParserConfig;
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
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

    /// Required (exactly one of --accept-unsigned-abis / --accept-signatures-from-pubkey):
    /// accept caller-supplied ABI mappings that carry no signature. Their integrity and
    /// provenance are unverified. Mutually exclusive with --accept-signatures-from-pubkey.
    ///
    /// This binary is the actual TVC public ingress once deployed (parser_app's gRPC
    /// becomes internal vsock IPC), so it carries the same fail-closed posture
    /// requirement as parser_app: the choice must be explicit and land in this
    /// deployment's pivotArgs, not default silently to the permissive posture.
    #[arg(long)]
    accept_unsigned_abis: bool,

    /// Required (exactly one of --accept-unsigned-abis / --accept-signatures-from-pubkey):
    /// only accept caller-supplied ABI mappings signed by this hex secp256k1 public key;
    /// unsigned and otherwise-signed mappings are rejected. Repeatable. Mutually exclusive
    /// with --accept-unsigned-abis. Requires a build with the `ethereum` feature (on by
    /// default); a build without it refuses to start when this flag is given.
    #[arg(long = "accept-signatures-from-pubkey")]
    accept_signatures_from_pubkey: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    ephemeral_key: Arc<P256Pair>,
    boot_proof: Arc<dyn BootProofSource + Send + Sync>,
    config: ParserConfig,
}

async fn health() -> StatusCode {
    StatusCode::OK
}

// Handlers take raw bytes, never `Json<T>`. A later PR verifies an X-Stamp
// signature over the exact request bytes; a `Json<T>` extractor re-serializes
// before the handler body runs, changing key order / whitespace / unicode
// escaping and invalidating every signature. Both routes share one body.
async fn parse_v1(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    // parse() does the full decode/charset-validation/sign path, which is
    // CPU-bound, not I/O-bound. Running it directly on the async task would
    // pin a Tokio worker thread per concurrent request, starving everything
    // else on that worker (including GET /health) on a 1-2 vCPU TVC replica.
    // Matches the block_in_place precedent parser_app::service::Processor::process
    // already uses around this same parse() call on the vsock/gRPC path.
    tokio::task::block_in_place(|| handle_parse(&state, &body))
}

/// v2 is byte-identical to v1 in this PR. Registering it now keeps the
/// deployed URL stable across the stack as later PRs add enforcement here.
async fn parse_v2(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    tokio::task::block_in_place(|| handle_parse(&state, &body))
}

/// Deserialize the envelope from the original bytes. Kept separate so the
/// caller still owns the untouched slice (see the X-Stamp seam test).
fn parse_envelope(body: &[u8]) -> Result<TurnkeyRequestWrapper, serde_json::Error> {
    serde_json::from_slice(body)
}

/// Builds the Turnkey-shaped error envelope shared by every non-2xx response
/// in this binary: `handle_parse`'s own error arms, the 413 rewrite in
/// `envelope_body_limit_rejection`, and the 404/405 fallbacks below.
fn error_status(
    state: &AppState,
    status: StatusCode,
    msg: String,
) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    (
        status,
        Json(error_response(msg, state.boot_proof.boot_proof())),
    )
}

fn handle_parse(state: &AppState, body: &[u8]) -> (StatusCode, Json<TurnkeyResponseWrapper>) {
    // A later PR inserts the X-Stamp check here, before anything else touches `body`.
    let wrapper = match parse_envelope(body) {
        Ok(w) => w,
        Err(e) => {
            // serde_json's Display for a type-mismatch error embeds the
            // offending value verbatim, which would otherwise reflect up to
            // the full request body back to an unauthenticated caller (and,
            // logged as-is, into enclave logs). Log only bounded metadata
            // (error category + position), never the Display string.
            eprintln!(
                "invalid request body: category={:?} line={} column={}",
                e.classify(),
                e.line(),
                e.column()
            );
            return error_status(
                state,
                StatusCode::BAD_REQUEST,
                "invalid request body".to_string(),
            );
        }
    };

    let Some(chain) = Chain::from_str_name(&wrapper.request.chain).map(|c| c as i32) else {
        // `chain` is caller-controlled and unbounded up to the request body
        // limit; logging it verbatim would let an unauthenticated caller
        // forge log lines or amplify enclave logs. Drop the value entirely,
        // matching the other bounded-logging fixes in this file.
        eprintln!("unknown chain requested");
        return error_status(state, StatusCode::BAD_REQUEST, "unknown chain".to_string());
    };

    let proto_req = generated::parser::ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
        include_intermediate_output: wrapper.request.include_intermediate_output,
    };

    let proto_resp = match parse(&proto_req, &state.ephemeral_key, &state.config) {
        Ok(r) => r,
        Err(e) => {
            // Only NotFound carries a message safe to hand back to an
            // unauthenticated caller. `parse()` maps every converter error to
            // InvalidArgument (parser/app/src/routes/parse.rs), and those
            // errors can embed request-controlled data verbatim (e.g. an
            // invalid NEAR networkId, visualsign-near/src/networks.rs), so
            // InvalidArgument is logged server-side and replaced with a fixed
            // message too, matching the other reflection fixes in this file.
            let (http_status, msg) = match e.code {
                generated::google::rpc::Code::InvalidArgument => {
                    eprintln!("parse failed: code={:?}", e.code);
                    (StatusCode::BAD_REQUEST, "invalid request".to_string())
                }
                generated::google::rpc::Code::NotFound => (StatusCode::NOT_FOUND, e.message),
                _ => {
                    eprintln!("parse failed: {} ({:?})", e.message, e.code);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal error".to_string(),
                    )
                }
            };
            return error_status(state, http_status, msg);
        }
    };

    let Some(parsed_tx) = proto_resp.parsed_transaction else {
        eprintln!("parse returned no parsed_transaction");
        return error_status(
            state,
            StatusCode::INTERNAL_SERVER_ERROR,
            "parser_app returned no parsed_transaction".to_string(),
        );
    };
    let Some(payload) = parsed_tx.payload else {
        eprintln!("parse returned no payload");
        return error_status(
            state,
            StatusCode::INTERNAL_SERVER_ERROR,
            "parser_app returned no payload".to_string(),
        );
    };
    let signature = parsed_tx.signature.map(|sig| {
        let scheme = SignatureScheme::try_from(sig.scheme).unwrap_or(SignatureScheme::Unspecified);
        TurnkeySignature {
            message: sig.message,
            public_key: sig.public_key,
            scheme: scheme.as_str_name().to_string(),
            signature: sig.signature,
        }
    });

    (
        StatusCode::OK,
        Json(success_response(
            state.boot_proof.boot_proof(),
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

/// axum's built-in rejection for an oversized body (413) never reaches
/// `handle_parse` - `DefaultBodyLimit` rejects the request while reading it,
/// before any handler runs - so it skips the Turnkey envelope entirely.
/// Keyed on status alone this is safe only because 413 cannot originate from
/// a handler: `handle_parse` never returns `PAYLOAD_TOO_LARGE`. 404 and 405
/// are deliberately NOT handled here, even though axum's default rejections
/// for them have the same gap - `handle_parse` legitimately returns 404
/// itself (`Code::NotFound`, with the parser's real error message), and a
/// status-keyed middleware sitting in front of every handler response cannot
/// tell that apart from an unmatched route. Those two go through
/// `Router::fallback` / `Router::method_not_allowed_fallback` below instead,
/// which axum only invokes when no handler ran at all.
async fn envelope_body_limit_rejection(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return error_status(
            &state,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload too large".to_string(),
        )
        .into_response();
    }
    response
}

/// `Router::fallback` target for unmatched routes. axum only calls this when
/// no route matched, so it never sees `handle_parse`'s own 404s.
async fn not_found_fallback(State(state): State<AppState>) -> Response {
    error_status(&state, StatusCode::NOT_FOUND, "not found".to_string()).into_response()
}

/// `Router::method_not_allowed_fallback` target for a matched path called
/// with an unsupported method.
async fn method_not_allowed_fallback(State(state): State<AppState>) -> Response {
    error_status(
        &state,
        StatusCode::METHOD_NOT_ALLOWED,
        "method not allowed".to_string(),
    )
    .into_response()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Fail closed rather than default: this binary is the TVC public ingress, so an
    // unstated posture here would be worse than on parser_app, not better. See
    // ParserConfig::abi_trust_from_options for why exactly one flag is required.
    let abi_trust = ParserConfig::abi_trust_from_options(
        args.accept_unsigned_abis,
        &args.accept_signatures_from_pubkey,
    )
    .map_err(|e| format!("invalid ABI trust config: {e}"))?;
    eprintln!("caller-supplied ABI trust: {abi_trust}");
    let config = ParserConfig::new(abi_trust);

    let handle = EphemeralKeyHandle::new(qos_core::EPHEMERAL_KEY_FILE.to_string());
    let ephemeral_key = handle
        .get_ephemeral_key()
        .map_err(|e| format!("failed to load ephemeral key: {e}"))?;
    eprintln!(
        "parser_http_server {} loaded ephemeral key from {}",
        env!("VERSION"),
        qos_core::EPHEMERAL_KEY_FILE,
    );

    let boot_proof = StaticBootProof::from_enclave_files(
        &ephemeral_key,
        args.enclave_app,
        args.deployment_label,
    )
    .map_err(|e| format!("failed to build boot proof: {e:?}"))?;

    let state = AppState {
        ephemeral_key: Arc::new(ephemeral_key),
        boot_proof: Arc::new(boot_proof),
        config,
    };

    // 64 KiB caps every parse-request body the TVC pivot will accept.
    // axum's default is 2 MiB; a real parse envelope is hundreds of bytes,
    // and accepting more lets an attacker force expensive sync parsing on
    // the enclave's CPU per call. Deliberately far below the gateway's own
    // cap (`host_primitives::GRPC_MAX_RECV_MSG_SIZE`, 25 MiB), which sizes
    // for gRPC message limits, not this DoS concern.
    const PIVOT_BODY_LIMIT_BYTES: usize = 64 * 1024;
    let app = Router::new()
        .route("/health", get(health))
        .route("/visualsign/api/v1/parse", post(parse_v1))
        .route("/visualsign/api/v2/parse", post(parse_v2))
        .fallback(not_found_fallback)
        .method_not_allowed_fallback(method_not_allowed_fallback)
        .layer(axum::extract::DefaultBodyLimit::max(PIVOT_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            envelope_body_limit_rejection,
        ))
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
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(e) => {
                eprintln!("failed to register SIGTERM handler: {e}; falling back to ctrl-c only");
                if let Err(e) = ctrl_c.await {
                    eprintln!("failed to listen for ctrl-c: {e}");
                }
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(e) = ctrl_c.await {
        eprintln!("failed to listen for ctrl-c: {e}");
    }
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

    // Handler-level regression pin, complementing the test above: that one only
    // calls `parse_envelope` directly and never touches `parse_v1`/`parse_v2`'s
    // own parameter type, so it would keep passing even if a future change swapped
    // `body: axum::body::Bytes` for `body: Json<TurnkeyRequestWrapper>` there - axum's
    // `Json` extractor deserializes and re-serializes before the handler body runs,
    // which is exactly the byte-changing round trip the seam exists to prevent (see
    // the module doc on `parse_v1`). Passing a `Bytes` value as the `body` argument
    // here means that swap would fail to *compile*, catching the regression at build
    // time rather than needing a runtime assertion this test has no other way to make.
    // `block_in_place` (used inside `parse_v1`) requires the multi-threaded
    // runtime; the current-thread flavor other tests in this module use
    // panics with "can call blocking only when running on the multi-threaded
    // runtime".
    #[tokio::test(flavor = "multi_thread")]
    async fn parse_v1_handler_extracts_raw_bytes_not_a_json_type() {
        let manifest_path = boot_proof::tests::write_test_manifest_fixture();
        let pair = qos_p256::P256Pair::generate().unwrap();
        let boot_proof = StaticBootProof::from_enclave_files_at(
            &pair,
            "visualsign-parser".to_string(),
            "test".to_string(),
            &manifest_path,
        )
        .expect("test manifest fixture should be readable");
        let state = AppState {
            ephemeral_key: Arc::new(pair),
            boot_proof: Arc::new(boot_proof),
            config: ParserConfig::accept_unsigned(),
        };
        let raw = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02","include_intermediate_output":false}}"#;
        let body = axum::body::Bytes::from_static(raw);
        let (_, Json(resp)) = parse_v1(State(state), body).await;
        // Reaching a structured envelope (rather than a panic or a bare axum
        // rejection) proves the handler ran end to end through the real `Bytes`
        // extractor, not a bypassed helper.
        assert!(resp.error.is_some());
    }

    #[test]
    fn static_boot_proof_has_the_six_keys_and_a_real_ephemeral_pubkey() {
        let manifest_path = boot_proof::tests::write_test_manifest_fixture();
        let pair = qos_p256::P256Pair::generate().unwrap();
        let expected_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let source = StaticBootProof::from_enclave_files_at(
            &pair,
            "visualsign-parser".to_string(),
            "test".to_string(),
            &manifest_path,
        )
        .expect("test manifest fixture should be readable");
        let bp = source.boot_proof();
        assert_eq!(bp.ephemeral_public_key_hex, expected_hex);
        // A later PR fills the doc; until then it is explicitly empty, never a fake.
        assert!(bp.aws_attestation_doc_b64.is_empty());
        let value = serde_json::to_value(&bp).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 6);
    }

    // Regression pin: an earlier version of this middleware rewrote every
    // 404/405 response by status alone, which clobbered `handle_parse`'s own
    // 404 (`Code::NotFound`, with the parser's real error message) with this
    // fixed generic text. `Router::fallback` / `method_not_allowed_fallback`
    // are only invoked by axum when no handler produced a response at all
    // (see the axum docs on `method_not_allowed_fallback`), so they can never
    // run after `handle_parse` - fixing the class of bug structurally rather
    // than by inspecting response bodies. This test pins the fallbacks'
    // fixed messages; it cannot exercise `handle_parse`'s own `Code::NotFound`
    // arm end to end because nothing in `parser_app::routes::parse::parse`
    // currently returns that code.
    #[tokio::test]
    async fn fallbacks_carry_their_own_fixed_message_and_boot_proof() {
        let manifest_path = boot_proof::tests::write_test_manifest_fixture();
        let pair = qos_p256::P256Pair::generate().unwrap();
        let boot_proof = StaticBootProof::from_enclave_files_at(
            &pair,
            "visualsign-parser".to_string(),
            "test".to_string(),
            &manifest_path,
        )
        .expect("test manifest fixture should be readable");
        let state = AppState {
            ephemeral_key: Arc::new(pair),
            boot_proof: Arc::new(boot_proof),
            config: ParserConfig::accept_unsigned(),
        };

        let not_found = not_found_fallback(State(state.clone())).await;
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(not_found.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("error").unwrap(), "not found");
        assert!(value.get("bootProof").is_some());

        let method_not_allowed = method_not_allowed_fallback(State(state)).await;
        assert_eq!(method_not_allowed.status(), StatusCode::METHOD_NOT_ALLOWED);
        let body = axum::body::to_bytes(method_not_allowed.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("error").unwrap(), "method not allowed");
        assert!(value.get("bootProof").is_some());
    }
}
