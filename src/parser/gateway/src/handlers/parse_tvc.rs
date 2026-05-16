//! TVC-enforced v2 parse handler.
//!
//! Hand-rolled call order — verify → settle → sign VPM → forward — so
//! parser_app sees the txid before it ever processes the request. We do
//! NOT use x402-axum's `.layer(middleware)` here because its
//! settle-on-success contract runs settle *after* the handler returns,
//! which is too late to put the txid into the VPM.
//!
//! The facilitator is still external (payai or the bundled mock_facilitator);
//! the gateway just orchestrates the order of calls and signs the marker
//! between settle and parser_app.

use crate::signing::GatewaySigner;
use crate::state::AppState;
use crate::turnkey::{
    TurnkeyParsedTransaction, TurnkeyPayload, TurnkeyRequestWrapper, TurnkeyResponse,
    TurnkeyResponseWrapper, TurnkeySignature, error_response,
};
use crate::x402_config::X402Config;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use generated::parser::{Chain, ChainMetadata, ParseRequest, SignatureScheme};
use generated::tonic;
use host_primitives::payment_marker::{VPM_VERSION, VerifiedPaymentMarker, request_hash};
use qos_crypto::sha_256;
use serde::Deserialize;
use std::time::Duration;

const PARSE_TIMEOUT: Duration = Duration::from_secs(30);
const FACILITATOR_TIMEOUT: Duration = Duration::from_secs(10);

/// x402 v2 payment header name. The PayAI TS client uses `PAYMENT-SIGNATURE`;
/// the case-insensitive HTTP standard makes this match across casings.
const PAYMENT_HEADER: &str = "payment-signature";

/// Outer wire shape of the X-PAYMENT body the buyer sends. We only care
/// about the `accepted` echo (it carries the network/asset/payTo the
/// buyer agreed to). The full body is forwarded to the facilitator as-is.
#[derive(Debug, Deserialize)]
struct PaymentPayloadLite {
    #[serde(default)]
    accepted: Option<serde_json::Value>,
}

