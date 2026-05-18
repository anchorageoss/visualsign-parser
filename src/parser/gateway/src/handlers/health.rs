//! Health-check handler — proxies to the gRPC backend's health service.

use crate::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use generated::grpc::health::v1::{HealthCheckRequest, health_check_response::ServingStatus};
use generated::tonic;
use std::time::Duration;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn health_handler(
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
                    Json(serde_json::json!({
                        "status": "unhealthy",
                        "reason": "grpc service not serving"
                    })),
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
                Json(serde_json::json!({
                    "status": "unhealthy",
                    "reason": "health check timed out"
                })),
            )
        }
    }
}
