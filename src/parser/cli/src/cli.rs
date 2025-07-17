use crate::chains;
use chains::{available_chains, parse_chain};
use clap::{Arg, Command};
use parser_app::registry::create_registry;
use visualsign::vsptrait::VisualSignOptions;

fn parse_and_display(chain: &str, raw_tx: &str, options: VisualSignOptions) {
    let registry_chain = parse_chain(chain);

    let registry = create_registry();
    let signable_payload_str = registry.convert_transaction(&registry_chain, raw_tx, options);

    println!("Visual Signing Properties:");
    println!("========================");
    println!("Chain: {chain}");
    println!("Transaction: {raw_tx}");
    println!("Properties: {signable_payload_str:#?}");
}

/// app cli
pub struct Cli;
impl Cli {
    /// start the parser cli
    ///
    /// # Panics
    ///
    /// Executes the CLI application, parsing command line arguments and processing the transaction
    pub fn execute() {
        let chains = available_chains();
        let chain_help = format!("Chain type ({})", chains.join(", "));

        let matches = Command::new("visualsign-parser")
            .version("1.0")
            .about("Converts raw transactions to visual signing properties")
            .arg(
                Arg::new("chain")
                    .short('c')
                    .long("chain")
                    .value_name("CHAIN")
                    .help(&chain_help)
                    .value_parser(chains.clone())
                    .required(true),
            )
            .arg(
                Arg::new("transaction")
                    .short('t')
                    .long("transaction")
                    .value_name("RAW_TX")
                    .help("Raw transaction hex string")
                    .required(true),
            )
            .get_matches();

        let chain = matches
            .get_one::<String>("chain")
            .expect("Chain is required");
        let raw_tx = matches
            .get_one::<String>("transaction")
            .expect("Transaction is required");

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        parse_and_display(chain, raw_tx, options);
    }
}