pub async fn parse_handler_tvc(
    State(AppState {
        mut grpc_client,
        signer,
        x402_config,
        ..
    }): State<AppState>,
    headers: HeaderMap,
    Json(wrapper): Json<TurnkeyRequestWrapper>,
) -> (StatusCode, HeaderMap, Json<TurnkeyResponseWrapper>) {
    let signer = match signer {
        Some(s) => s,
        None => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway signer not configured",
            );
        }
    };
    let config = match x402_config {
        Some(c) => c,
        None => return error(StatusCode::INTERNAL_SERVER_ERROR, "x402 config not loaded"),
    };

    let chain = match Chain::from_str_name(&wrapper.request.chain) {
        Some(c) => c as i32,
        None => {
            return error(
                StatusCode::BAD_REQUEST,
                &format!("unknown chain: {}", wrapper.request.chain),
            );
        }
    };

    // Read X-Payment (case-insensitive). Absent -> 402 with PaymentRequired.
    let payment_b64 = match headers
        .iter()
        .find(|(name, _)| name.as_str().eq_ignore_ascii_case(PAYMENT_HEADER))
        .map(|(_, v)| v.to_str().unwrap_or_default().to_string())
    {
        Some(s) if !s.is_empty() => s,
        _ => return payment_required(&config),
    };

    // Decode the payment payload so we can pick the matching price tag.
    let payment_bytes =
        match base64::engine::general_purpose::STANDARD.decode(payment_b64.as_bytes()) {
            Ok(b) => b,
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Payment-Signature is not valid base64",
                );
            }
        };
    let payload: PaymentPayloadLite = match serde_json::from_slice(&payment_bytes) {
        Ok(p) => p,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "Payment-Signature is not valid JSON",
            );
        }
    };

    // Find the matching price tag (config-side). We use the network as the
    // primary key; the buyer must have picked one of our offered tags.
    let accepted = payload
        .accepted
        .clone()
        .unwrap_or_else(|| serde_json::Value::Null);

    let client = match reqwest::Client::builder()
        .timeout(FACILITATOR_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reqwest build: {e}");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "reqwest build failure");
        }
    };

    let verify_url = format!(
        "{}/verify",
        config.facilitator_url.as_str().trim_end_matches('/')
    );
    let settle_url = format!(
        "{}/settle",
        config.facilitator_url.as_str().trim_end_matches('/')
    );

    // The verify/settle wire shape (x402 v2): { x402Version, paymentPayload, paymentRequirements }
    let verify_body = serde_json::json!({
        "x402Version": 2,
        "paymentPayload": serde_json::from_slice::<serde_json::Value>(&payment_bytes).unwrap_or(serde_json::Value::Null),
        "paymentRequirements": accepted,
    });

    match client.post(&verify_url).json(&verify_body).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            return error(
                StatusCode::PAYMENT_REQUIRED,
                &format!("facilitator /verify rejected: {}", r.status()),
            );
        }
        Err(e) => {
            eprintln!("facilitator /verify error: {e}");
            return error(StatusCode::BAD_GATEWAY, "facilitator unreachable");
        }
    }

    let settle_resp: serde_json::Value =
        match client.post(&settle_url).json(&verify_body).send().await {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("facilitator /settle JSON decode: {e}");
                    return error(StatusCode::BAD_GATEWAY, "facilitator returned non-JSON");
                }
            },
            Ok(r) => {
                return error(
                    StatusCode::BAD_GATEWAY,
                    &format!("facilitator /settle failed: {}", r.status()),
                );
            }
            Err(e) => {
                eprintln!("facilitator /settle error: {e}");
                return error(StatusCode::BAD_GATEWAY, "facilitator unreachable");
            }
        };

    let txid = settle_resp
        .get("transaction")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let payer = settle_resp
        .get("payer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let network = accepted
        .get("network")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let (mint, pay_to, amount) = accepted_to_terms(&accepted);

    // Build + sign VPM.
    let vpm = VerifiedPaymentMarker {
        version: VPM_VERSION,
        request_hash: request_hash(chain, &wrapper.request.unsigned_payload),
        txid: txid.clone(),
        payer,
        pay_to,
        amount,
        mint,
        x_payment_hash: sha_256(payment_b64.as_bytes()),
        network,
        settled_at_ms: now_ms(),
        gateway_pubkey_hex: signer.public_hex().to_string(),
    };
    let signed = match signer.sign(vpm) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("VPM sign error: {e}");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "gateway VPM signing failed",
            );
        }
    };
    let payment_marker = match borsh::to_vec(&signed) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("VPM borsh encode: {e}");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "VPM encode failure");
        }
    };

    // Forward to parser_app with the signed marker.
    let request = tonic::Request::new(ParseRequest {
        unsigned_payload: wrapper.request.unsigned_payload,
        chain,
        chain_metadata: wrapper.request.chain_metadata.map(ChainMetadata::from),
        payment_marker,
    });
    let response = match tokio::time::timeout(PARSE_TIMEOUT, grpc_client.parse(request)).await {
        Ok(Ok(r)) => r.into_inner(),
        Ok(Err(e)) => {
            let (http_status, msg) = match e.code() {
                tonic::Code::InvalidArgument => (StatusCode::BAD_REQUEST, e.message().to_string()),
                tonic::Code::NotFound => (StatusCode::NOT_FOUND, e.message().to_string()),
                tonic::Code::FailedPrecondition => {
                    // parser_app's "payment required" path — surface the
                    // canonical 402 with our PaymentRequired body.
                    eprintln!("parser_app rejected payment: {}", e.message());
                    return payment_required(&config);
                }
                _ => {
                    eprintln!("gRPC error: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal error".to_string(),
                    )
                }
            };
            return error(http_status, &msg);
        }
        Err(_) => return error(StatusCode::GATEWAY_TIMEOUT, "request timed out"),
    };

    // Materialize the response (same shape as the existing v1 handler).
    let parsed_tx = match response.parsed_transaction {
        Some(tx) => tx,
        None => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "missing parsed_transaction",
            );
        }
    };
    let payload = match parsed_tx.payload {
        Some(p) => p,
        None => return error(StatusCode::INTERNAL_SERVER_ERROR, "missing payload"),
    };
    let proto_signature = match parsed_tx.signature {
        Some(s) => s,
        None => return error(StatusCode::BAD_GATEWAY, "missing signature"),
    };
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

    // Build Payment-Response header (base64 JSON of the settle response,
    // same shape x402-axum used to set on the way out).
    let mut resp_headers = HeaderMap::new();
    if let Ok(b64) = serde_json::to_vec(&settle_resp)
        .map(|v| base64::engine::general_purpose::STANDARD.encode(v))
        && let Ok(val) = axum::http::HeaderValue::from_str(&b64)
    {
        resp_headers.insert("payment-response", val);
    }

    (
        StatusCode::OK,
        resp_headers,
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

fn error(code: StatusCode, msg: &str) -> (StatusCode, HeaderMap, Json<TurnkeyResponseWrapper>) {
    (
        code,
        HeaderMap::new(),
        Json(error_response(msg.to_string())),
    )
}

/// 402 PaymentRequired with the canonical x402 v2 body. Translates our
/// internal network/asset names to the CAIP-2 form payai's TS client
/// expects, and includes payai's known fee-payer in `extra` so the buyer
/// can build a tx with payai as fee-payer (the only fee-payer the
/// facilitator will co-sign at /settle).
fn payment_required(config: &X402Config) -> (StatusCode, HeaderMap, Json<TurnkeyResponseWrapper>) {
    let accepts: Vec<serde_json::Value> = config
        .price_tags
        .iter()
        .map(|t| {
            let (network, asset, extra) = translate_to_canonical(&t.network, &t.asset);
            let amount = (t.price_usd * rust_decimal::Decimal::from(1_000_000u64))
                .round()
                .to_string();
            let pay_to = match &t.pay_to {
                crate::x402_config::PayToAddress::Evm(s) => s.clone(),
                crate::x402_config::PayToAddress::Solana(s) => s.clone(),
            };
            let mut entry = serde_json::json!({
                "scheme": "exact",
                "network": network,
                "asset": asset,
                "amount": amount,
                "payTo": pay_to,
                "maxTimeoutSeconds": 300,
            });
            if let Some(extra_obj) = extra {
                entry["extra"] = extra_obj;
            }
            entry
        })
        .collect();

    let body = serde_json::json!({
        "x402Version": 2,
        "error": "Payment-Signature header is required",
        "accepts": accepts,
    });
    let mut headers = HeaderMap::new();
    if let Ok(bytes) = serde_json::to_vec(&body)
        && let Ok(val) = axum::http::HeaderValue::from_str(
            &base64::engine::general_purpose::STANDARD.encode(bytes),
        )
    {
        headers.insert("payment-required", val);
    }
    (
        StatusCode::PAYMENT_REQUIRED,
        headers,
        Json(error_response("payment required".to_string())),
    )
}

/// Map our short network/asset names to the CAIP-2 / mint-address form payai
/// uses on the wire, plus the facilitator-side `extra` block (fee-payer).
///
/// Stable per-network values; if payai ever changes these we'll need to
/// fetch them dynamically from `/supported`.
fn translate_to_canonical(
    network: &str,
    asset: &str,
) -> (String, String, Option<serde_json::Value>) {
    match network {
        "solana-devnet" => (
            "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".to_string(),
            // Asset on the wire is the USDC mint, regardless of what our
            // config calls it. We only know "USDC"; payai expects the mint.
            if asset == "USDC" {
                "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU".to_string()
            } else {
                asset.to_string()
            },
            Some(serde_json::json!({
                "feePayer": "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4",
            })),
        ),
        "solana" => (
            "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_string(),
            // Solana mainnet USDC mint
            if asset == "USDC" {
                "EPjFWdd5AufqSSqeMxKf8aSXdrEv2Hk7UFEqA8zoYC".to_string()
            } else {
                asset.to_string()
            },
            Some(serde_json::json!({
                "feePayer": "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4",
            })),
        ),
        // EVM and unknown networks: pass through, no extra block.
        _ => (network.to_string(), asset.to_string(), None),
    }
}

fn accepted_to_terms(accepted: &serde_json::Value) -> (String, String, String) {
    let mint = accepted
        .get("asset")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let pay_to = accepted
        .get("payTo")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let amount = accepted
        .get("amount")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (mint, pay_to, amount)
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Avoid the unused-import warning when `GatewaySigner` is only used via
/// the AppState alias.
#[allow(dead_code)]
fn _force_use(_: &GatewaySigner) {}
