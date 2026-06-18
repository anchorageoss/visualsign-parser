// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod logger;
#[cfg(feature = "serve")]
mod serve;

use clap::{Args as ClapArgs, Parser, Subcommand};
use parser_cli_core::{ChainPlugin, SharedArgs};

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = env!("VERSION"))]
#[command(about = "Converts raw transactions to visual signing properties")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Decode a single transaction and print it.
    Decode(DecodeArgs),
    /// Serve a directory of raw-transaction files via a local web UI.
    #[cfg(feature = "serve")]
    Serve(serve::ServeArgs),
}

/// Per-chain CLI args, flattened into every subcommand that builds plugins.
///
/// **To add a new chain:** add one `#[command(flatten)]` field here (behind its
/// feature flag) and one `plugins.push(...)` in [`ChainArgs::build_plugins`].
#[derive(ClapArgs, Debug, Default, Clone)]
pub(crate) struct ChainArgs {
    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    pub(crate) ethereum: visualsign_ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub(crate) solana: visualsign_solana::SolanaArgs,

    #[cfg(feature = "tron")]
    #[command(flatten)]
    pub(crate) tron: visualsign_tron::TronArgs,
}

impl ChainArgs {
    /// Construct all enabled chain plugins, each pre-loaded with its CLI args.
    #[allow(clippy::vec_init_then_push)] // cfg-gated pushes cannot be expressed as vec![...]
    pub(crate) fn build_plugins(&self) -> Vec<Box<dyn ChainPlugin>> {
        let mut plugins: Vec<Box<dyn ChainPlugin>> = vec![];
        #[cfg(feature = "ethereum")]
        plugins.push(Box::new(visualsign_ethereum::EthereumPlugin::new(
            self.ethereum.clone(),
        )));
        #[cfg(feature = "solana")]
        plugins.push(Box::new(visualsign_solana::SolanaPlugin::new(
            self.solana.clone(),
        )));
        #[cfg(feature = "tron")]
        plugins.push(Box::new(visualsign_tron::TronPlugin::new(
            self.tron.clone(),
        )));
        plugins
    }
}

/// Args for the `decode` subcommand: the shared decode flags plus the per-chain args.
#[derive(ClapArgs, Debug)]
struct DecodeArgs {
    #[command(flatten)]
    shared: SharedArgs,
    #[command(flatten)]
    chains: ChainArgs,
}

fn main() {
    logger::setup_logger();

    let args = Args::parse();

    let result = match &args.command {
        Command::Decode(decode) => {
            let plugins = decode.chains.build_plugins();
            parser_cli_core::run(&decode.shared, &plugins)
        }
        #[cfg(feature = "serve")]
        Command::Serve(serve_args) => serve::run(serve_args),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
