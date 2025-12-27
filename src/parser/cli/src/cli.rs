use crate::chains;
use chains::parse_chain;
use clap::Parser;
use generated::parser::{
    Abi, ChainMetadata, EthereumMetadata, SolanaMetadata, chain_metadata::Metadata,
};
use parser_app::registry::create_registry;
use visualsign::registry::Chain;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayload, SignablePayloadField};
use visualsign_ethereum::networks::parse_network;

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = "1.0")]
#[command(about = "Converts raw transactions to visual signing properties")]
struct Args {
    #[arg(short, long, help = "Chain type")]
    chain: String,

    #[arg(
        short,
        long,
        value_name = "RAW_TX",
        help = "Raw transaction hex string"
    )]
    transaction: String,

    #[arg(short, long, default_value = "text", help = "Output format")]
    output: OutputFormat,

    #[arg(
        long,
        help = "Show only condensed view (what hardware wallets display)"
    )]
    condensed_only: bool,

    #[arg(
        long,
        short = 'n',
        value_name = "NETWORK",
        help = "Network identifier - chain ID (e.g., 1, 137) or name (e.g., ETHEREUM_MAINNET, POLYGON_MAINNET)"
    )]
    network: Option<String>,

    #[arg(
        long,
        short = 'a',
        value_name = "ABI_JSON",
        help = "ABI JSON string for Ethereum contract decoding"
    )]
    abi: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Text,
    Json,
    Human,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "human" => Ok(OutputFormat::Human),
            _ => Err(format!("Invalid output format: {s}")),
        }
    }
}

struct HumanReadableFormatter<'a> {
    payload: &'a SignablePayload,
    condensed_only: bool,
}

impl<'a> HumanReadableFormatter<'a> {
    fn new(payload: &'a SignablePayload, condensed_only: bool) -> Self {
        Self {
            payload,
            condensed_only,
        }
    }

    fn format_field(
        &self,
        field: &SignablePayloadField,
        writer: &mut dyn std::fmt::Write,
        prefix: &str,
        continuation: &str,
    ) -> std::fmt::Result {
        match field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                writeln!(writer, "{} {}: {}", prefix, common.label, text_v2.text)?;
            }
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                writeln!(writer, "{} {}", prefix, common.label)?;

                if let Some(title) = &preview_layout.title {
                    writeln!(writer, "{}   Title: {}", continuation, title.text)?;
                }
                if let Some(subtitle) = &preview_layout.subtitle {
                    writeln!(writer, "{}   Detail: {}", continuation, subtitle.text)?;
                }

                // Condensed view (if present)
                if let Some(condensed_layout) = &preview_layout.condensed {
                    if !condensed_layout.fields.is_empty() {
                        writeln!(writer, "{continuation}   📋 Condensed View:")?;
                        for (i, nested_field) in condensed_layout.fields.iter().enumerate() {
                            let is_last_nested = i == condensed_layout.fields.len() - 1;
                            let nested_prefix = format!(
                                "{}   {}",
                                continuation,
                                if is_last_nested { "└─" } else { "├─" }
                            );
                            let nested_continuation = format!(
                                "{}   {}",
                                continuation,
                                if is_last_nested { "   " } else { "│  " }
                            );
                            self.format_field(
                                &nested_field.signable_payload_field,
                                writer,
                                &nested_prefix,
                                &nested_continuation,
                            )?;
                        }
                    }
                }

                // Expanded view (if present, only show if not condensed_only)
                if !self.condensed_only {
                    if let Some(expanded_layout) = &preview_layout.expanded {
                        if !expanded_layout.fields.is_empty() {
                            writeln!(writer, "{continuation}   📖 Expanded View:")?;
                            for (i, nested_field) in expanded_layout.fields.iter().enumerate() {
                                let is_last_nested = i == expanded_layout.fields.len() - 1;
                                let nested_prefix = format!(
                                    "{}   {}",
                                    continuation,
                                    if is_last_nested { "└─" } else { "├─" }
                                );
                                let nested_continuation = format!(
                                    "{}   {}",
                                    continuation,
                                    if is_last_nested { "   " } else { "│  " }
                                );
                                self.format_field(
                                    &nested_field.signable_payload_field,
                                    writer,
                                    &nested_prefix,
                                    &nested_continuation,
                                )?;
                            }
                        }
                    }
                }
            }
            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                writeln!(
                    writer,
                    "{} {}: {} {}",
                    prefix,
                    common.label,
                    amount_v2.amount,
                    amount_v2.abbreviation.as_deref().unwrap_or("")
                )?;
            }
            SignablePayloadField::AddressV2 { common, address_v2 } => {
                writeln!(
                    writer,
                    "{} {}: {}",
                    prefix, common.label, address_v2.address
                )?;
            }
            _ => {
                writeln!(writer, "{} Field: {}", prefix, common_label(field))?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for HumanReadableFormatter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "┌─ Transaction: {}", self.payload.title)?;
        if let Some(subtitle) = &self.payload.subtitle {
            writeln!(f, "│  Subtitle: {subtitle}")?;
        }
        writeln!(f, "│  Version: {}", self.payload.version)?;
        if !self.payload.payload_type.is_empty() {
            writeln!(f, "│  Type: {}", self.payload.payload_type)?;
        }
        f.write_str("│\n")?;

        if !self.payload.fields.is_empty() {
            f.write_str("└─ Fields:\n")?;
            for (i, field) in self.payload.fields.iter().enumerate() {
                let is_last = i == self.payload.fields.len() - 1;
                let prefix = if is_last { "   └─" } else { "   ├─" };
                let continuation = if is_last { "      " } else { "   │  " };

                self.format_field(field, f, prefix, continuation)?;
            }
        }

        Ok(())
    }
}

