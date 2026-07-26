//! CLI for the parser app
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

use qos_core::{
    EPHEMERAL_KEY_FILE,
    cli::EPHEMERAL_FILE_OPT,
    handles::EphemeralKeyHandle,
    parser::{GetParserForOptions, OptionsParser, Parser, Token},
};
#[cfg(feature = "external-chains")]
use visualsign::registry::TransactionConverterRegistry;

const HOST_IP: &str = "host-ip";
const HOST_PORT: &str = "host-port";

/// CLI options for starting up the app server.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct ParserOpts {
    parsed: Parser,
}

impl ParserOpts {
    fn new(args: &mut Vec<String>) -> Self {
        let parsed =
            OptionsParser::<ParserParser>::parse(args).expect("Parser: Entered invalid CLI args");

        Self { parsed }
    }

    /// Address the host server should listen on.
    fn host_addr(&self) -> SocketAddr {
        let ip = Ipv4Addr::from_str(&self.ip()).expect("could not parse ip to IP v4");
        let port = self
            .port()
            .parse::<u16>()
            .expect("could not parse port to u16");
        SocketAddr::new(IpAddr::V4(ip), port)
    }

    fn ip(&self) -> String {
        self.parsed
            .single(HOST_IP)
            .expect("host ip required")
            .clone()
    }

    fn port(&self) -> String {
        self.parsed
            .single(HOST_PORT)
            .expect("host port required")
            .clone()
    }

    fn ephemeral_file(&self) -> String {
        self.parsed
            .single(EPHEMERAL_FILE_OPT)
            .expect("has a default value.")
            .clone()
    }
}

struct ParserParser;
impl GetParserForOptions for ParserParser {
    fn parser() -> Parser {
        Parser::new()
            .token(
                Token::new(HOST_IP, "IP address this server should listen on")
                    .takes_value(true)
                    .required(true),
            )
            .token(
                Token::new(HOST_PORT, "port this server should listen on")
                    .takes_value(true)
                    .required(true),
            )

            .token(
                Token::new(
                    EPHEMERAL_FILE_OPT,
                    "path to file where the Ephemeral Key secret should be retrieved from. Use default for production.",
                )
                .takes_value(true)
                .default_value(EPHEMERAL_KEY_FILE),
            )
    }
}

/// app cli
pub struct Cli;
impl Cli {
    /// start the parser app
    ///
    /// # Panics
    ///
    /// Panics if the socket server cannot start
    pub async fn execute() {
        Self::execute_with(Box::new(crate::registry::create_registry)).await;
    }

    /// Same as [`Self::execute`] but serving a caller-provided registry
    /// factory: the composition entry point for a downstream workspace's own
    /// enclave binary. Such a binary extends the built-in set with its own
    /// chains and inherits this crate's full serving stack (CLI args, host
    /// server, health checks, signing) unchanged:
    ///
    /// ```ignore
    /// parser_app::cli::Cli::execute_with_registry_factory(|| {
    ///     let mut registry = parser_app::registry::create_registry();
    ///     registry.register::<NearTransaction, _>(
    ///         Chain::Custom("near".to_string()),
    ///         NearVisualSignConverter,
    ///     );
    ///     registry
    /// })
    /// .await;
    /// ```
    ///
    /// The factory runs once per request, never cached, so converter state
    /// cannot outlive a request (see `service::RegistryFactory`).
    ///
    /// # Panics
    ///
    /// Panics if the socket server cannot start
    #[cfg(feature = "external-chains")]
    pub async fn execute_with_registry_factory(
        registry_factory: impl Fn() -> TransactionConverterRegistry + Send + Sync + 'static,
    ) {
        Self::execute_with(Box::new(registry_factory)).await;
    }

    async fn execute_with(registry_factory: crate::service::RegistryFactory) {
        let mut args: Vec<String> = std::env::args().collect();

        let opts = ParserOpts::new(&mut args);

        if opts.parsed.version() {
            println!("version: {}", env!("VERSION"));
        } else if opts.parsed.help() {
            println!("{}", opts.parsed.info());
        } else {
            let processor = crate::service::Processor::with_registry_factory(
                EphemeralKeyHandle::new(opts.ephemeral_file()),
                registry_factory,
            );

            println!(
                "---- Starting Parser server (version: {}) -----",
                env!("VERSION")
            );
            let mut tasks = Vec::new();
            tasks.push(tokio::spawn(async move {
                crate::host::Host::listen(opts.host_addr(), processor)
                    .await
                    .expect("`AsyncHost::listen` error");
            }));

            match tokio::signal::ctrl_c().await {
                Ok(()) => eprintln!("handling ctrl+c the tokio way"),

                Err(err) => panic!("{err}"),
            }
        }
    }
}
