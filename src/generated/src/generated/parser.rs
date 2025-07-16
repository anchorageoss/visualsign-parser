/// This is a bit odd, but needed for the QOS host.
/// (the QOS host receives messages which can be either parser responses or QOS-level responses)
/// TODO: can we remove the need for these?
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct QosParserRequest {
    #[prost(oneof = "qos_parser_request::Input", tags = "1, 2")]
    pub input: ::core::option::Option<qos_parser_request::Input>,
}
/// Nested message and enum types in `QOSParserRequest`.
pub mod qos_parser_request {
    #[cfg_attr(
        feature = "serde_derive",
        derive(::serde::Serialize, ::serde::Deserialize),
        serde(rename_all = "camelCase")
    )]
    #[cfg_attr(feature = "serde_derive", serde(untagged))]
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Input {
        #[prost(message, tag = "1")]
        ParseRequest(super::ParseRequest),
        #[prost(message, tag = "2")]
        HealthRequest(super::super::health::AppHealthRequest),
    }
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct QosParserResponse {
    #[prost(oneof = "qos_parser_response::Output", tags = "1, 3, 4")]
    pub output: ::core::option::Option<qos_parser_response::Output>,
}
/// Nested message and enum types in `QOSParserResponse`.
pub mod qos_parser_response {
    #[cfg_attr(
        feature = "serde_derive",
        derive(::serde::Serialize, ::serde::Deserialize),
        serde(rename_all = "camelCase")
    )]
    #[cfg_attr(feature = "serde_derive", serde(untagged))]
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Output {
        #[prost(message, tag = "1")]
        ParseResponse(super::ParseResponse),
        #[prost(message, tag = "3")]
        HealthResponse(super::super::health::AppHealthResponse),
        #[prost(message, tag = "4")]
        Status(super::super::google::rpc::Status),
    }
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ParseRequest {
    #[prost(string, tag = "1")]
    pub unsigned_payload: ::prost::alloc::string::String,
    #[prost(enumeration = "Chain", tag = "2")]
    pub chain: i32,
    #[prost(oneof = "parse_request::ChainMetadata", tags = "3, 4")]
    pub chain_metadata: ::core::option::Option<parse_request::ChainMetadata>,
}
/// Nested message and enum types in `ParseRequest`.
pub mod parse_request {
    #[cfg_attr(
        feature = "serde_derive",
        derive(::serde::Serialize, ::serde::Deserialize),
        serde(rename_all = "camelCase")
    )]
    #[cfg_attr(feature = "serde_derive", serde(untagged))]
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum ChainMetadata {
        #[prost(message, tag = "3")]
        Ethereum(super::EthereumMetadata),
        #[prost(message, tag = "4")]
        Solana(super::SolanaMetadata),
    }
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ParseResponse {
    #[prost(message, optional, tag = "1")]
    pub parsed_transaction: ::core::option::Option<ParsedTransaction>,
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Metadata {
    #[prost(string, tag = "1")]
    pub key: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub value: ::prost::alloc::string::String,
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ParsedTransactionPayload {
    #[prost(string, tag = "4")]
    pub signable_payload: ::prost::alloc::string::String,
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ParsedTransaction {
    #[prost(message, optional, tag = "1")]
    pub payload: ::core::option::Option<ParsedTransactionPayload>,
    #[prost(message, optional, tag = "2")]
    pub signature: ::core::option::Option<Signature>,
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Signature {
    #[prost(enumeration = "SignatureScheme", tag = "1")]
    pub scheme: i32,
    #[prost(string, tag = "2")]
    pub public_key: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub message: ::prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub signature: ::prost::alloc::string::String,
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EthereumMetadata {}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SolanaMetadata {}
/// Chain represents supported blockchain networks
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[cfg_attr(feature = "serde_derive", serde(untagged))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Chain {
    Unspecified = 0,
    Bitcoin = 1,
    Ethereum = 2,
    Solana = 3,
    Sui = 4,
    /// Custom for extensibility
    Custom = 999,
}
impl Chain {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Chain::Unspecified => "CHAIN_UNSPECIFIED",
            Chain::Bitcoin => "CHAIN_BITCOIN",
            Chain::Ethereum => "CHAIN_ETHEREUM",
            Chain::Solana => "CHAIN_SOLANA",
            Chain::Sui => "CHAIN_SUI",
            Chain::Custom => "CHAIN_CUSTOM",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "CHAIN_UNSPECIFIED" => Some(Self::Unspecified),
            "CHAIN_BITCOIN" => Some(Self::Bitcoin),
            "CHAIN_ETHEREUM" => Some(Self::Ethereum),
            "CHAIN_SOLANA" => Some(Self::Solana),
            "CHAIN_SUI" => Some(Self::Sui),
            "CHAIN_CUSTOM" => Some(Self::Custom),
            _ => None,
        }
    }
}
#[cfg_attr(
    feature = "serde_derive",
    derive(::serde::Serialize, ::serde::Deserialize),
    serde(rename_all = "camelCase")
)]
#[cfg_attr(feature = "serde_derive", serde(untagged))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SignatureScheme {
    Unspecified = 0,
    /// Scheme used for Turnkey app proofs
    TurnkeyP256EphemeralKey = 1,
}
impl SignatureScheme {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            SignatureScheme::Unspecified => "SIGNATURE_SCHEME_UNSPECIFIED",
            SignatureScheme::TurnkeyP256EphemeralKey => {
                "SIGNATURE_SCHEME_TURNKEY_P256_EPHEMERAL_KEY"
            }
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "SIGNATURE_SCHEME_UNSPECIFIED" => Some(Self::Unspecified),
            "SIGNATURE_SCHEME_TURNKEY_P256_EPHEMERAL_KEY" => {
                Some(Self::TurnkeyP256EphemeralKey)
            }
            _ => None,
        }
    }
}
/// Generated client implementations.
#[cfg(feature = "tonic_types")]
pub mod parser_service_client {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    #[derive(Debug, Clone)]
    pub struct ParserServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl ParserServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> ParserServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> ParserServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
            >>::Error: Into<StdError> + Send + Sync,
        {
            ParserServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        pub async fn parse(
            &mut self,
            request: impl tonic::IntoRequest<super::ParseRequest>,
        ) -> std::result::Result<tonic::Response<super::ParseResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/parser.ParserService/Parse",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("parser.ParserService", "Parse"));
            self.inner.unary(req, path, codec).await
        }
    }
}
/// Generated server implementations.
#[cfg(feature = "tonic_types")]
pub mod parser_service_server {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with ParserServiceServer.
    #[async_trait]
    pub trait ParserService: Send + Sync + 'static {
        async fn parse(
            &self,
            request: tonic::Request<super::ParseRequest>,
        ) -> std::result::Result<tonic::Response<super::ParseResponse>, tonic::Status>;
    }
    #[derive(Debug)]
    pub struct ParserServiceServer<T: ParserService> {
        inner: _Inner<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    struct _Inner<T>(Arc<T>);
    impl<T: ParserService> ParserServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            let inner = _Inner(inner);
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for ParserServiceServer<T>
    where
        T: ParserService,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::BoxBody>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = self.inner.clone();
            match req.uri().path() {
                "/parser.ParserService/Parse" => {
                    #[allow(non_camel_case_types)]
                    struct ParseSvc<T: ParserService>(pub Arc<T>);
                    impl<
                        T: ParserService,
                    > tonic::server::UnaryService<super::ParseRequest> for ParseSvc<T> {
                        type Response = super::ParseResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ParseRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move { (*inner).parse(request).await };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let inner = inner.0;
                        let method = ParseSvc(inner);
                        let codec = tonic::codec::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        Ok(
                            http::Response::builder()
                                .status(200)
                                .header("grpc-status", "12")
                                .header("content-type", "application/grpc")
                                .body(empty_body())
                                .unwrap(),
                        )
                    })
                }
            }
        }
    }
    impl<T: ParserService> Clone for ParserServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    impl<T: ParserService> Clone for _Inner<T> {
        fn clone(&self) -> Self {
            Self(Arc::clone(&self.0))
        }
    }
    impl<T: std::fmt::Debug> std::fmt::Debug for _Inner<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
    impl<T: ParserService> tonic::server::NamedService for ParserServiceServer<T> {
        const NAME: &'static str = "parser.ParserService";
    }
}
