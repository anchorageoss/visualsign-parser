// TODO(#231): Remove this exemption and fix violations in a follow-up PR.
// unwrap_used and panic have no remaining non-test call sites in this crate;
// only the SIGTERM-handler setup below still relies on expect_used.
#![allow(clippy::expect_used)]

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

    // Distinguish "unset" from "set but not valid UTF-8" the same way the
    // bearer-token loader does (see auth.rs::read_env_var): collapsing both
    // into the `local` default would let a malformed deployment env silently
    // bypass the non-local attestation requirement below instead of failing
    // startup loudly.
    let profile_str = match std::env::var("X402_PROFILE") {
        Ok(v) => v,
        Err(std::env::VarError::NotPresent) => "local".to_string(),
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("FATAL: X402_PROFILE contains invalid (non-UTF-8) bytes");
            std::process::exit(1);
        }
    };
    let is_local_profile = profile_str == "local";

    // Load x402 config up front (before the attestation decision below) so
    // that decision can account for whether this profile could actually
    // settle real money, not just its declared name. `X402_PROFILE=local`
    // can still be pointed at a mainnet network/payTo and an external,
    // non-loopback facilitator; in that case, treating it as "no verifier
    // needed" (the `local` default) would let real USDC settle while
    // forwarding responses with zero attestation. Config errors here are
    // handled again, identically, by the soft-fail block below -- this
    // first load only feeds the attestation-requirement check.
    //
    // Also require the config to actually build (e.g. a valid
    // (payTo, network) combination): `build_middleware` is the real
    // arbiter of whether x402 will be mounted at all, and the soft-fail
    // block below disables x402 (keeping v1/health up) on the same
    // failure. A config that can't build can't settle anything, so it
    // must not trip the fail-closed attestation requirement.
    let x402_result = X402Config::from_env();
    let x402_can_settle_for_real = match &x402_result {
        Ok(cfg) => cfg.build_middleware().is_ok() && x402_targets_real_settlement(cfg),
        Err(_) => false,
    };

    // Build the TVC attestation verifier. The pinned pubkey is provisioned
    // out-of-band (Turnkey TVC plants it as a launch arg) and must match the
    // enclave's ephemeral key. Fail-closed whenever the resolved x402 config
    // can settle real payments: a gateway without a pinned verifier would
    // happily forward (and settle for) unattested responses.
    let attestation: Option<Arc<AttestationVerifier>> = match AttestationVerifier::from_env() {
        Ok(Some(v)) => {
            let hex = v.pinned_hex();
            let head = &hex[..8.min(hex.len())];
            let tail = &hex[hex.len().saturating_sub(8)..];
            println!("x402 attestation: pinned TVC pubkey {head}..{tail}");
            Some(Arc::new(v))
        }
        Ok(None) => {
            if is_local_profile && !x402_can_settle_for_real {
                eprintln!(
                    "WARNING: TVC_DEMO_PINNED_PUBKEY_HEX not set; gateway will not attest \
                     parse responses (allowed because X402_PROFILE=local and the resolved \
                     x402 config is loopback/testnet only)"
                );
                None
            } else {
                eprintln!(
                    "FATAL: TVC_DEMO_PINNED_PUBKEY_HEX (or _FILE) is required for \
                     X402_PROFILE={profile_str}{}",
                    if is_local_profile {
                        " because the resolved facilitator/network can settle real payments"
                    } else {
                        ""
                    }
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("FATAL: invalid TVC verifier pubkey configuration: {e}");
            std::process::exit(1);
        }
    };

    let state = parser_gateway::state::AppState {
        grpc_client,
        health_client,
        attestation,
    };

    // Caps the public ingress body. The gRPC backend's
    // `GRPC_MAX_RECV_MSG_SIZE` (~25 MiB) is the wrong ceiling for the public
    // HTTP layer -- a 25 MiB unauthenticated body lets a non-paying caller
    // force the gateway to JSON-parse 25 MB before any Payment-Signature
    // check runs. But the backend contract (visualsign-ethereum's
    // `MAX_ABI_JSON_BYTES` / visualsign-solana's `MAX_IDL_JSON_BYTES`) allows
    // each proto-supplied `abi_mappings`/`idl_mappings` entry up to 1 MiB, and
    // `chain_metadata` may carry more than one (e.g. a proxy plus its
    // implementation). 2 MiB comfortably covers that documented contract
    // (one full-size mapping plus JSON overhead, with room for a second)
    // while still shrinking the pre-paywall amplification surface by ~12x
    // vs the old 25 MiB ceiling. Applied router-wide (below, after both
    // routes are mounted), matching bd3b0657's intent on a wider bound.
    const PUBLIC_BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024;

    let mut app = Router::new()
        .route(
            "/health",
            get(parser_gateway::handlers::health::health_handler),
        )
        .route(
            "/visualsign/api/v1/parse",
            post(parser_gateway::handlers::parse::parse_handler),
        );

    // x402 config/price-tag errors are a soft fail: log and keep serving
    // v1 + health only. Same treatment as an unreachable facilitator below;
    // an x402-only misconfiguration should not take the unrelated v1 route
    // down with it.
    match x402_result {
        Ok(x402_cfg) => match x402_cfg.build_middleware() {
            Ok(x402_middleware) => {
                if let Err(e) =
                    probe_facilitator(&x402_cfg.facilitator_url, x402_cfg.facilitator_timeout).await
                {
                    eprintln!(
                        "WARNING: x402 disabled; facilitator probe failed for {}: {e}",
                        x402_cfg.facilitator_url
                    );
                } else {
                    println!("x402 facilitator probe OK");
                    for tag in &x402_cfg.price_tags {
                        println!(
                            "x402 price tag: network={} asset={} price_usd={} payTo={:?}",
                            tag.network, tag.asset, tag.price_usd, tag.pay_to
                        );
                    }
                    app = app.route(
                        "/visualsign/api/v2/parse",
                        post(parser_gateway::handlers::parse::parse_handler).layer(x402_middleware),
                    );
                }
            }
            Err(e) => eprintln!("WARNING: x402 disabled; failed to build x402 middleware: {e}"),
        },
        Err(e) => eprintln!("WARNING: x402 disabled; invalid x402 configuration: {e}"),
    }

    // Optional shared-bearer-token gate. /health is carved out inside the
    // middleware (Cloud Run / operator probes don't need the token).
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

    if let Some(token) = bearer_token {
        app = app.layer(axum::middleware::from_fn_with_state(
            token,
            parser_gateway::auth::require_bearer_token,
        ));
    }
    let app = app
        .layer(DefaultBodyLimit::max(PUBLIC_BODY_LIMIT_BYTES))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("failed to bind {addr}: {e}"))?;
    println!("parser_gateway {} listening on {addr}", env!("VERSION"));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Whether the resolved x402 config could settle a real (mainnet) payment:
/// a non-loopback facilitator, or a mainnet price-tag network. Used to
/// require a pinned attestation verifier even under `X402_PROFILE=local`,
/// which otherwise defaults to running unattested.
fn x402_targets_real_settlement(cfg: &X402Config) -> bool {
    let facilitator_is_loopback = matches!(
        cfg.facilitator_url.host_str(),
        // `Url::host_str()` keeps the brackets on an IPv6 literal (RFC 3986
        // authority syntax), so `::1` alone never matches.
        Some("127.0.0.1") | Some("localhost") | Some("::1") | Some("[::1]")
    );
    let any_mainnet_tag = cfg
        .price_tags
        .iter()
        .any(|tag| matches!(tag.network.as_str(), "base" | "solana"));
    !facilitator_is_loopback || any_mainnet_tag
}

async fn probe_facilitator(
    url: &url::Url,
    timeout: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve "./supported" the same way x402-axum's `FacilitatorClient`
    // resolves its own `./verify` / `./settle` / `./supported` endpoints
    // (RFC 3986 relative `Url::join`), not by naively string-appending
    // "/supported". For a facilitator URL with a non-root path and no
    // trailing slash the two approaches resolve to different endpoints, so
    // matching the real client's semantics keeps this probe meaningful
    // evidence about the path x402-axum will actually use.
    let probe_url = url.join("./supported")?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use parser_gateway::x402_config::{PayToAddress, PriceScheme, PriceTagConfig, X402Profile};
    use std::str::FromStr;
    use std::time::Duration;

    fn base_config(facilitator_url: &str, network: &str) -> X402Config {
        X402Config {
            profile: X402Profile::Local,
            facilitator_url: url::Url::parse(facilitator_url).unwrap(),
            facilitator_timeout: Duration::from_secs(5),
            protocol_version: "v2".to_string(),
            price_tags: vec![PriceTagConfig {
                network: network.to_string(),
                asset: "USDC".to_string(),
                price_usd: rust_decimal::Decimal::from_str("0.001").unwrap(),
                pay_to: PayToAddress::Evm("0x000000000000000000000000000000000000dEaD".to_string()),
                scheme: PriceScheme::Exact,
            }],
        }
    }

    #[test]
    fn loopback_testnet_does_not_target_real_settlement() {
        let cfg = base_config("http://127.0.0.1:8090", "base-sepolia");
        assert!(!x402_targets_real_settlement(&cfg));
    }

    #[test]
    fn non_loopback_facilitator_targets_real_settlement() {
        // Testnet network, but the facilitator itself is a real external
        // endpoint that could settle for real -- this must be flagged even
        // though the network alone would look safe.
        let cfg = base_config("https://facilitator.payai.network", "base-sepolia");
        assert!(x402_targets_real_settlement(&cfg));
    }

    #[test]
    fn loopback_facilitator_with_mainnet_network_targets_real_settlement() {
        // Loopback facilitator, but a mainnet network tag -- still flagged,
        // since a locally-run facilitator can still forward to real rails.
        let cfg = base_config("http://127.0.0.1:8090", "base");
        assert!(x402_targets_real_settlement(&cfg));
    }

    #[test]
    fn localhost_hostname_is_treated_as_loopback() {
        let cfg = base_config("http://localhost:8090", "solana-devnet");
        assert!(!x402_targets_real_settlement(&cfg));
    }

    #[test]
    fn ipv6_loopback_is_treated_as_loopback() {
        // `Url::host_str()` returns the bracketed form for an IPv6 literal
        // ("[::1]", not "::1"); a bare "::1" match arm never fires.
        let cfg = base_config("http://[::1]:8090", "base-sepolia");
        assert!(!x402_targets_real_settlement(&cfg));
    }

    #[test]
    fn mismatched_payto_network_combination_does_not_build_even_though_flagged_as_real_settlement()
    {
        // A mainnet network tag paired with the wrong chain's payTo address
        // (e.g. `solana` network with an EVM payTo) is exactly what
        // `build_middleware` rejects (see x402_config::build_price_tag) --
        // it can never settle anything. `x402_targets_real_settlement`
        // alone would still flag it (it only looks at the network name),
        // which is why `main` ANDs it with `build_middleware().is_ok()`
        // before treating it as requiring a pinned attestation verifier.
        let cfg = base_config("http://127.0.0.1:8090", "solana");
        assert!(x402_targets_real_settlement(&cfg));
        assert!(cfg.build_middleware().is_err());
    }
}
