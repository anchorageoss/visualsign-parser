use crate::chains::parse_chain;
use clap::{Parser, Subcommand};
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = "1.0")]
#[command(about = "Converts raw transactions to visual signing properties")]
pub(crate) struct Args {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Decode a single transaction and print it.
    Decode(DecodeArgs),
    /// Serve a directory of raw-transaction files via a local web UI.
    #[cfg(feature = "serve")]
    Serve(crate::serve::ServeArgs),
}

#[derive(clap::Args, Debug)]
pub(crate) struct DecodeArgs {
    #[arg(short, long, help = "Chain type")]
    pub(crate) chain: String,

    #[arg(
        short,
        long,
        value_name = "RAW_TX",
        help = "Raw transaction string. Prefix with '@' to read from a file \
                (e.g. '@/path/to/tx.hex'), or use '@-' to read from stdin."
    )]
    pub(crate) transaction: String,

    #[arg(short, long, default_value = "text", help = "Output format")]
    pub(crate) output: OutputFormat,

    #[arg(
        long,
        help = "Show only condensed view (what hardware wallets display)"
    )]
    pub(crate) condensed_only: bool,

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
    pub(crate) ethereum: crate::ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub(crate) solana: crate::solana::SolanaArgs,
}

impl DecodeArgs {
    pub(crate) fn plugin_args(&self) -> crate::PluginArgs {
        crate::PluginArgs {
            #[cfg(feature = "ethereum")]
            ethereum: self.ethereum.clone(),
            #[cfg(feature = "solana")]
            solana: self.solana.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OutputFormat {
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
    registry: &TransactionConverterRegistry,
    options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
) {
    let registry_chain = parse_chain(chain);
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

/// Resolves chain + plugins + per-chain metadata, returning a ready-to-use registry.
///
/// Shared by `decode` and `serve`. On invalid chain or metadata error, returns
/// an `Err(String)` with a user-facing message — the caller is responsible for
/// printing it and exiting non-zero.
pub(crate) struct Runtime {
    pub registry: TransactionConverterRegistry,
    pub options: VisualSignOptions,
}

pub(crate) fn prepare_runtime(
    chain_str: &str,
    network: Option<String>,
    plugin_args: &crate::PluginArgs,
) -> Result<Runtime, String> {
    let chain = parse_chain(chain_str);
    let plugins = crate::build_plugins(plugin_args);

    let mut registry = TransactionConverterRegistry::new();
    for plugin in &plugins {
        plugin.register(&mut registry);
    }

    let plugin = plugins.iter().find(|p| p.chain() == chain);

    let Some(plugin) = plugin else {
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
            return Err(format!(
                "unrecognized chain '{chain_str}'.\nSupported chains: {supported_str}"
            ));
        }
        return Err(format!(
            "chain '{chain_str}' is not supported by this CLI build.\nSupported chains: {supported_str}"
        ));
    };

    let chain_metadata = plugin.create_metadata(network)?;

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: chain_metadata,
        developer_config: Some(DeveloperConfig {
            allow_signed_transactions: true,
        }),
    };

    Ok(Runtime { registry, options })
}

fn execute_decode(args: &DecodeArgs) {
    let plugin_args = args.plugin_args();
    let runtime = match prepare_runtime(&args.chain, args.network.clone(), &plugin_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let raw_tx = match crate::tx_input::resolve_transaction_input(&args.transaction) {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    parse_and_display(
        &args.chain,
        &raw_tx,
        &runtime.registry,
        runtime.options,
        args.output,
        args.condensed_only,
    );
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
        match &args.command {
            Command::Decode(a) => execute_decode(a),
            #[cfg(feature = "serve")]
            Command::Serve(a) => crate::serve::execute_serve(a),
        }
    }
}
