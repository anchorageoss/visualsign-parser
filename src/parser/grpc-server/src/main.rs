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
use parser_app::config::ParserConfig;
use parser_app::routes::parse::parse;
use qos_core::handles::EphemeralKeyHandle;
use qos_p256::P256Pair;
use std::net::SocketAddr;
use visualsign::signing::MetadataTrustPolicy;

/// Standalone gRPC service that calls the parser directly
struct GrpcService {
    ephemeral_key: P256Pair,
    config: ParserConfig,
}

/// Health check service - always returns SERVING
struct HealthService;

impl GrpcService {
    fn new(ephemeral_file: &str, config: ParserConfig) -> Self {
        let handle = EphemeralKeyHandle::new(ephemeral_file.to_string());
        let ephemeral_key = handle
            .get_ephemeral_key()
            .expect("Failed to load ephemeral key");
        Self {
            ephemeral_key,
            config,
        }
    }
}

#[tonic::async_trait]
impl ParserService for GrpcService {
    async fn parse(
        &self,
        request: Request<ParseRequest>,
    ) -> Result<Response<ParseResponse>, Status> {
        // Direct function call - no sockets needed
        parse(&request.into_inner(), &self.ephemeral_key, &self.config)
            .map(Response::new)
            .map_err(|e| Status::new(tonic::Code::from(e.code as i32), e.message))
    }
}

/// Cmdline surface of this binary: the ABI trust posture, and nothing else.
const USAGE: &str = "usage: parser_grpc_server \
     [--accept-unsigned-abis | --accept-signatures-from-pubkey <hex> ...]";

/// Resolve the ABI trust posture from this server's own cmdline, mirroring
/// `parser_app`'s `--accept-unsigned-abis` / `--accept-signatures-from-pubkey`.
///
/// Unlike `parser_app`, omitting both is allowed and falls back to accept-unsigned
/// with a loud log line. This binary is the non-TEE deployment: there is no
/// attestation and no signed manifest to audit the posture against, so the
/// signer-verifiability argument that makes an explicit choice mandatory in the
/// enclave does not apply here, and requiring the flag would only break local dev.
///
/// Hand-rolled rather than pulled in via clap: two options, and this binary
/// otherwise has no CLI surface at all.
fn abi_trust_from_args() -> Result<MetadataTrustPolicy, String> {
    let mut accept_unsigned = false;
    let mut signer_pubkeys: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--accept-unsigned-abis" => accept_unsigned = true,
            "--accept-signatures-from-pubkey" => {
                let key = args
                    .next()
                    .ok_or("--accept-signatures-from-pubkey needs a hex public key")?;
                signer_pubkeys.push(key);
            }
            other => return Err(format!("unexpected argument '{other}'; {USAGE}")),
        }
    }

    if !accept_unsigned && signer_pubkeys.is_empty() {
        println!(
            "no ABI trust posture given; defaulting to --accept-unsigned-abis. \
             This is the non-attested dev server; the enclave binary (parser_app) \
             requires the choice to be explicit."
        );
        return Ok(MetadataTrustPolicy::AcceptUnsigned);
    }

    ParserConfig::abi_trust_from_options(accept_unsigned, &signer_pubkeys)
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

    // Use the test fixture for development; in production, use EPHEMERAL_KEY_FILE
    let ephemeral_file = std::env::var("EPHEMERAL_FILE")
        .unwrap_or_else(|_| "integration/fixtures/ephemeral.secret".to_string());

    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }

    let abi_trust = abi_trust_from_args().map_err(|e| format!("invalid ABI trust config: {e}"))?;
    println!("caller-supplied ABI trust: {abi_trust}");

    let svc = GrpcService::new(&ephemeral_file, ParserConfig::new(abi_trust));

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
