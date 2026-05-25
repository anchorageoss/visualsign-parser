// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

//! gRPC server - single binary gRPC server for non-TEE deployments

use generated::grpc::health::v1::{
    HealthCheckRequest, HealthCheckResponse,
    health_check_response::ServingStatus,
    health_server::{Health, HealthServer},
};
use generated::parser::{
    ParseRequest, ParseResponse,
    parser_service_server::{ParserService, ParserServiceServer},
};
use generated::tonic::{self, Request, Response, Status};
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
use std::net::SocketAddr;

/// Environment variable that must point at the ephemeral signing key file.
const EPHEMERAL_FILE_ENV: &str = "EPHEMERAL_FILE";

/// Error returned when the operator forgot to point the server at a real key.
#[derive(Debug)]
struct MissingEphemeralFile;

impl std::fmt::Display for MissingEphemeralFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{EPHEMERAL_FILE_ENV} is not set; refusing to start. \
             Point it at an operator-provisioned ephemeral key file. \
             Do not reuse the checked-in test fixture in production."
        )
    }
}

impl std::error::Error for MissingEphemeralFile {}

/// Resolve the ephemeral key file path from an environment lookup result.
///
/// Pure helper extracted so we can test the policy ("fail when unset") without
/// mutating process-wide env vars (which is racy across parallel test
/// threads).
fn resolve_ephemeral_file(
    env: Result<String, std::env::VarError>,
) -> Result<String, MissingEphemeralFile> {
    match env {
        Ok(path) if !path.is_empty() => Ok(path),
        _ => Err(MissingEphemeralFile),
    }
}

/// Standalone gRPC service that calls the parser directly
struct GrpcService {
    ephemeral_key: P256Pair,
}

/// Health check service - always returns SERVING
struct HealthService;

impl GrpcService {
    fn new(ephemeral_file: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let handle = EphemeralKeyHandle::new(ephemeral_file.to_string());
        let ephemeral_key = handle
            .get_ephemeral_key()
            .map_err(|e| format!("failed to load ephemeral key from {ephemeral_file}: {e:?}"))?;
        Ok(Self { ephemeral_key })
    }
}

#[tonic::async_trait]
impl ParserService for GrpcService {
    async fn parse(
        &self,
        request: Request<ParseRequest>,
    ) -> Result<Response<ParseResponse>, Status> {
        // Direct function call - no sockets needed
        parse(&request.into_inner(), &self.ephemeral_key)
            .map(Response::new)
            .map_err(|e| Status::new(tonic::Code::from_i32(e.code as i32), e.message))
    }
}

#[tonic::async_trait]
impl Health for HealthService {
    async fn check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        Ok(Response::new(HealthCheckResponse {
            status: ServingStatus::Serving as i32,
        }))
    }

    type WatchStream = tokio_stream::wrappers::ReceiverStream<Result<HealthCheckResponse, Status>>;

    async fn watch(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        Err(Status::unimplemented("watch is not supported"))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = "0.0.0.0:44020".parse()?;

    // Refuse to start without an explicit ephemeral key path. The repo ships a
    // P-256 fixture under `integration/fixtures/ephemeral.secret`; falling back
    // to it would mean every default deployment signs with a key any reader of
    // the repo can forge (PRS-233). Operators must opt in via EPHEMERAL_FILE.
    let ephemeral_file = resolve_ephemeral_file(std::env::var(EPHEMERAL_FILE_ENV))?;

    let svc = GrpcService::new(&ephemeral_file)?;

    let reflection_service = generated::tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(generated::FILE_DESCRIPTOR_SET)
        .build()
        .expect("failed to start reflection service");

    println!("parser_grpc_server {} listening on {addr}", env!("VERSION"));

    tonic::transport::Server::builder()
        .add_service(reflection_service)
        .add_service(HealthServer::new(HealthService))
        .add_service(ParserServiceServer::new(svc))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn resolve_ephemeral_file_errors_when_unset() {
        let err = resolve_ephemeral_file(Err(std::env::VarError::NotPresent)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("EPHEMERAL_FILE"),
            "error must name the env var, got: {msg}"
        );
        assert!(
            msg.contains("refusing to start"),
            "error must make the fail-closed behavior obvious, got: {msg}"
        );
    }

    #[test]
    fn resolve_ephemeral_file_errors_when_empty() {
        let err = resolve_ephemeral_file(Ok(String::new())).unwrap_err();
        assert!(err.to_string().contains("EPHEMERAL_FILE"));
    }

    #[test]
    fn resolve_ephemeral_file_errors_on_invalid_unicode() {
        let err = resolve_ephemeral_file(Err(std::env::VarError::NotUnicode(
            "bad".to_string().into(),
        )))
        .unwrap_err();
        assert!(err.to_string().contains("EPHEMERAL_FILE"));
    }

    #[test]
    fn resolve_ephemeral_file_returns_path_when_set() {
        let path = resolve_ephemeral_file(Ok("/etc/visualsign/ephemeral.secret".to_string()))
            .expect("non-empty path must succeed");
        assert_eq!(path, "/etc/visualsign/ephemeral.secret");
    }
}
