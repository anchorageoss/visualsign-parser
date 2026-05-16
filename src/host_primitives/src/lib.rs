//! Primitives for building Turnkey secure app gRPC host servers.

#![deny(clippy::all, clippy::unwrap_used)]

pub mod payment_marker;
pub mod turnkey;

use std::time::Duration;

use borsh::BorshDeserialize;
use prost::Message;
use qos_core::protocol::{ProtocolError, msg::ProtocolMsg};
use tonic::Status;

/// Buffer size for socket message queue.
pub static ENCLAVE_QUEUE_CAPACITY: usize = 12;
/// Maximum gRPC message size. Set to 25MB (25*1024*1024)
pub static GRPC_MAX_RECV_MSG_SIZE: usize = 26_214_400;

/// Send a message to a secure app via QOS proxy using the `Client`, without a tracing index
pub async fn send_proxy_request<Req, Resp>(
    request: Req,
    client: &qos_core::client::SocketClient,
) -> Result<Resp, tonic::Status>
where
    Resp: Message + Default,
    Req: Message,
{
    let encoded_qos_request = {
        let data = request.encode_to_vec();
        let qos_request = ProtocolMsg::ProxyRequest { data };

        borsh::to_vec(&qos_request)
            .map_err(|e| Status::internal(format!("Failed to serialize qos request: {e:?}")))?
    };

    let encoded_qos_response = client
        .call(&encoded_qos_request)
        .await
        .map_err(|e| Status::internal(format!("Failed to query enclave: {e:?}")))?;
    let qos_response = ProtocolMsg::try_from_slice(&encoded_qos_response)
        .map_err(|e| Status::internal(format!("Failed to deserialized enclave response: {e:?}")))?;

    let encoded_app_response = match qos_response {
        ProtocolMsg::ProxyResponse { data } => data,
        ProtocolMsg::ProtocolErrorResponse(ProtocolError::AppClientRecvTimeout) => {
            let msg = "AppClientRecvTimeout: QOS server app client time out: app likely panicked";
            eprintln!("{msg}");
            return Err(Status::internal(msg));
        }
        other => {
            return Err(Status::internal(format!(
                "Expected a ProtocolMsg::ProxyResponse but got {other:?}"
            )));
        }
    };

    Resp::decode(&*encoded_app_response)
        .map_err(|e| Status::internal(format!("Failed to deserialize enclave response: {e:?}")))
}

/// A default timeout for hosts to configure their qos protocol socket client with.
pub const fn enclave_client_timeout() -> Duration {
    qos_core::protocol::INITIAL_CLIENT_TIMEOUT
}
