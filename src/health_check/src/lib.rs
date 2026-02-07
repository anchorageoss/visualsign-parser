//! Services for health checking QOS hosts. Intended to be adding to secure app
//! hosts.

//! #![`forbid(unsafe_code)`]
#![deny(clippy::all, clippy::unwrap_used)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use generated::grpc::health::v1::{
    HealthCheckRequest as K8HealthCheckRequest, HealthCheckResponse as K8HealthCheckResponse,
    health_check_response::ServingStatus as K8ServingStatus,
    health_server::{Health as K8HealthService, HealthServer as K8HealthServer},
};
use generated::health::health_check_service_server::{
    HealthCheckService, HealthCheckServiceServer,
};
use generated::health::{
    AppHealthRequest, AppHealthResponse, HostHealthRequest, HostHealthResponse,
};
use generated::tonic;
use std::{pin::Pin, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::Stream;

const WATCH_STREAM_TIMEOUT_SEC: u64 = 3;
const STREAM_MSG_BUFFER_MAX: usize = 16;
/// k8s terminology to check if a service is up, but not necessarily ready to serve traffic.
pub const LIVENESS: &str = "liveness";
/// k8s terminology to check if a service is ready to serve traffic.
pub const READINESS: &str = "readiness";

/// Turnkeys health check service for performing primitive health checks via an
/// app host.
pub struct TkHealthCheck<T> {
    app_check: T,
}

impl<T> TkHealthCheck<T>
where
    T: AppHealthCheckable + Send + Sync + 'static,
{
    /// Create a new instance of [`Self`], with the given enclave
    /// (`enclave_addr`).
    #[must_use]
    pub fn build_service(app_check: T) -> HealthCheckServiceServer<TkHealthCheck<T>> {
        let inner = Self { app_check };
        HealthCheckServiceServer::new(inner)
    }
}

/// Something that can perform a health check on an app over a socket client.
pub trait AppHealthCheckable: Clone {
    /// Perform a health check on a enclave app.
    fn app_health_check(
        &self,
    ) -> impl std::future::Future<Output = Result<tonic::Response<AppHealthResponse>, tonic::Status>>
    + Send;
}

#[tonic::async_trait]
impl<T> HealthCheckService for TkHealthCheck<T>
where
    T: AppHealthCheckable + Send + Sync + 'static,
{
    async fn host_health(
        &self,
        _request: tonic::Request<HostHealthRequest>,
    ) -> Result<tonic::Response<HostHealthResponse>, tonic::Status> {
        let response = HostHealthResponse { code: 200 };
        Ok(tonic::Response::new(response))
    }

    async fn app_health(
        &self,
        _request: tonic::Request<AppHealthRequest>,
    ) -> Result<tonic::Response<AppHealthResponse>, tonic::Status> {
        self.app_check.app_health_check().await
    }
}

/// GRPC Health Checking Protocol
/// <https://github.com/grpc/grpc/blob/master/doc/health-checking.md>
#[derive(Clone)]
pub struct K8Health<T> {
    app_check: T,
}

impl<T> K8Health<T>
where
    T: AppHealthCheckable + Send + Sync + 'static,
{
    /// Create a new instance of [`Self`], with the given enclave
    /// (`enclave_addr`).
    #[must_use]
    pub fn build_service(app_check: T) -> K8HealthServer<K8Health<T>> {
        let inner = Self { app_check };
        K8HealthServer::new(inner)
    }

    async fn app_status(&self) -> K8ServingStatus {
        match self
            .app_check
            .app_health_check()
            .await
            .map(|resp| match resp.into_inner().code {
                200 => K8ServingStatus::Serving,
                _ => K8ServingStatus::NotServing,
            })
            .map_err(|_status| K8ServingStatus::NotServing)
        {
            Ok(s) | Err(s) => s,
        }
    }

    async fn k8_request(
        &self,
        request: &tonic::Request<K8HealthCheckRequest>,
    ) -> K8HealthCheckResponse {
        let status = match request.get_ref().service.as_str() {
            LIVENESS => K8ServingStatus::Serving,
            READINESS => self.app_status().await,
            _ => K8ServingStatus::ServiceUnknown,
        };

        K8HealthCheckResponse {
            status: status as i32,
        }
    }
}

type ResponseStream =
    Pin<Box<dyn Stream<Item = Result<K8HealthCheckResponse, tonic::Status>> + Send>>;

#[tonic::async_trait]
impl<T> K8HealthService for K8Health<T>
where
    T: AppHealthCheckable + Send + Sync + 'static,
{
    async fn check(
        &self,
        request: tonic::Request<K8HealthCheckRequest>,
    ) -> std::result::Result<tonic::Response<K8HealthCheckResponse>, tonic::Status> {
        Ok(tonic::Response::new(self.k8_request(&request).await))
    }

    type WatchStream = ResponseStream;

    async fn watch(
        &self,
        request: tonic::Request<K8HealthCheckRequest>,
    ) -> std::result::Result<tonic::Response<Self::WatchStream>, tonic::Status> {
        let (tx, rx) = mpsc::channel(STREAM_MSG_BUFFER_MAX);
        let self2 = self.clone();
        tokio::spawn(async move {
            loop {
                let status = self2.k8_request(&request).await;
                match tx.send(Ok(status)).await {
                    Ok(()) => {
                        // `status` was queued to be sent to the gRPC client
                    }
                    Err(_) => {
                        // `output_stream` was dropped, meaning the gRPC client
                        // connection closed. Since `output_stream` was built with
                        // `rx`, `tx.send` will error once `output_stream` (and
                        // therefore also `rx`) is dropped.
                        break;
                    }
                }

                tokio::time::sleep(Duration::from_secs(WATCH_STREAM_TIMEOUT_SEC)).await;
            }
        });

        let output_stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        Ok(tonic::Response::new(
            Box::pin(output_stream) as Self::WatchStream
        ))
    }
}
