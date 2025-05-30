//! Utilities for integration tests
#![forbid(unsafe_code)]
#![deny(clippy::all)] // don't deny unwraps for integration testing
#![warn(missing_docs, clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_excessive_bools,
    clippy::missing_panics_doc
)]

use std::net::TcpListener;
use std::ops::Range;
use std::thread;
use std::time::Duration;
use std::{fs, future::Future, panic::AssertUnwindSafe, process::Command};

use futures::future::FutureExt;
use generated::health::health_check_service_client::HealthCheckServiceClient;
use generated::health::{HealthCheckRequest, HealthCheckResponse};
use generated::health::{health_check_response::ServingStatus, health_client::HealthClient};
use generated::parser::parser_service_client::ParserServiceClient;

use host_primitives::GRPC_MAX_RECV_MSG_SIZE;
use qos_core::protocol::services::boot::{Manifest, ManifestEnvelope, MemberPubKey, PatchSet};
use qos_p256::P256Pair;
use qos_test_primitives::PathWrapper;
use tonic::transport::Channel;

const MAX_PORT_BIND_WAIT_TIME: Duration = Duration::from_secs(90);
const PORT_BIND_WAIT_TIME_INCREMENT: Duration = Duration::from_millis(500);
const POST_BIND_SLEEP: Duration = Duration::from_millis(500);
const SERVER_PORT_RANGE: Range<u16> = 10000..60000;
const MAX_PORT_SEARCH_ATTEMPTS: u16 = 50;

/// Wrapper type for [`std::process::Child`] that kills the process on drop.
#[derive(Debug)]
pub struct ChildWrapper(std::process::Child);

impl From<std::process::Child> for ChildWrapper {
    fn from(child: std::process::Child) -> Self {
        Self(child)
    }
}

impl Drop for ChildWrapper {
    fn drop(&mut self) {
        // Kill the process and explicitly ignore the result
        drop(self.0.kill());
    }
}

/// Get a bind-able TCP port on the local system.
#[must_use]
pub fn find_free_port() -> Option<u16> {
    for _ in 0..MAX_PORT_SEARCH_ATTEMPTS {
        let port = rand::random_range(SERVER_PORT_RANGE);
        if port_is_available(port) {
            return Some(port);
        }
    }

    None
}

/// Wait until the given `port` is bound. Helpful for telling if something is
/// listening on the given port.
///
/// # Panics
///
/// Panics if the the port is not bound to within `MAX_PORT_BIND_WAIT_TIME`.
pub fn wait_until_port_is_bound(port: u16) {
    let mut wait_time = PORT_BIND_WAIT_TIME_INCREMENT;

    while wait_time < MAX_PORT_BIND_WAIT_TIME {
        thread::sleep(wait_time);
        if port_is_available(port) {
            wait_time += PORT_BIND_WAIT_TIME_INCREMENT;
        } else {
            thread::sleep(POST_BIND_SLEEP);
            return;
        }
    }
    panic!(
        "Server has not come up: port {} is still available after {}s",
        port,
        MAX_PORT_BIND_WAIT_TIME.as_secs()
    )
}

/// Return wether or not the port can be bind-ed too.
fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

const HOST_IP: &str = "127.0.0.1";
const SIMULATOR_ENCLAVE_PATH: &str = "../target/debug/simulator_enclave";

/// Arguments passed to the `test` function in [`Builder::execute`].
#[derive(Default)]
pub struct TestArgs {
    /// A client for the parser server
    pub parser_client: Option<ParserServiceClient<Channel>>,
    /// A client for the tls fetcher server
    pub health_check_client: Option<HealthCheckServiceClient<Channel>>,
    /// A client for canonical gRPC health check service
    /// See <https://github.com/grpc/grpc/blob/master/doc/health-checking.md>
    pub k8_health_client: Option<HealthClient<Channel>>,
}

/// Test harness builder.
#[derive(Default)]
pub struct Builder {}

