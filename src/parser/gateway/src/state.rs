//! Shared application state for the gateway router.

use generated::grpc::health::v1::health_client::HealthClient;
use generated::parser::parser_service_client::ParserServiceClient;
use generated::tonic;

pub type GrpcClient = ParserServiceClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct AppState {
    pub grpc_client: GrpcClient,
    pub health_client: HealthClient<tonic::transport::Channel>,
}
