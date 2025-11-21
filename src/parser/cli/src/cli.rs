use crate::chains;
use chains::parse_chain;
use clap::Parser;
use parser_app::registry::create_registry;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayload, SignablePayloadField};
use visualsign_ethereum::embedded_abis::load_and_map_abi;
use visualsign_ethereum::abi_registry::AbiRegistry;
use std::sync::Arc;

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
        long = "abi-json-mappings",
        value_name = "ABI_NAME:0xADDRESS",
        help = "Map custom ABI JSON file to contract address. Format: AbiName:/path/to/abi.json:0xAddress. Can be used multiple times"
    )]
    abi_json_mappings: Vec<String>,
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

/// Parses full ABI mapping with file path: "AbiName:/path/to/file.json:0xAddress"
fn parse_abi_file_mapping(mapping_str: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = mapping_str.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return None;
    }

    let address_str = parts[0];
    let rest = parts[1];

    let name_file_parts: Vec<&str> = rest.splitn(2, ':').collect();
    if name_file_parts.len() != 2 {
        return None;
    }

    let abi_name = name_file_parts[0].to_string();
    let file_path = name_file_parts[1].to_string();
    let address_str = address_str.to_string();

    Some((abi_name, file_path, address_str))
}

/// Builds an ABI registry from CLI mappings with file paths
/// Returns (registry, valid_count) and logs any errors
fn build_abi_registry_from_mappings(abi_json_mappings: &[String]) -> (AbiRegistry, usize) {
    let mut registry = AbiRegistry::new();
    let mut valid_count = 0;

    for mapping in abi_json_mappings {
        match parse_abi_file_mapping(mapping) {
            Some((abi_name, file_path, address_str)) => {
                let chain_id = 1u64; // TODO: Make chain_id configurable
                match load_and_map_abi(&mut registry, &abi_name, &file_path, chain_id, &address_str) {
                    Ok(()) => {
                        valid_count += 1;
                        eprintln!("  Loaded ABI '{}' from {} and mapped to {}", abi_name, file_path, address_str);
                    }
                    Err(e) => {
                        eprintln!("  Warning: Failed to load/map ABI '{}': {}", abi_name, e);
                    }
                }
            }
            None => {
                eprintln!(
                    "  Warning: Invalid ABI mapping '{}' (expected format: AbiName:/path/to/file.json:0xAddress)",
                    mapping
                );
            }
        }
    }

    (registry, valid_count)
}

fn parse_and_display(
    chain: &str,
    raw_tx: &str,
    mut options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
    abi_json_mappings: &[String],
) {
    let registry_chain = parse_chain(chain);

    // Build and report ABI registry from mappings
    if !abi_json_mappings.is_empty() {
        eprintln!("Registering custom ABIs:");
        let (registry, valid_count) = build_abi_registry_from_mappings(abi_json_mappings);
        eprintln!("Successfully registered {}/{} ABI mappings\n", valid_count, abi_json_mappings.len());
        options.abi_registry = Some(Arc::new(registry));
    }

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

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            abi_registry: None,
        };

        parse_and_display(
            &args.chain,
            &args.transaction,
            options,
            args.output,
            args.condensed_only,
            &args.abi_json_mappings,
        );
    }
}
