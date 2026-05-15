//! Shared application state for the gateway router.

use crate::attestation::AttestationVerifier;
use generated::grpc::health::v1::health_client::HealthClient;
use generated::parser::parser_service_client::ParserServiceClient;
use generated::tonic;
use std::sync::Arc;

pub type GrpcClient = ParserServiceClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct AppState {
    pub grpc_client: GrpcClient,
    pub health_client: HealthClient<tonic::transport::Channel>,
    /// Optional pinned TVC verifier. When set, every parse response is
    /// validated before the gateway returns 200; on failure the handler
    /// returns 502 and x402-axum's settle-on-success contract skips
    /// settlement. When `None`, the gateway runs without attestation —
    /// allowed only when `X402_PROFILE=local` (enforced at startup).
    pub attestation: Option<Arc<AttestationVerifier>>,
}
