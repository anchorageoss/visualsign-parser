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
use crate::x402_config::{X402Config, network_matches_chain};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use generated::parser::{Chain, ChainMetadata, ParseRequest};
use generated::tonic;
use host_primitives::payment_marker::{VPM_VERSION, VerifiedPaymentMarker, request_hash};
use qos_crypto::sha_256;
use serde::Deserialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use x402_chain_eip155::KnownNetworkEip155;
use x402_chain_eip155::chain::{AssetTransferMethod, Eip155TokenDeployment};
use x402_types::networks::USDC as UsdcEip155;

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
        http_backend_url,
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

    let chain_enum = match Chain::from_str_name(&wrapper.request.chain) {
        Some(c) => c,
        None => {
            return error(
                StatusCode::BAD_REQUEST,
                &format!("unknown chain: {}", wrapper.request.chain),
            );
        }
    };
    let chain = chain_enum as i32;

    // Read X-Payment (case-insensitive). Absent -> 402 with PaymentRequired.
    let payment_b64 = match headers
        .iter()
        .find(|(name, _)| name.as_str().eq_ignore_ascii_case(PAYMENT_HEADER))
        .map(|(_, v)| v.to_str().unwrap_or_default().to_string())
    {
        Some(s) if !s.is_empty() => s,
        _ => return payment_required(&config, chain_enum),
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

    // The buyer must have echoed one of our offered tags verbatim.
    // Without this gate the buyer fully controls payTo/asset/amount/network
    // through to the facilitator and the VPM the enclave consumes — a
    // self-transfer of 1 atomic unit satisfies the facilitator and the
    // enclave's signature-only check, bypassing the paywall.
    let accepted = payload.accepted.clone().unwrap_or(serde_json::Value::Null);
    let offers = build_canonical_offers(&config, chain_enum);
    if let Err(why) = validate_accepted_is_offered(&accepted, &offers) {
        eprintln!("rejected Payment-Signature: {why}");
        return payment_required(&config, chain_enum);
    }

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

    let unsigned_payload = wrapper.request.unsigned_payload;
    let chain_metadata = wrapper.request.chain_metadata;

    let forward_result = if let Some(http_url) = http_backend_url.as_deref() {
        forward_http(
            http_url,
            unsigned_payload,
            chain,
            chain_metadata,
            payment_marker,
        )
        .await
    } else {
        forward_grpc(
            &mut grpc_client,
            unsigned_payload,
            chain,
            chain_metadata,
            payment_marker,
        )
        .await
    };
    let (payload, signature) = match forward_result {
        Ok(parts) => parts,
        Err(BackendError::PaymentRejected(msg)) => {
            eprintln!("backend rejected payment: {msg}");
            return payment_required(&config, chain_enum);
        }
        Err(BackendError::Failed { status, msg }) => return error(status, &msg),
    };

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

/// 402 PaymentRequired with the canonical x402 v2 body, filtered to the
/// price tags whose network matches the parse-request `chain`. Translates
/// our internal network/asset names to the CAIP-2 form payai's TS client
/// expects, and includes payai's known fee-payer in `extra` so the buyer
/// can build a tx with payai as fee-payer (the only fee-payer the
/// facilitator will co-sign at /settle).
///
/// When no configured tag covers the request's chain (e.g. CHAIN_TRON,
/// CHAIN_SUI, CHAIN_BITCOIN, CHAIN_CUSTOM, CHAIN_UNSPECIFIED today),
/// returns 400 instead of a 402 — paywalling a request the gateway has
/// no settlement path for would mislead the buyer.
fn payment_required(
    config: &X402Config,
    chain: Chain,
) -> (StatusCode, HeaderMap, Json<TurnkeyResponseWrapper>) {
    let accepts = build_canonical_offers(config, chain);
    if accepts.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            &format!(
                "x402 payment not available for chain {chain_name}; \
                 gateway has no price tag matching a network x402 settles on \
                 for this chain (supported: CHAIN_ETHEREUM, CHAIN_SOLANA)",
                chain_name = chain.as_str_name(),
            ),
        );
    }

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

/// Build the canonical `accepts[]` array the gateway is willing to honor for
/// a request on `chain`. The same logic powers the 402 emission AND the
/// per-request validation gate in `validate_accepted_is_offered` — sharing
/// the construction is how we make sure the buyer can never echo back an
/// offer the gateway didn't actually make.
fn build_canonical_offers(config: &X402Config, chain: Chain) -> Vec<serde_json::Value> {
    config
        .price_tags
        .iter()
        .filter(|t| network_matches_chain(&t.network, chain))
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
        .collect()
}

