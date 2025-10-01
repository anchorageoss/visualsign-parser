use crate::chains;
use chains::{available_chains, parse_chain};
use clap::{Arg, Command};
use parser_app::registry::create_registry;
use visualsign::vsptrait::VisualSignOptions;

fn parse_and_display(
    chain: &str,
    raw_tx: &str,
    options: VisualSignOptions,
    output_format: &str,
    allow_partial: bool,
    debug_mode: bool,
) {
    let registry_chain = parse_chain(chain);

    // Debug output - show raw transaction details
    if debug_mode {
        println!("=== DEBUG MODE ===");
        println!("Raw transaction: {}", raw_tx);
        println!("Chain: {}", chain);
        println!("Allow partial: {}", allow_partial);
        println!("Output format: {}", output_format);

        // Hex analysis
        if let Ok(bytes) = hex::decode(raw_tx.strip_prefix("0x").unwrap_or(raw_tx)) {
            println!("Hex bytes length: {}", bytes.len());
            println!(
                "Raw bytes: [{}]",
                bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            if !bytes.is_empty() {
                println!("First byte: 0x{:02x} ({})", bytes[0], bytes[0]);
                // Known transaction type tags
                match bytes[0] {
                    0x00 => println!("  -> EIP-2930 transaction"),
                    0x01 => println!("  -> EIP-2930 transaction"),
                    0x02 => println!("  -> EIP-1559 transaction"),
                    0x03 => println!("  -> EIP-4844 blob transaction"),
                    b if b <= 0x7f => println!("  -> EIP-2718 typed transaction (type {})", b),
                    b if b >= 0xc0 => println!("  -> RLP list"),
                    _ => println!("  -> Unknown format"),
                }
            }
        } else {
            println!("Failed to decode hex");
        }
        println!("==================");
        println!();
    }

    let registry = create_registry();
    let signable_payload_str = registry.convert_transaction(&registry_chain, raw_tx, options);
    match signable_payload_str {
        Ok(payload) => {
            if debug_mode {
                println!("=== PARSED PAYLOAD DEBUG ===");
                println!("Title: {}", payload.title);
                println!("Subtitle: {:?}", payload.subtitle);
                println!("Version: {}", payload.version);
                println!("Payload Type: {}", payload.payload_type);
                println!("Number of fields: {}", payload.fields.len());
                for (i, field) in payload.fields.iter().enumerate() {
                    println!(
                        "Field {}: {} = {}",
                        i + 1,
                        field.label(),
                        field.fallback_text()
                    );
                }
                println!("============================");
                println!();
            }

            match output_format {
                "json" => {
                    if let Ok(json_output) = serde_json::to_string_pretty(&payload) {
                        println!("{json_output}");
                    } else {
                        eprintln!("Error: Failed to serialize output as JSON");
                    }
                }
                "text" => {
                    println!("{payload:#?}");
                }
                _ => {
                    eprintln!("Error: Unsupported output format '{output_format}'");
                }
            }
        }
        Err(err) => {
            if debug_mode {
                println!("=== PARSING ERROR DEBUG ===");
                println!("Error type: {:?}", err);
                println!("===========================");
            }
            eprintln!("Error: {err:?}");
        }
    }
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
            .arg(
                Arg::new("output")
                    .short('o')
                    .long("output")
                    .value_name("FORMAT")
                    .help("Output format")
                    .value_parser(["text", "json"])
                    .default_value("text"),
            )
            .arg(
                Arg::new("partial")
                    .short('p')
                    .long("partial")
                    .help("Allow parsing of partial/incomplete transactions")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("debug")
                    .short('d')
                    .long("debug")
                    .help("Show low-level debug information including raw hex analysis")
                    .action(clap::ArgAction::SetTrue),
            )
            .get_matches();

        let chain = matches
            .get_one::<String>("chain")
            .expect("Chain is required");
        let raw_tx = matches
            .get_one::<String>("transaction")
            .expect("Transaction is required");
        let output_format = matches
            .get_one::<String>("output")
            .expect("Output format has default value");
        let allow_partial = matches.get_flag("partial");
        let debug_mode = matches.get_flag("debug");

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: allow_partial,
        };

        parse_and_display(
            chain,
            raw_tx,
            options,
            output_format,
            allow_partial,
            debug_mode,
        );
    }
}
