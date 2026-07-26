//! Parser
use std::sync::Arc;

use generated::{
    google::rpc::{Code, Status},
    health::AppHealthResponse,
    parser::{QosParserRequest, QosParserResponse, qos_parser_request, qos_parser_response},
};
use qos_core::handles::EphemeralKeyHandle;
use tokio::sync::RwLock;
use visualsign::registry::TransactionConverterRegistry;

/// Builds the converter registry a [`Processor`] serves. Invoked once per
/// request, never cached: every request gets freshly constructed converters,
/// so no converter state can outlive a request. That isolation is deliberate
/// -- this runs inside an enclave where a process restart is not an available
/// mitigation for accumulated state.
pub(crate) type RegistryFactory = Box<dyn Fn() -> TransactionConverterRegistry + Send + Sync>;

/// Struct holding a request processor for QOS
pub struct Processor {
    handle: EphemeralKeyHandle,
    registry_factory: RegistryFactory,
}

impl std::fmt::Debug for Processor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Processor")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

/// `Processor` shared between tasks
pub type SharedProcessor = Arc<RwLock<Processor>>;

impl Processor {
    /// Creates a request processor serving this workspace's built-in chains.
    #[must_use]
    pub fn new(handle: EphemeralKeyHandle) -> SharedProcessor {
        Self::with_registry_factory(handle, Box::new(crate::registry::create_registry))
    }

    /// Creates a request processor serving a caller-provided registry factory;
    /// reached from outside the crate via `Cli::execute_with_registry_factory`
    /// (the `external-chains` composition entry point).
    #[must_use]
    pub(crate) fn with_registry_factory(
        handle: EphemeralKeyHandle,
        registry_factory: RegistryFactory,
    ) -> SharedProcessor {
        Arc::new(RwLock::new(Self {
            handle,
            registry_factory,
        }))
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
                    match crate::routes::parse::parse_with_registry(
                        parse_request,
                        &ephemeral_key,
                        &(self.registry_factory)(),
                    )
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{
        Chain as ProtoChain, ParseRequest, QosParserRequest, qos_parser_request,
        qos_parser_response,
    };
    use visualsign::errors::VisualSignError;
    use visualsign::registry::Chain;
    use visualsign::vsptrait::{
        ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
        VisualSignConverterFromString, VisualSignOptions,
    };
    use visualsign::{
        SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
        SignablePayloadFieldTextV2,
    };

    #[derive(Debug, Clone)]
    struct EchoTransaction;

    impl Transaction for EchoTransaction {
        fn from_string(_data: &str) -> Result<Self, TransactionParseError> {
            Ok(Self)
        }

        fn transaction_type(&self) -> String {
            "Echo".to_string()
        }
    }

    struct EchoConverter;

    impl VisualSignConverter<EchoTransaction> for EchoConverter {
        fn to_visual_sign_payload(
            &self,
            _transaction: EchoTransaction,
            _options: VisualSignOptions,
        ) -> Result<ConversionResult, VisualSignError> {
            Ok(ConversionResult::new(SignablePayload::new(
                0,
                "Echo".to_string(),
                None,
                vec![SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "ok".to_string(),
                        label: "Echo".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "ok".to_string(),
                    },
                }],
                "Echo".to_string(),
            )))
        }
    }

    impl VisualSignConverterFromString<EchoTransaction> for EchoConverter {}

    fn ephemeral_handle() -> EphemeralKeyHandle {
        EphemeralKeyHandle::new(format!(
            "{}/../../integration/fixtures/ephemeral.secret",
            env!("CARGO_MANIFEST_DIR")
        ))
    }

    fn parse_request(chain: ProtoChain, custom_chain_name: Option<&str>) -> QosParserRequest {
        QosParserRequest {
            input: Some(qos_parser_request::Input::ParseRequest(ParseRequest {
                unsigned_payload: "stub".to_string(),
                chain: chain as i32,
                chain_metadata: None,
                include_intermediate_output: false,
                custom_chain_name: custom_chain_name.map(str::to_string),
            })),
        }
    }

    /// `process()` serves the registry factory the `Processor` was built with
    /// -- the seam a downstream enclave binary composes through
    /// (`Cli::execute_with_registry_factory`).
    #[tokio::test(flavor = "multi_thread")]
    async fn process_serves_the_injected_registry() {
        let processor = Processor::with_registry_factory(
            ephemeral_handle(),
            Box::new(|| {
                let mut registry = TransactionConverterRegistry::new();
                registry.register::<EchoTransaction, _>(
                    Chain::Custom("near".to_string()),
                    EchoConverter,
                );
                registry
            }),
        );

        let response = processor
            .read()
            .await
            .process(&parse_request(ProtoChain::Custom, Some("near")));
        match response.output {
            Some(qos_parser_response::Output::ParseResponse(r)) => {
                assert!(r.parsed_transaction.is_some());
            }
            other => panic!("expected ParseResponse, got {other:?}"),
        }
    }

    /// A chain absent from the injected registry is rejected -- proving the
    /// injected factory (not the built-in default) serves requests.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_rejects_chains_absent_from_injected_registry() {
        let processor = Processor::with_registry_factory(
            ephemeral_handle(),
            Box::new(TransactionConverterRegistry::new),
        );

        let response = processor
            .read()
            .await
            .process(&parse_request(ProtoChain::Tron, None));
        match response.output {
            Some(qos_parser_response::Output::Status(s)) => {
                assert!(
                    s.message.contains("No converter registered"),
                    "unexpected message: {}",
                    s.message
                );
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }
}
