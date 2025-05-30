//! CLI for the parser app
use qos_core::{
    QUORUM_FILE, SEC_APP_SOCK,
    cli::{EPHEMERAL_FILE_OPT, USOCK},
    handles::EphemeralKeyHandle,
    io::SocketAddress,
    parser::{GetParserForOptions, OptionsParser, Parser, Token},
    server::SocketServer,
};

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

    fn addr(&self) -> SocketAddress {
        SocketAddress::new_unix(self.parsed.single(USOCK).expect("Unix socket is required"))
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
                Token::new(USOCK, "unix socket (`.sock`) to listen on.")
                    .takes_value(true)
                    .forbids(vec!["port", "cid"])
                    .default_value(SEC_APP_SOCK),
            )
            .token(
                Token::new(
                    EPHEMERAL_FILE_OPT,
                    "path to file where the Ephemeral Key secret should be retrieved from. Use default for production.",
                )
                .takes_value(true)
                .default_value(QUORUM_FILE),
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
    pub fn execute() {
        let mut args: Vec<String> = std::env::args().collect();

        let opts = ParserOpts::new(&mut args);

        if opts.parsed.version() {
            println!("version: {}", env!("CARGO_PKG_VERSION"));
        } else if opts.parsed.help() {
            println!("{}", opts.parsed.info());
        } else {
            let processor =
                crate::service::Processor::new(EphemeralKeyHandle::new(opts.ephemeral_file()));

            println!("---- Starting Parser server -----");
            SocketServer::listen(opts.addr(), processor).expect("unable to start Parser server");
        }
    }
}