/// Helper to extract common label from any field type
fn common_label(field: &SignablePayloadField) -> String {
    match field {
        SignablePayloadField::TextV2 { common, .. }
        | SignablePayloadField::PreviewLayout { common, .. }
        | SignablePayloadField::AmountV2 { common, .. }
        | SignablePayloadField::AddressV2 { common, .. } => common.label.clone(),
        _ => "Unknown".to_string(),
    }
}

fn parse_and_display(
    chain: &str,
    raw_tx: &str,
    options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
) {
    let registry_chain = parse_chain(chain);

    let registry = create_registry();
    let signable_payload_str = registry.convert_transaction(&registry_chain, raw_tx, options);
    match signable_payload_str {
        Ok(payload) => match output_format {
            OutputFormat::Json => {
                if let Ok(json_output) = serde_json::to_string_pretty(&payload) {
                    println!("{json_output}");
                } else {
                    eprintln!("Error: Failed to serialize output as JSON");
                }
            }
            OutputFormat::Text => {
                println!("{payload:#?}");
            }
            OutputFormat::Human => {
                let formatter = HumanReadableFormatter::new(&payload, condensed_only);
                println!("{formatter}");
                if !condensed_only {
                    eprintln!(
                        "\nRun with `--condensed-only` to see what users see on hardware wallets"
                    );
                }
            }
        },
        Err(err) => {
            eprintln!("Error: {err:?}");
        }
    }
}

/// Creates chain-specific metadata from the `network` and `abi` arguments
///
/// The network can be specified as either:
/// - A chain ID number (e.g., 1, 137, 42161)
/// - A canonical network name (e.g., `ETHEREUM_MAINNET`, `POLYGON_MAINNET`)
///
/// The ABI can be specified as a JSON string for Ethereum contract decoding.
///
/// Returns `None` if no network or ABI is specified.
/// Prints an error and exits if the network identifier is invalid.
fn create_chain_metadata(
    chain: &Chain,
    network: Option<String>,
    abi: Option<String>,
) -> Option<ChainMetadata> {
    // Return None if neither network nor abi is provided
    if network.is_none() && abi.is_none() {
        return None;
    }

    // Parse and validate the network identifier if provided
    let network_id = if let Some(network) = network {
        let Some(network_id) = parse_network(&network) else {
            eprintln!(
                "Error: Invalid network '{network}'. Use a chain ID (e.g., 1, 137) or name (e.g., ETHEREUM_MAINNET, POLYGON_MAINNET)"
            );
            std::process::exit(1);
        };
        Some(network_id)
    } else {
        None
    };

    // Create ABI struct if provided
    let abi_struct = abi.map(|abi_json| Abi {
        value: abi_json,
        signature: None,
    });

    let metadata = match chain {
        Chain::Solana => Metadata::Solana(SolanaMetadata {
            network_id,
            idl: None,
        }),
        // For Ethereum and other chains, use EthereumMetadata structure
        _ => Metadata::Ethereum(EthereumMetadata {
            network_id,
            abi: abi_struct,
        }),
    };

    Some(ChainMetadata {
        metadata: Some(metadata),
    })
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
        let args = Args::parse();

        let chain = parse_chain(&args.chain);
        let metadata = create_chain_metadata(&chain, args.network, args.abi);

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata,
        };

        parse_and_display(
            &args.chain,
            &args.transaction,
            options,
            args.output,
            args.condensed_only,
        );
    }
}
