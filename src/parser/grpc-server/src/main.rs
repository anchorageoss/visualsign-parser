//! gRPC server - single binary gRPC server for non-TEE deployments

use generated::parser::{
    parser_service_server::{ParserService, ParserServiceServer},
    ParseRequest, ParseResponse,
};
use generated::tonic::{self, Request, Response, Status};
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
use std::net::SocketAddr;

/// Standalone gRPC service that calls the parser directly
struct GrpcService {
    ephemeral_key: P256Pair,
}

impl GrpcService {
    fn new(ephemeral_file: &str) -> Self {
        let handle = EphemeralKeyHandle::new(ephemeral_file.to_string());
        let ephemeral_key = handle
            .get_ephemeral_key()
            .expect("Failed to load ephemeral key");
        Self { ephemeral_key }
    }
}

#[tonic::async_trait]
impl ParserService for GrpcService {
    async fn parse(
        &self,
        request: Request<ParseRequest>,
    ) -> Result<Response<ParseResponse>, Status> {
        // Direct function call - no sockets needed
        parse(request.into_inner(), &self.ephemeral_key)
            .map(Response::new)
            .map_err(|e| Status::new(tonic::Code::from_i32(e.code as i32), e.message))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = "0.0.0.0:44020".parse()?;

    // Use the test fixture for development; in production, use EPHEMERAL_KEY_FILE
    let ephemeral_file = std::env::var("EPHEMERAL_FILE")
        .unwrap_or_else(|_| "integration/fixtures/ephemeral.secret".to_string());

    let svc = GrpcService::new(&ephemeral_file);

    let reflection_service = generated::tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(generated::FILE_DESCRIPTOR_SET)
        .build()
        .expect("failed to start reflection service");

    println!("gRPC server listening on {addr}");

    tonic::transport::Server::builder()
        .add_service(reflection_service)
        .add_service(ParserServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}
