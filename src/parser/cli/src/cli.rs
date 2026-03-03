use crate::chains::parse_chain;
use clap::Parser;
use generated::parser::ChainMetadata;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

#[cfg(feature = "ethereum")]
use crate::ethereum;
#[cfg(feature = "solana")]
use crate::solana;

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
        help = "Network identifier - supports:\n\
                Chain ID: 1, 137, 42161, etc.\n\
                Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET, etc."
    )]
    network: Option<String>,

    #[cfg(feature = "ethereum")]
    #[arg(
        long = "abi-json-mappings",
        value_name = "ABI_NAME:FILE_PATH:0xADDRESS",
        help = "Map custom ABI JSON file to contract address. Format: AbiName:/path/to/abi.json:0xAddress. Can be used multiple times"
    )]
    abi_json_mappings: Vec<String>,

    #[cfg(feature = "solana")]
    #[arg(
        long = "idl-json-mappings",
        value_name = "IDL_NAME:FILE_PATH:PROGRAM_ID",
        help = "Map custom IDL JSON file to Solana program. Format: IdlName:/path/to/idl.json:base58_program_id. Can be used multiple times"
    )]
    idl_json_mappings: Vec<String>,
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
    let registry = Cli::build_registry();
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

fn create_chain_metadata(
    chain: &Chain,
    _network: Option<String>,
    idl_json_mappings: &[String],
) -> Option<ChainMetadata> {
    match chain {
        #[cfg(feature = "solana")]
        Chain::Solana => solana::create_chain_metadata(idl_json_mappings),
        #[cfg(feature = "ethereum")]
        Chain::Ethereum => ethereum::create_chain_metadata(_network),
        _ => None,
    }
}

/// app cli
pub struct Cli;
impl Cli {
    fn build_registry() -> TransactionConverterRegistry {
        let mut registry = TransactionConverterRegistry::new();

        #[cfg(feature = "ethereum")]
        registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
            Chain::Ethereum,
            visualsign_ethereum::EthereumVisualSignConverter::new(),
        );

        #[cfg(feature = "solana")]
        registry.register::<visualsign_solana::SolanaTransactionWrapper, _>(
            Chain::Solana,
            visualsign_solana::SolanaVisualSignConverter,
        );

        registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
            Chain::Unspecified,
            visualsign_unspecified::UnspecifiedVisualSignConverter,
        );

        registry
    }

    /// start the parser cli
    ///
    /// # Panics
    ///
    /// Executes the CLI application, parsing command line arguments and processing the transaction
    #[allow(unused_mut)] // mut only needed when ethereum feature is enabled
    pub fn execute() {
        let args = Args::parse();

        let chain = parse_chain(&args.chain);

        #[cfg(feature = "solana")]
        let idl_json_mappings = args.idl_json_mappings;
        #[cfg(not(feature = "solana"))]
        let idl_json_mappings: Vec<String> = vec![];

        let chain_metadata = create_chain_metadata(&chain, args.network, &idl_json_mappings);

        let mut options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: chain_metadata,
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
            abi_registry: None,
        };

        #[cfg(feature = "ethereum")]
        ethereum::apply_abi_registry(&mut options, &args.abi_json_mappings);

        parse_and_display(
            &args.chain,
            &args.transaction,
            options,
            args.output,
            args.condensed_only,
        );
    }
}