/// Reject `Payment-Signature` requests whose echoed `accepted` block doesn't
/// match one of the gateway's offers for the request's chain. Without this
/// gate, the buyer fully controls the `payTo`/`asset`/`amount`/`network`
/// fields that flow into the facilitator's `/verify` + `/settle` and into
/// the VPM the enclave consumes — a self-transfer of 1 atomic unit to the
/// buyer's own wallet would be settled by payai (internally consistent),
/// signed by the gateway, and honored by the enclave (which only verifies
/// signature + request-hash binding, not economic policy).
///
/// Match rule: the buyer's `(scheme, network, asset, payTo)` must equal
/// some offered tag, and the buyer's `amount` must be `>=` that tag's
/// `amount` (overpay is allowed; underpay is not). EVM addresses are
/// compared case-insensitively because checksummed mixed-case echoes are
/// equivalent to lowercase. Comparison is intentionally strict on the
/// network identifier (CAIP-2 string) to prevent cross-chain confusion.
fn validate_accepted_is_offered(
    accepted: &serde_json::Value,
    offers: &[serde_json::Value],
) -> Result<(), String> {
    if !accepted.is_object() {
        return Err("Payment-Signature `accepted` must be a JSON object".to_string());
    }
    let buyer = AcceptedClaim::extract(accepted)?;

    for offer in offers {
        // `offers` is generated by us, fields always present and well-typed.
        let o = AcceptedClaim::extract(offer)
            .map_err(|e| format!("gateway offer is malformed (bug): {e}"))?;
        if o.scheme == buyer.scheme
            && o.network == buyer.network
            && o.asset.eq_ignore_ascii_case(buyer.asset)
            && o.pay_to.eq_ignore_ascii_case(buyer.pay_to)
            && buyer.amount_atomic >= o.amount_atomic
        {
            return Ok(());
        }
    }

    Err(format!(
        "Payment-Signature accepted {{network={:?}, payTo={:?}, asset={:?}, scheme={:?}, amount={}}} \
         does not match any offer the gateway advertised for this chain. Refetch the 402 and \
         echo the unmodified `Payment-Required` entry.",
        buyer.network, buyer.pay_to, buyer.asset, buyer.scheme, buyer.amount_atomic,
    ))
}

struct AcceptedClaim<'a> {
    scheme: &'a str,
    network: &'a str,
    asset: &'a str,
    pay_to: &'a str,
    amount_atomic: u128,
}

impl<'a> AcceptedClaim<'a> {
    fn extract(v: &'a serde_json::Value) -> Result<Self, String> {
        fn s<'b>(v: &'b serde_json::Value, key: &str) -> Result<&'b str, String> {
            v.get(key)
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("missing or non-string field `{key}` in accepted"))
        }
        let amount_str = s(v, "amount")?;
        let amount_atomic: u128 = amount_str
            .parse()
            .map_err(|_| format!("amount {amount_str:?} must be a non-negative integer string"))?;
        Ok(AcceptedClaim {
            scheme: s(v, "scheme")?,
            network: s(v, "network")?,
            asset: s(v, "asset")?,
            pay_to: s(v, "payTo")?,
            amount_atomic,
        })
    }
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
        // EVM networks (USDC): pull the canonical deployment from
        // x402_chain_eip155 instead of hand-rolling. That crate is the
        // upstream source of truth for chain id, token contract address,
        // and EIP-712 domain — Circle's USDC on Base uses name "USD Coin"
        // (not "USDC") on mainnet and "USDC" on Base Sepolia, and getting
        // that wrong silently breaks signature verification.
        _ if asset == "USDC" => match eip155_usdc_deployment(network) {
            Some(usdc) => match usdc.transfer_method {
                AssetTransferMethod::Eip3009 { name, version } => (
                    usdc.chain_reference.as_chain_id().to_string(),
                    usdc.address.to_checksum(None),
                    Some(serde_json::json!({"name": name, "version": version})),
                ),
                // USDC on every chain we support today is Eip3009. If a
                // future deployment moves to Permit2, fall back to a
                // pass-through; the facilitator will reject the
                // unsupported scheme rather than silently mis-sign.
                AssetTransferMethod::Permit2 => (network.to_string(), asset.to_string(), None),
            },
            None => (network.to_string(), asset.to_string(), None),
        },
        _ => (network.to_string(), asset.to_string(), None),
    }
}

