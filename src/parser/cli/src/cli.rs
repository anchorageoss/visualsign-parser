use clap::Parser;
use parser_cli_core::chains::parse_chain;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = env!("VERSION"))]
#[command(about = "Converts raw transactions to visual signing properties")]
pub(crate) struct Args {
    #[arg(short, long, help = "Chain type")]
    pub(crate) chain: String,

    #[arg(
        short,
        long,
        value_name = "RAW_TX",
        help = "Raw transaction string. Prefix with '@' to read from a file \
                (e.g. '@/path/to/tx.hex'), or use '@-' to read from stdin."
    )]
    transaction: String,

    #[arg(short, long, default_value = "text", help = "Output format")]
    output: parser_cli_core::OutputFormat,

    #[arg(
        long,
        help = "Show only condensed view (what hardware wallets display)"
    )]
    condensed_only: bool,

    #[arg(
        long,
        short = 'n',
        value_name = "NETWORK",
        help = "Network identifier - supports:\n\
                Chain ID: 1, 137, 42161, etc.\n\
                Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET, etc."
    )]
    pub(crate) network: Option<String>,

    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    pub(crate) ethereum: visualsign_ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub(crate) solana: visualsign_solana::SolanaArgs,

    #[cfg(feature = "tron")]
    #[command(flatten)]
    pub(crate) tron: crate::tron::TronArgs,
}

/// CLI entry point.
pub struct Cli;
impl Cli {
    /// Parse arguments and run the transaction visualizer.
    pub fn execute() -> Result<(), String> {
        let args = Args::parse();
        let chain = parse_chain(&args.chain);

        #[allow(clippy::vec_init_then_push)] // cfg-gated pushes cannot be expressed as vec![...]
        let plugins: Vec<Box<dyn parser_cli_core::ChainPlugin>> = {
            let mut plugins: Vec<Box<dyn parser_cli_core::ChainPlugin>> = vec![];
            #[cfg(feature = "ethereum")]
            plugins.push(Box::new(visualsign_ethereum::EthereumPlugin::new(
                args.ethereum.clone(),
            )));
            #[cfg(feature = "solana")]
            plugins.push(Box::new(visualsign_solana::SolanaPlugin::new(
                args.solana.clone(),
            )));
            #[cfg(feature = "tron")]
            plugins.push(Box::new(crate::tron::TronPlugin::new(args.tron.clone())));
            plugins
        };

        let mut registry = TransactionConverterRegistry::new();
        for plugin in &plugins {
            plugin.register(&mut registry);
        }

        let plugin = plugins.iter().find(|p| p.chain() == chain).ok_or_else(|| {
            let supported: Vec<String> = plugins
                .iter()
                .map(|p| p.chain().as_str().to_lowercase())
                .collect();
            let supported_str = if supported.is_empty() {
                "none".to_string()
            } else {
                supported.join(", ")
            };
            if chain == Chain::Unspecified {
                format!(
                    "unrecognized chain '{}'.\nSupported chains: {supported_str}",
                    args.chain,
                )
            } else {
                format!(
                    "chain '{}' is not supported by this CLI build.\n\
                     Supported chains: {supported_str}",
                    args.chain,
                )
            }
        })?;

        let chain_metadata = plugin.create_metadata(args.network.clone())?;

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: chain_metadata,
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
        };

        let raw_tx = match parser_cli_core::tx_input::resolve_transaction_input(&args.transaction) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        };

        parser_cli_core::output::parse_and_display(
            &args.chain,
            &raw_tx,
            &registry,
            options,
            args.output,
            args.condensed_only,
        )
    }
}
