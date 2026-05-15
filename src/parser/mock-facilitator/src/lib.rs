//! Mock x402 v2 facilitator — approves everything; dev/test only.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    pub payment_payload: Value,
    pub payment_requirements: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResponse {
    pub is_valid: bool,
    pub payer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleRequest {
    pub payment_payload: Value,
    pub payment_requirements: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleResponse {
    pub success: bool,
    pub transaction: String,
    pub network: String,
    pub payer: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponse {
    pub kinds: Vec<SupportedKind>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedKind {
    pub network: String,
    pub asset: String,
    pub scheme: String,
}

/// Test-observable counters for the mock facilitator.
///
/// `settle_count` is incremented on every successful `/settle` call. The x402
/// gateway integration tests use this to confirm the gateway's
/// settle-on-success contract: a 4xx/5xx response must NOT trigger settlement.
#[derive(Clone, Default)]
pub struct MockState {
    pub settle_count: Arc<AtomicUsize>,
}

pub fn router() -> Router {
    router_with_state(MockState::default())
}

pub fn router_with_state(state: MockState) -> Router {
    Router::new()
        .route("/verify", post(verify))
        .route("/settle", post(settle))
        .route("/supported", get(supported))
        .route("/debug/settle_count", get(settle_count_handler))
        .with_state(state)
}

fn extract_payer(payload: &Value) -> String {
    payload
        .get("payer")
        .and_then(|v| v.as_str())
        .unwrap_or("0xMOCKPAYER000000000000000000000000000000")
        .to_string()
}

fn extract_network(req: &Value) -> String {
    req.get("network")
        .and_then(|v| v.as_str())
        .unwrap_or("base-sepolia")
        .to_string()
}

async fn verify(Json(req): Json<VerifyRequest>) -> Json<VerifyResponse> {
    Json(VerifyResponse {
        is_valid: true,
        payer: extract_payer(&req.payment_payload),
    })
}

async fn settle(
    State(state): State<MockState>,
    Json(req): Json<SettleRequest>,
) -> Json<SettleResponse> {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let tx = format!("0xmock{}", hex_encode(&buf));
    state.settle_count.fetch_add(1, Ordering::Relaxed);
    Json(SettleResponse {
        success: true,
        transaction: tx,
        network: extract_network(&req.payment_requirements),
        payer: extract_payer(&req.payment_payload),
    })
}

async fn supported() -> Json<SupportedResponse> {
    Json(SupportedResponse {
        kinds: vec![
            SupportedKind {
                network: "base-sepolia".to_string(),
                asset: "USDC".to_string(),
                scheme: "exact".to_string(),
            },
            SupportedKind {
                network: "base".to_string(),
                asset: "USDC".to_string(),
                scheme: "exact".to_string(),
            },
            SupportedKind {
                network: "solana".to_string(),
                asset: "USDC".to_string(),
                scheme: "exact".to_string(),
            },
            SupportedKind {
                network: "solana-devnet".to_string(),
                asset: "USDC".to_string(),
                scheme: "exact".to_string(),
            },
        ],
    })
}

async fn settle_count_handler(State(state): State<MockState>) -> Json<serde_json::Value> {
    let n = state.settle_count.load(Ordering::Relaxed);
    Json(serde_json::json!({ "settle_count": n }))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn verify_always_succeeds() {
        let app = router();
        let body = serde_json::json!({
            "paymentPayload": { "payer": "0xabc" },
            "paymentRequirements": {}
        });
        let resp = app
            .oneshot(
                Request::post("/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["isValid"], true);
        assert_eq!(v["payer"], "0xabc");
    }

    #[tokio::test]
    async fn settle_returns_mock_tx_hash() {
        let app = router();
        let body = serde_json::json!({
            "paymentPayload": { "payer": "0xdef" },
            "paymentRequirements": { "network": "base" }
        });
        let resp = app
            .oneshot(
                Request::post("/settle")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["success"], true);
        assert_eq!(v["network"], "base");
        assert_eq!(v["payer"], "0xdef");
        assert!(v["transaction"].as_str().unwrap().starts_with("0xmock"));
    }

    #[tokio::test]
    async fn supported_lists_four_networks() {
        let app = router();
        let resp = app
            .oneshot(Request::get("/supported").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let kinds = v["kinds"].as_array().unwrap();
        assert_eq!(kinds.len(), 4);
        let networks: Vec<&str> = kinds
            .iter()
            .map(|k| k["network"].as_str().unwrap())
            .collect();
        assert!(networks.contains(&"solana-devnet"));
    }

    #[tokio::test]
    async fn settle_count_increments_only_on_settle() {
        let state = MockState::default();
        let app = router_with_state(state.clone());
        // initial reading
        let resp = app
            .clone()
            .oneshot(
                Request::get("/debug/settle_count")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["settle_count"], 0);

        // verify alone does NOT increment
        let body =
            serde_json::json!({ "paymentPayload": { "payer": "x" }, "paymentRequirements": {} });
        let _ = app
            .clone()
            .oneshot(
                Request::post("/verify")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(state.settle_count.load(Ordering::Relaxed), 0);

        // settle increments
        let _ = app
            .clone()
            .oneshot(
                Request::post("/settle")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(state.settle_count.load(Ordering::Relaxed), 1);
    }
}
