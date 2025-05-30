//! Parser Host.

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::missing_panics_doc
)]

use generated::health::{AppHealthRequest, AppHealthResponse};
use generated::parser::{
    ParseRequest, ParseResponse, QosParserRequest, QosParserResponse, parser_service_server,
    qos_parser_request, qos_parser_response,
};
use generated::tonic;
use generated::tonic::{Request, Response, Status};
use health_check::AppHealthCheckable;
use host_primitives::{GRPC_MAX_RECV_MSG_SIZE, enclave_client_timeout};
use metrics::request;
use qos_core::{client::Client as SocketClient, io::SocketAddress};
use std::{
    sync::mpsc::{SyncSender, sync_channel},
    thread,
    time::Instant,
};

use tokio::sync::oneshot::{self, Sender};
use tokio::{
    signal::unix::{SignalKind, signal},
    spawn,
};

type SocketMessage = host_primitives::SocketMessage<QosParserRequest, QosParserResponse>;
/// Host `gRPC` server.
#[derive(Debug)]
pub struct Host {
    response_channel: SyncSender<SocketMessage>,
}

impl Host {
    fn new(response_channel: SyncSender<SocketMessage>) -> Self {
        Self { response_channel }
    }

    /// Start the host server.
    pub async fn listen(
        listen_addr: std::net::SocketAddr,
        enclave_addr: SocketAddress,
    ) -> Result<(), tonic::transport::Error> {
        let reflection_service = generated::tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(generated::FILE_DESCRIPTOR_SET)
            .build()
            .expect("failed to start reflection service");

        let (response_channel, receiver) =
            sync_channel::<SocketMessage>(host_primitives::SOCKET_MESSAGE_QUEUE_BUFFER_SIZE);

        let app_checker = ParserHealth {
            response_channel: response_channel.clone(),
        };
        let health_check_service =
            health_check::TkHealthCheck::build_service(enclave_addr.clone(), app_checker.clone());
        let k8_health_service = health_check::K8Health::build_service(app_checker);

        let host = Host::new(response_channel);

        thread::spawn(move || {
            let client = SocketClient::new(enclave_addr, enclave_client_timeout());

            loop {
                let SocketMessage { replier, request } =
                    receiver.recv().expect("failed to receive message");

                let enclave_resp = host_primitives::send_proxy_request(request, &client);

                replier
                    .send(enclave_resp)
                    .expect("message processor failed");
            }
        });

        println!("HostServer listening on {listen_addr}");

        let (sigterm_sender, sigterm_receiver) = oneshot::channel();
        spawn(Self::wait_for_sigterm(sigterm_sender));

        tonic::transport::Server::builder()
            .add_service(reflection_service)
            .add_service(
                parser_service_server::ParserServiceServer::new(host)
                    .max_decoding_message_size(GRPC_MAX_RECV_MSG_SIZE),
            )
            .add_service(health_check_service)
            .add_service(k8_health_service)
            .serve_with_shutdown(listen_addr, async {
                sigterm_receiver.await.ok();
                println!("SIGTERM received");
            })
            .await
    }

    async fn wait_for_sigterm(sender: Sender<()>) {
        let _ = signal(SignalKind::terminate())
            .expect("failed to create SIGTERM signal handler")
            .recv()
            .await;
        println!("SIGTERM signal handled, forwarding to host server");
        let _ = sender.send(());
    }
}

#[tonic::async_trait]
impl parser_service_server::ParserService for Host {
    async fn parse(
        &self,
        request: Request<ParseRequest>,
    ) -> Result<Response<ParseResponse>, Status> {
        let now = Instant::now();

        let request = QosParserRequest {
            input: Some(qos_parser_request::Input::ParseRequest(
                request.into_inner(),
            )),
        };

        let request_decode_elapsed = now.elapsed();

        let now_step = Instant::now();

        let output = host_primitives::send_socket_message(request, &self.response_channel)
            .map_err(|e| Status::internal(format!("Parse: unexpected socket failure: {e:?}")))?
            .output
            .ok_or_else(|| Status::internal("QosParserResponse::output was None"))?;

        let send_message_elapsed = now_step.elapsed();

        let now_step = Instant::now();

        #[allow(clippy::match_wildcard_for_single_variants)]
        let response = match output {
            qos_parser_response::Output::ParseResponse(response) => Ok(Response::new(response)),
            qos_parser_response::Output::Status(status) => Err(Status::from(status)),
            _ => Err(Status::internal(format!(
                "Unexpected response from enclave: {output:?}",
            ))),
        };

        let response_encode_elapsed = now_step.elapsed();

        request::track_enclave_request("parse", response.is_ok(), now.elapsed());
        request::track_enclave_details(
            "parse",
            response.is_ok(),
            "request_decode",
            request_decode_elapsed,
        );
        request::track_enclave_details(
            "parse",
            response.is_ok(),
            "send_message",
            send_message_elapsed,
        );
        request::track_enclave_details(
            "parse",
            response.is_ok(),
            "response_encode",
            response_encode_elapsed,
        );

        response
    }
}

#[derive(Clone)]
struct ParserHealth {
    response_channel: SyncSender<SocketMessage>,
}

impl AppHealthCheckable for ParserHealth {
    fn app_health_check(&self) -> Result<tonic::Response<AppHealthResponse>, Status> {
        let now = Instant::now();

        let request = QosParserRequest {
            input: Some(qos_parser_request::Input::HealthRequest(
                AppHealthRequest {},
            )),
        };

        let output = host_primitives::send_socket_message(request, &self.response_channel)
            .map_err(|e| Status::internal(format!("App Health: unexpected socket failure: {e:?}")))?
            .output
            .ok_or_else(|| Status::internal("QosParserResponse::output was None"))?;

        #[allow(clippy::match_wildcard_for_single_variants)]
        let response = match output {
            qos_parser_response::Output::HealthResponse(response) => {
                Ok(tonic::Response::new(response))
            }
            qos_parser_response::Output::Status(status) => Err(Status::from(status)),
            _ => Err(Status::internal(format!(
                "unexpected health check response: {output:?}"
            ))),
        };

        request::track_enclave_request("health", response.is_ok(), now.elapsed());

        response
    }
}