impl Builder {
    /// Create a new instance of [`Self`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute `test`.
    ///
    /// Note this test env builder relies on binaries from other crates already
    /// being built and existing in the target directory. Thus any test that
    /// uses this should only be called after the whole workspace is compiled.
    /// This can be accomplished by either running `cargo build` or `cargo test`
    /// in the workspace root. However if you just test this crate (`cargo test
    /// -p integration`), this might fail because we don't build the
    /// binaries from the other crates.
    ///
    /// # Panics
    ///
    /// Panics if `test` panics or any of the binaries started panics.
    pub async fn execute<F, T>(self, test: F)
    where
        F: Fn(TestArgs) -> T,
        T: Future<Output = ()>,
    {
        let test_id = format!("{:?}", rand::random::<u64>());
        let mut process_handles = vec![];
        let mut file_handles = vec![];

        // TODO: is this needed?
        // We pass manifest_path as an argument in some apps for testing
        // but I suspect we can do without it here.
        let _ = setup_manifest(&test_id);

        let mut test_args = TestArgs {
            ..Default::default()
        };

        let app_sock_path = format!("./{test_id}.parser.app.sock");
        file_handles.push(app_sock_path.clone());
        let enclave_sock_path = format!("./{test_id}.parser.enclave.sock");
        file_handles.push(enclave_sock_path.clone());

        // Start parser enclave (simulator)
        let enclave_process: ChildWrapper = Command::new(SIMULATOR_ENCLAVE_PATH)
            .arg(&enclave_sock_path)
            .arg(&app_sock_path)
            .spawn()
            .unwrap()
            .into();
        process_handles.push(enclave_process);

        // Start parser secure app
        let parser_process: ChildWrapper = Command::new("../target/debug/parser_app")
            .arg("--usock")
            .arg(&app_sock_path)
            .arg("--ephemeral-file")
            .arg("./fixtures/ephemeral.secret")
            .spawn()
            .unwrap()
            .into();
        process_handles.push(parser_process);

        // Start parser host
        let host_port = find_free_port().unwrap();
        let host_process: ChildWrapper = Command::new("../target/debug/parser_host")
            .arg("--host-ip")
            .arg(HOST_IP)
            .arg("--host-port")
            .arg(host_port.to_string())
            .arg("--usock")
            .arg(&enclave_sock_path)
            .spawn()
            .unwrap()
            .into();
        process_handles.push(host_process);
        wait_until_port_is_bound(host_port);

        let host_addr = format!("http://{HOST_IP}:{host_port}");

        let health_check_client = HealthCheckServiceClient::connect(host_addr.clone())
            .await
            .unwrap();

        test_args.health_check_client = Some(health_check_client);

        let k8_health_client = HealthClient::connect(host_addr.clone()).await.unwrap();
        test_args.k8_health_client = Some(k8_health_client);

        let parser_client = ParserServiceClient::connect(host_addr)
            .await
            .unwrap()
            .max_decoding_message_size(GRPC_MAX_RECV_MSG_SIZE);

        test_args.parser_client = Some(parser_client);

        // Note: this isn't actually unwind safe. However, since we don't
        // attempt to access any memory from `test` that may get corrupted
        // by a panic, it is ok to ignore the compiler.
        let res = AssertUnwindSafe(test(test_args)).catch_unwind().await;

        for path in file_handles {
            drop(fs::remove_file(path));
        }

        assert!(res.is_ok());
    }
}

fn setup_manifest(test_id: &str) -> PathWrapper {
    let path: PathWrapper = format!("./{test_id}.manifest_envelope").into();
    let (patch_set, _) = make_patch_set(3, 2);
    let manifest = Manifest {
        patch_set,
        ..Default::default()
    };

    let envelope = borsh::to_vec(&ManifestEnvelope {
        manifest,
        ..Default::default()
    })
    .unwrap();
    fs::write(&*path, envelope).expect("failed to write manifest envelope to disk");

    path
}

/// Make a manifest set and get the associated key pairs.
#[must_use]
pub fn make_patch_set(member_count: usize, threshold: u32) -> (PatchSet, Vec<P256Pair>) {
    let pairs: Vec<_> = (0..member_count)
        .map(|_| P256Pair::generate().unwrap())
        .collect();

    let members = pairs
        .iter()
        .map(|p| MemberPubKey {
            pub_key: p.public_key().to_bytes(),
        })
        .collect();

    (PatchSet { threshold, members }, pairs)
}

/// Test the k8s health endpoints.
pub async fn k8_health(test_args: TestArgs) {
    use health_check::{LIVENESS, READINESS};
    let mut client = test_args.k8_health_client.unwrap();

    let request = tonic::Request::new(HealthCheckRequest {
        service: LIVENESS.to_string(),
    });
    let response = client.check(request).await;
    assert_eq!(
        response.unwrap().into_inner(),
        HealthCheckResponse {
            status: ServingStatus::Serving as i32
        }
    );

    let request = tonic::Request::new(HealthCheckRequest {
        service: READINESS.to_string(),
    });
    let response = client.check(request).await;
    assert_eq!(
        response.unwrap().into_inner(),
        HealthCheckResponse {
            status: ServingStatus::Serving as i32
        }
    );

    let request = tonic::Request::new(HealthCheckRequest {
        service: "signer".to_string(),
    });
    let response = client.check(request).await;
    assert_eq!(
        response.unwrap().into_inner(),
        HealthCheckResponse {
            status: ServingStatus::ServiceUnknown as i32
        }
    );

    let request = tonic::Request::new(HealthCheckRequest {
        service: LIVENESS.to_string(),
    });
    let response = client
        .watch(request)
        .await
        .unwrap()
        .into_inner()
        .message()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response,
        HealthCheckResponse {
            status: ServingStatus::Serving as i32
        }
    );

    let request = tonic::Request::new(HealthCheckRequest {
        service: READINESS.to_string(),
    });
    let response = client
        .watch(request)
        .await
        .unwrap()
        .into_inner()
        .message()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response,
        HealthCheckResponse {
            status: ServingStatus::Serving as i32
        }
    );

    let request = tonic::Request::new(HealthCheckRequest {
        service: "other".to_string(),
    });
    let response = client
        .watch(request)
        .await
        .unwrap()
        .into_inner()
        .message()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response,
        HealthCheckResponse {
            status: ServingStatus::ServiceUnknown as i32
        }
    );
}
