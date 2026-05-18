// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use generated::grpc::health::v1::health_client::HealthClient;
use generated::parser::parser_service_client::ParserServiceClient;
use generated::tonic;
use host_primitives::GRPC_MAX_RECV_MSG_SIZE;
use parser_gateway::attestation::AttestationVerifier;
use parser_gateway::auth::BearerToken;
use parser_gateway::signing::GatewaySigner;
use parser_gateway::x402_config::X402Config;
use std::net::SocketAddr;
use std::sync::Arc;

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

    let profile_str = std::env::var("X402_PROFILE").unwrap_or_else(|_| "local".to_string());
    let is_local_profile = profile_str == "local";
    // The new TVC-enforced flow (GATEWAY_SIGNING_KEY_FILE set) replaces the
    // demo response-attestation verifier — parser_app is the trust boundary
    // instead. In that mode the demo TVC_DEMO_PINNED_PUBKEY_HEX is optional.
    let tvc_enforced = std::env::var("GATEWAY_SIGNING_KEY_FILE").is_ok();

    let attestation: Option<Arc<AttestationVerifier>> = match AttestationVerifier::from_env() {
        Ok(Some(v)) => {
            let hex = v.pinned_hex();
            let head = &hex[..8.min(hex.len())];
            let tail = &hex[hex.len().saturating_sub(8)..];
            println!("x402 attestation: pinned TVC pubkey {head}..{tail}");
            Some(Arc::new(v))
        }
        Ok(None) => {
            if is_local_profile || tvc_enforced {
                eprintln!(
                    "INFO: demo TVC_DEMO_PINNED_PUBKEY_HEX not set (allowed for \
                     X402_PROFILE=local or when GATEWAY_SIGNING_KEY_FILE drives the \
                     TVC-enforced flow)."
                );
                None
            } else {
                eprintln!(
                    "FATAL: TVC_DEMO_PINNED_PUBKEY_HEX (or _FILE) is required for \
                     X402_PROFILE={profile_str} without GATEWAY_SIGNING_KEY_FILE"
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("FATAL: invalid TVC verifier pubkey configuration: {e}");
            std::process::exit(1);
        }
    };

    // Optional gateway signing identity for the TVC-enforced v2 route.
    // When set, the v2 route uses the hand-rolled handler that does
    // verify -> settle -> sign VPM -> forward in that order.
    let signer = match GatewaySigner::from_env() {
        Ok(s) => {
            println!("gateway signer loaded; pubkey {}...", &s.public_hex()[..16]);
            Some(Arc::new(s))
        }
        Err(parser_gateway::signing::SigningError::MissingEnv) => None,
        Err(e) => {
            eprintln!("FATAL: GATEWAY_SIGNING_KEY_FILE is set but invalid: {e}");
            std::process::exit(1);
        }
    };

    // Load x402 config up front so both the v2 route handler and the
    // payment-required response builder share the same source of truth.
    let x402_cfg = match X402Config::from_env() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("WARNING: x402 disabled; invalid x402 configuration: {e}");
            None
        }
    };

    let mut app = Router::new().route(
        "/health",
        get(parser_gateway::handlers::health::health_handler),
    );

    // Mount the open v1 route ONLY when the gateway is not in TVC-enforced
    // mode. parser_app's PaymentPolicy is global once `GATEWAY_SIGNING_PUBKEY_HEX`
    // is pinned — every parse request needs a valid VPM, so an open v1 route
    // would 402 every call.
    if signer.is_none() {
        app = app.route(
            "/visualsign/api/v1/parse",
            post(parser_gateway::handlers::parse::parse_handler),
        );
    }

    if let Some(ref cfg) = x402_cfg {
        if let Err(e) = probe_facilitator(&cfg.facilitator_url, cfg.facilitator_timeout).await {
            eprintln!(
                "WARNING: x402 disabled; facilitator probe failed for {}: {e}",
                cfg.facilitator_url
            );
        } else {
            println!("x402 facilitator probe OK");
            // TVC-enforced flow when the gateway has a signer; otherwise
            // fall back to the legacy demo path. The legacy path was
            // gateway-asserted-payment; in this branch we replace it with
            // the TVC-enforced handler when configured.
            app = app.route(
                "/visualsign/api/v2/parse",
                post(parser_gateway::handlers::parse_tvc::parse_handler_tvc),
            );
            if signer.is_none() {
                eprintln!(
                    "WARNING: v2 route mounted without a GATEWAY_SIGNING_KEY_FILE; \
                     every request will fail with 500 'gateway signer not configured'. \
                     Set GATEWAY_SIGNING_KEY_FILE to enable TVC-enforced payment."
                );
            }
        }
    }

    // Optional HTTP backend: when set, the v2 TVC-enforced handler POSTs
    // the parse request to this URL (paired with parser_http_server)
    // instead of forwarding via gRPC. Used to put the gateway in front of
    // a TVC-deployed parser_http_server whose only listener is HTTP.
    let http_backend_url = std::env::var("HTTP_BACKEND_URL")
        .ok()
        .filter(|s| !s.is_empty());
    if let Some(ref url) = http_backend_url {
        println!("HTTP backend: {url} (gRPC channel unused on /v2)");
    }

    let state = parser_gateway::state::AppState {
        grpc_client,
        health_client,
        attestation,
        signer,
        x402_config: x402_cfg.map(Arc::new),
        http_backend_url,
    };

    // Optional shared-bearer-token gate. Sits above the body-limit layer so
    // unauthenticated callers don't even consume the 64 KiB JSON-parse budget.
    // /health is carved out inside the middleware (Cloud Run / operator probes
    // don't need the token).
    let bearer_token = match BearerToken::from_env() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("FATAL: invalid gateway-auth bearer-token configuration: {e}");
            std::process::exit(1);
        }
    };
    if let Some(token) = bearer_token.as_ref() {
        let len = token.byte_len();
        println!("gateway bearer-token gate enabled ({len}-byte token)");
    }

    // 64 KiB caps the public ingress body. The gRPC backend's
    // `GRPC_MAX_RECV_MSG_SIZE` (~25 MiB) is the wrong number for the public
    // HTTP layer — a parse request is ≤ a few KB in real traffic, while a
    // 25 MiB unauthenticated body lets a non-paying caller force the gateway
    // to JSON-parse 25 MB before any Payment-Signature check runs. 64 KiB
    // leaves headroom for `chain_metadata.abi_mappings` while shrinking the
    // pre-paywall amplification surface by ~400×.
    const PUBLIC_BODY_LIMIT_BYTES: usize = 64 * 1024;
    let mut app = app.layer(DefaultBodyLimit::max(PUBLIC_BODY_LIMIT_BYTES));
    if let Some(token) = bearer_token {
        app = app.layer(axum::middleware::from_fn_with_state(
            token,
            parser_gateway::auth::require_bearer_token,
        ));
    }
    let app = app.with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("parser_gateway {} listening on {addr}", env!("VERSION"));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn probe_facilitator(
    url: &url::Url,
    timeout: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut probe_url = url.clone();
    let base_path = probe_url.path().trim_end_matches('/').to_string();
    probe_url.set_path(&format!("{base_path}/supported"));
    let client = reqwest::Client::builder().timeout(timeout).build()?;
    let resp = client.get(probe_url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("facilitator returned {}", resp.status()).into());
    }
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