/// Returns the canonical USDC deployment for an EVM `network` short name,
/// or `None` if the network isn't a known EVM x402 venue. Centralises the
/// short-name → upstream-constant mapping so the price-tag side
/// (`x402_config::seeded_tag`) and the wire-emission side
/// (`translate_to_canonical`) can't drift.
fn eip155_usdc_deployment(network: &str) -> Option<Eip155TokenDeployment> {
    match network {
        "base-sepolia" => Some(UsdcEip155::base_sepolia()),
        "base" => Some(UsdcEip155::base()),
        _ => None,
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Avoid the unused-import warning when `GatewaySigner` is only used via
/// the AppState alias.
#[allow(dead_code)]
fn _force_use(_: &GatewaySigner) {}

#[derive(Debug)]
enum BackendError {
    /// Backend returned a "payment required" signal — the gateway will
    /// translate to the canonical 402 + PaymentRequired body.
    PaymentRejected(String),
    /// Generic backend error — surfaced to the client as-is.
    Failed { status: StatusCode, msg: String },
}

type BackendOk = (
    generated::parser::ParsedTransactionPayload,
    Option<TurnkeySignature>,
);

async fn forward_grpc(
    grpc_client: &mut crate::state::GrpcClient,
    unsigned_payload: String,
    chain: i32,
    chain_metadata: Option<host_primitives::turnkey::ChainMetadataInput>,
    payment_marker: Vec<u8>,
) -> Result<BackendOk, BackendError> {
    use generated::parser::SignatureScheme;
    let request = tonic::Request::new(ParseRequest {
        unsigned_payload,
        chain,
        chain_metadata: chain_metadata.map(ChainMetadata::from),
        payment_marker,
    });
    let response = match tokio::time::timeout(PARSE_TIMEOUT, grpc_client.parse(request)).await {
        Ok(Ok(r)) => r.into_inner(),
        Ok(Err(e)) => {
            return Err(match e.code() {
                tonic::Code::InvalidArgument => BackendError::Failed {
                    status: StatusCode::BAD_REQUEST,
                    msg: e.message().to_string(),
                },
                tonic::Code::NotFound => BackendError::Failed {
                    status: StatusCode::NOT_FOUND,
                    msg: e.message().to_string(),
                },
                tonic::Code::FailedPrecondition => {
                    BackendError::PaymentRejected(e.message().to_string())
                }
                _ => {
                    eprintln!("gRPC error: {e}");
                    BackendError::Failed {
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                        msg: "internal error".to_string(),
                    }
                }
            });
        }
        Err(_) => {
            return Err(BackendError::Failed {
                status: StatusCode::GATEWAY_TIMEOUT,
                msg: "request timed out".to_string(),
            });
        }
    };

    let parsed_tx = response
        .parsed_transaction
        .ok_or_else(|| BackendError::Failed {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg: "missing parsed_transaction".to_string(),
        })?;
    let payload = parsed_tx.payload.ok_or_else(|| BackendError::Failed {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        msg: "missing payload".to_string(),
    })?;
    let proto_signature = parsed_tx.signature.ok_or_else(|| BackendError::Failed {
        status: StatusCode::BAD_GATEWAY,
        msg: "missing signature".to_string(),
    })?;
    let scheme = match proto_signature.scheme {
        x if x == SignatureScheme::TurnkeyP256EphemeralKey as i32 => {
            SignatureScheme::TurnkeyP256EphemeralKey
        }
        _ => SignatureScheme::Unspecified,
    };
    Ok((
        payload,
        Some(TurnkeySignature {
            message: proto_signature.message,
            public_key: proto_signature.public_key,
            scheme: scheme.as_str_name().to_string(),
            signature: proto_signature.signature,
        }),
    ))
}

async fn forward_http(
    base_url: &str,
    unsigned_payload: String,
    chain: i32,
    chain_metadata: Option<host_primitives::turnkey::ChainMetadataInput>,
    payment_marker: Vec<u8>,
) -> Result<BackendOk, BackendError> {
    use host_primitives::turnkey::{TurnkeyRequest, TurnkeyRequestWrapper};
    let chain_str = match generated::parser::Chain::from_i32(chain) {
        Some(c) => c.as_str_name().to_string(),
        None => {
            return Err(BackendError::Failed {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                msg: format!("invalid chain enum {chain}"),
            });
        }
    };
    let body = TurnkeyRequestWrapper {
        request: TurnkeyRequest {
            unsigned_payload,
            chain: chain_str,
            chain_metadata,
            payment_marker_b64: Some(
                base64::engine::general_purpose::STANDARD.encode(&payment_marker),
            ),
        },
    };

    let url = format!("{}/visualsign/api/v2/parse", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(PARSE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            return Err(BackendError::Failed {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                msg: format!("reqwest build: {e}"),
            });
        }
    };
    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(BackendError::Failed {
                status: StatusCode::BAD_GATEWAY,
                msg: format!("http backend unreachable: {e}"),
            });
        }
    };
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let parsed: TurnkeyResponseWrapper = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Err(BackendError::Failed {
                status: StatusCode::BAD_GATEWAY,
                msg: format!("http backend returned non-JSON ({status}): {e}; body: {text}"),
            });
        }
    };

    // parser_http_server returns 402 with `error` set when the VPM check
    // fails. Surface that as PaymentRejected so the gateway translates
    // back to a canonical PaymentRequired body keyed off our config.
    if status == StatusCode::PAYMENT_REQUIRED {
        return Err(BackendError::PaymentRejected(
            parsed
                .error
                .unwrap_or_else(|| "payment required".to_string()),
        ));
    }
    if !status.is_success() {
        return Err(BackendError::Failed {
            status: StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            msg: parsed
                .error
                .unwrap_or_else(|| format!("http backend error {status}")),
        });
    }

    let payload = parsed.response.parsed_transaction.payload;
    let signature = parsed.response.parsed_transaction.signature;
    Ok((
        generated::parser::ParsedTransactionPayload {
            parsed_payload: payload.signable_payload.clone(),
            input_payload_digest: payload.input_payload_digest,
            metadata_digest: payload.metadata_digest,
            signable_payload: payload.signable_payload,
        },
        signature,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn translate_evm_base_sepolia_emits_v2_shape() {
        let (network, asset, extra) = translate_to_canonical("base-sepolia", "USDC");
        assert_eq!(network, "eip155:84532");
        let asset_addr = asset.strip_prefix("0x").expect("EVM asset must be 0x-hex");
        assert_eq!(asset_addr.len(), 40, "EVM asset must be 20-byte contract");
        let extra = extra.expect("EVM must carry EIP-712 extra");
        assert_eq!(extra["name"], "USDC", "Base Sepolia USDC domain name");
        assert_eq!(extra["version"], "2");
    }

    #[test]
    fn translate_evm_base_mainnet_uses_usd_coin_name() {
        // Latent bug guard: Circle's FiatTokenV2_2 on Base mainnet uses
        // the EIP-712 domain name "USD Coin", not "USDC". Hardcoding
        // "USDC" (as a previous revision did) silently breaks signature
        // verification on mainnet while Base Sepolia keeps working.
        let (network, _, extra) = translate_to_canonical("base", "USDC");
        assert_eq!(network, "eip155:8453");
        let extra = extra.expect("EVM must carry EIP-712 extra");
        assert_eq!(
            extra["name"], "USD Coin",
            "Base mainnet USDC domain name must come from x402_chain_eip155 (Circle uses \"USD Coin\")"
        );
        assert_eq!(extra["version"], "2");
    }

    #[test]
    fn translate_solana_devnet_emits_caip2_and_fee_payer() {
        let (network, asset, extra) = translate_to_canonical("solana-devnet", "USDC");
        assert!(network.starts_with("solana:"), "CAIP-2 form for Solana");
        assert_eq!(asset, "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
        let extra = extra.expect("payai requires a feePayer in extra");
        assert!(
            extra.get("feePayer").is_some(),
            "Solana extra must carry payai feePayer"
        );
    }

    #[test]
    fn translate_solana_mainnet_emits_caip2_and_fee_payer() {
        let (network, asset, extra) = translate_to_canonical("solana", "USDC");
        assert!(network.starts_with("solana:"));
        assert_eq!(asset, "EPjFWdd5AufqSSqeMxKf8aSXdrEv2Hk7UFEqA8zoYC");
        assert!(extra.is_some());
    }

    #[test]
    fn translate_unknown_network_passes_through() {
        let (n, a, x) = translate_to_canonical("polkadot", "USDC");
        assert_eq!(n, "polkadot");
        assert_eq!(a, "USDC");
        assert!(x.is_none());
    }

    #[test]
    fn translate_non_usdc_asset_passes_through_unchanged() {
        // We only know how to resolve USDC today. A request for, say,
        // "USDT" on base-sepolia leaves both fields alone — that scheme
        // isn't supported and the facilitator will reject downstream.
        let (n, a, _) = translate_to_canonical("base-sepolia", "USDT");
        assert_eq!(n, "base-sepolia");
        assert_eq!(a, "USDT");
    }

    // ── validate_accepted_is_offered ────────────────────────────────────

    fn one_offer() -> serde_json::Value {
        serde_json::json!({
            "scheme": "exact",
            "network": "eip155:84532",
            "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
            "amount": "100",
            "payTo": "0x7850B376011285F023603E8AD09b550b47f05bf5",
            "maxTimeoutSeconds": 300,
            "extra": {"name": "USDC", "version": "2"},
        })
    }

    #[test]
    fn validate_accepts_exact_echo() {
        let offer = one_offer();
        assert!(validate_accepted_is_offered(&offer, &[offer.clone()]).is_ok());
    }

    #[test]
    fn validate_accepts_overpay() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer["amount"] = serde_json::Value::String("1000000".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_ok());
    }

    #[test]
    fn validate_accepts_lowercased_evm_addresses() {
        // EVM addresses come back from `Address::to_checksum(None)` mixed-case.
        // A buyer that normalises to lowercase before echoing still matches.
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer["asset"] =
            serde_json::Value::String("0x036cbd53842c5426634e7929541ec2318f3dcf7e".to_string());
        buyer["payTo"] =
            serde_json::Value::String("0x7850b376011285f023603e8ad09b550b47f05bf5".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_ok());
    }

    #[test]
    fn validate_rejects_tampered_pay_to() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer["payTo"] =
            serde_json::Value::String("0xAAAAaaaaAAaaAaaAaaAaAAAaaAAAAAaaaAAAAaAa".to_string());
        let err = validate_accepted_is_offered(&buyer, &[offer]).unwrap_err();
        assert!(err.contains("does not match"), "unexpected error: {err}");
    }

    #[test]
    fn validate_rejects_undercut_amount() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer["amount"] = serde_json::Value::String("1".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_err());
    }

    #[test]
    fn validate_rejects_wrong_asset() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        // Same-network, different (fake) token. A buyer can't substitute
        // their own ERC-20 even if the rest matches.
        buyer["asset"] =
            serde_json::Value::String("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_err());
    }

    #[test]
    fn validate_rejects_wrong_network() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        // Cross-chain confusion: claim Base mainnet against an offer for
        // Base Sepolia. CAIP-2 mismatch.
        buyer["network"] = serde_json::Value::String("eip155:8453".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_err());
    }

    #[test]
    fn validate_rejects_wrong_scheme() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer["scheme"] = serde_json::Value::String("upto".to_string());
        assert!(validate_accepted_is_offered(&buyer, &[offer]).is_err());
    }

    #[test]
    fn validate_rejects_missing_required_field() {
        let offer = one_offer();
        let mut buyer = offer.clone();
        buyer.as_object_mut().unwrap().remove("payTo");
        let err = validate_accepted_is_offered(&buyer, &[offer]).unwrap_err();
        assert!(
            err.contains("payTo"),
            "error should name the missing field: {err}"
        );
    }

    #[test]
    fn validate_rejects_non_object_accepted() {
        let offer = one_offer();
        // Mirrors the historic "no header sent" path where `accepted` falls
        // through as Value::Null.
        assert!(validate_accepted_is_offered(&serde_json::Value::Null, &[offer]).is_err());
    }

    #[test]
    fn validate_rejects_when_no_offers_for_chain() {
        // When the chain has no offers (e.g. CHAIN_TRON), even a perfectly
        // well-formed accepted is rejected — there's nothing to match.
        let buyer = one_offer();
        assert!(validate_accepted_is_offered(&buyer, &[]).is_err());
    }

    #[test]
    fn validate_picks_matching_offer_in_multi_chain_config() {
        // Local profile seeds two tags (one EVM, one Solana) — both must
        // be present in `offers` and the validator must pick the matching
        // one without false positives across chains.
        let evm = one_offer();
        let solana = serde_json::json!({
            "scheme": "exact",
            "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
            "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
            "amount": "100",
            "payTo": "11111111111111111111111111111111",
            "maxTimeoutSeconds": 300,
        });
        let offers = vec![evm.clone(), solana.clone()];

        assert!(validate_accepted_is_offered(&evm, &offers).is_ok());
        assert!(validate_accepted_is_offered(&solana, &offers).is_ok());

        // Cross-substitution: Solana asset against EVM offer — must not pass.
        let mut frankenstein = evm.clone();
        frankenstein["asset"] = solana["asset"].clone();
        assert!(validate_accepted_is_offered(&frankenstein, &offers).is_err());
    }
}
