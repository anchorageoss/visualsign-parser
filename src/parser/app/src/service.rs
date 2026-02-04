//! Parser
use std::sync::Arc;

use generated::{
    google::rpc::{Code, Status},
    health::AppHealthResponse,
    parser::{QosParserRequest, QosParserResponse, qos_parser_request, qos_parser_response},
};
use qos_core::handles::EphemeralKeyHandle;
use tokio::sync::RwLock;

/// Struct holding a request processor for QOS
#[derive(Debug)]
pub struct Processor {
    handle: EphemeralKeyHandle,
}

/// `Processor` shared between tasks
pub type SharedProcessor = Arc<RwLock<Processor>>;

impl Processor {
    /// Creates a new request processor. The only argument needed is an ephemeral key handle.
    #[must_use]
    pub fn new(handle: EphemeralKeyHandle) -> SharedProcessor {
        Arc::new(RwLock::new(Self { handle }))
    }
}

impl Processor {
    /// Process a `QosParserRequest` and respond with `QosParserResponse`
    #[must_use]
    pub fn process(&self, request: &QosParserRequest) -> QosParserResponse {
        // We're doing a potentially CPU intensive blocking task, we shouldn't just lock the runtime
        tokio::task::block_in_place(move || {
            let ephemeral_key = match self
                .handle
                .get_ephemeral_key()
                .map_err(|e| {
                    qos_parser_response::Output::Status(Status {
                        code: Code::Internal as i32,
                        message: format!("unable to get ephemeral key: {e:?}"),
                        details: vec![],
                    })
                })
                .map_err(|output| QosParserResponse {
                    output: Some(output),
                }) {
                Ok(input) => input,
                Err(err_resp) => return err_resp,
            };

            let input = match request
                .input
                .as_ref()
                .ok_or({
                    qos_parser_response::Output::Status(Status {
                        code: Code::InvalidArgument as i32,
                        message: "missing request input".to_string(),
                        details: vec![],
                    })
                })
                .map_err(|o| QosParserResponse { output: Some(o) })
            {
                Ok(input) => input,
                Err(err_resp) => return err_resp,
            };

            let output = match input {
                qos_parser_request::Input::ParseRequest(parse_request) => {
                    match crate::routes::parse::parse(parse_request, &ephemeral_key)
                        .map(qos_parser_response::Output::ParseResponse)
                        .map_err(|e| {
                            qos_parser_response::Output::Status(Status {
                                code: e.code as i32,
                                message: e.message,
                                details: vec![],
                            })
                        }) {
                        Ok(o) | Err(o) => o,
                    }
                }
                qos_parser_request::Input::HealthRequest(_) => {
                    qos_parser_response::Output::HealthResponse(AppHealthResponse { code: 200 })
                }
            };

            QosParserResponse {
                output: Some(output),
            }
        })
    }
}
