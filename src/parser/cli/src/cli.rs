use crate::chains::parse_chain;
use clap::Parser;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

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
    pub(crate) network: Option<String>,

    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    pub(crate) ethereum: crate::ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub(crate) solana: crate::solana::SolanaArgs,
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
    registry: &TransactionConverterRegistry,
    options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
) -> Result<(), String> {
    let registry_chain = parse_chain(chain);
    let payload = registry
        .convert_transaction(&registry_chain, raw_tx, options)
        .map_err(|err| err.to_string())?;
    match output_format {
        OutputFormat::Json => {
            let json_output = serde_json::to_string_pretty(&payload)
                .map_err(|err| format!("Failed to serialize output as JSON: {err}"))?;
            println!("{json_output}");
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
    }
    Ok(())
}

/// app cli
pub struct Cli;
impl Cli {
    /// start the parser cli
    pub fn execute() -> Result<(), String> {
        let args = Args::parse();
        let chain = parse_chain(&args.chain);
        let plugins = crate::build_plugins(&args);

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

        parse_and_display(
            &args.chain,
            &args.transaction,
            &registry,
            options,
            args.output,
            args.condensed_only,
        )
    }
}
