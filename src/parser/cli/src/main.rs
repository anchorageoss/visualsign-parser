// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod logger;

use clap::Parser;
use parser_cli_core::{ChainPlugin, SharedArgs};

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = env!("VERSION"))]
#[command(about = "Converts raw transactions to visual signing properties")]
struct Args {
    #[command(flatten)]
    shared: SharedArgs,

    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    ethereum: visualsign_ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    solana: visualsign_solana::SolanaArgs,

    #[cfg(feature = "tron")]
    #[command(flatten)]
    tron: visualsign_tron::TronArgs,
}

fn main() {
    logger::setup_logger();

    let args = Args::parse();

    #[allow(unused_mut)]
    let mut plugins: Vec<Box<dyn ChainPlugin>> = vec![];
    #[cfg(feature = "ethereum")]
    plugins.push(Box::new(visualsign_ethereum::EthereumPlugin::new(
        args.ethereum.clone(),
    )));
    #[cfg(feature = "solana")]
    plugins.push(Box::new(visualsign_solana::SolanaPlugin::new(
        args.solana.clone(),
    )));
    #[cfg(feature = "tron")]
    plugins.push(Box::new(visualsign_tron::TronPlugin::new(
        args.tron.clone(),
    )));

    if let Err(e) = parser_cli_core::run(&args.shared, &plugins) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
