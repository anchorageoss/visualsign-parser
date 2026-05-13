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
use std::net::SocketAddr;

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

    let state = parser_gateway::state::AppState {
        grpc_client,
        health_client,
    };

    let x402_cfg =
        parser_gateway::x402_config::X402Config::from_env().expect("invalid X402 configuration");
    let x402_middleware = x402_cfg
        .build_middleware()
        .expect("invalid X402 price tags");

    if let Err(e) = probe_facilitator(&x402_cfg.facilitator_url, x402_cfg.facilitator_timeout).await
    {
        return Err(format!(
            "x402 facilitator probe failed for {}: {e}",
            x402_cfg.facilitator_url
        )
        .into());
    }
    println!("x402 facilitator probe OK");

    let app = Router::new()
        .route(
            "/health",
            get(parser_gateway::handlers::health::health_handler),
        )
        .route(
            "/visualsign/api/v1/parse",
            post(parser_gateway::handlers::parse::parse_handler),
        )
        .route(
            "/visualsign/api/v2/parse",
            post(parser_gateway::handlers::parse::parse_handler).layer(x402_middleware),
        )
        .layer(DefaultBodyLimit::max(GRPC_MAX_RECV_MSG_SIZE))
        .with_state(state);

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
