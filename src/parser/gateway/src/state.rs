//! Shared application state for the gateway router.

use crate::attestation::AttestationVerifier;
use crate::signing::GatewaySigner;
use crate::x402_config::X402Config;
use generated::grpc::health::v1::health_client::HealthClient;
use generated::parser::parser_service_client::ParserServiceClient;
use generated::tonic;
use std::sync::Arc;

pub type GrpcClient = ParserServiceClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct AppState {
    pub grpc_client: GrpcClient,
    pub health_client: HealthClient<tonic::transport::Channel>,
    /// Optional pinned demo response-attestation verifier (plan v1).
    /// `None` in the new TVC-enforced flow, where parser_app is the
    /// trust boundary instead.
    pub attestation: Option<Arc<AttestationVerifier>>,
    /// Gateway signing identity for VerifiedPaymentMarkers. Set when the
    /// TVC-enforced v2 route is enabled.
    pub signer: Option<Arc<GatewaySigner>>,
    /// X402 config (facilitator URL, price tags). Set when v2 is enabled.
    pub x402_config: Option<Arc<X402Config>>,
}
