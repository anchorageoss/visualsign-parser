use crate::chains;
use crate::mapping_parser;
use chains::parse_chain;
use clap::Parser;
use generated::parser::{
    Abi, ChainMetadata, EthereumMetadata, SolanaIdlType, SolanaMetadata, chain_metadata::Metadata,
};
use parser_app::registry::create_registry;
use std::collections::HashMap;
use visualsign::registry::Chain;
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
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
        help = "Network identifier - supports:\n\
                Chain ID: 1, 137, 42161, etc.\n\
                Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET, etc."
    )]
    network: Option<String>,

    #[arg(
        long = "abi-json-mappings",
        value_name = "ABI_NAME:FILE_PATH:0xADDRESS",
        help = "Map custom ABI JSON file to contract address. Format: AbiName:/path/to/abi.json:0xAddress. Can be used multiple times"
    )]
    abi_json_mappings: Vec<String>,

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

/// Parses full ABI mapping with file path: `<Name:/path/to/abi.json:ContractAddress>`
///
/// Returns: (`abi_name`, `file_path`, `contract_address`)
fn parse_abi_file_mapping(mapping_str: &str) -> Option<(String, String, String)> {
    match mapping_parser::parse_mapping(mapping_str) {
        Ok(components) => Some((components.name, components.path, components.identifier)),
        Err(e) => {
            eprintln!("Error parsing ABI mapping: {e}");
            eprintln!("Expected format: Name:/path/to/abi.json:ContractAddress");
            eprintln!(
                "Example: UniswapV2:/home/user/uniswap.json:0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"
            );
            None
        }
    }
}

/// Load ABI JSON files and create `HashMap` for `EthereumMetadata.abi_mappings`
fn build_abi_mappings_from_files(abi_json_mappings: &[String]) -> (HashMap<String, Abi>, usize) {
    let mut mappings = HashMap::new();
    let mut valid_count = 0;

    for mapping in abi_json_mappings {
        match parse_abi_file_mapping(mapping) {
            Some((abi_name, file_path, address_str)) => match load_json_from_file(&file_path) {
                Ok(abi_json) => {
                    let abi = Abi {
                        value: abi_json,
                        signature: None,
                    };
                    mappings.insert(address_str.clone(), abi);
                    valid_count += 1;
                    eprintln!(
                        "  Loaded ABI '{abi_name}' from {file_path} and mapped to {address_str}"
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: Failed to load ABI '{abi_name}' from '{file_path}': {e}");
                }
            },
            None => {
                eprintln!(
                    "  Warning: Invalid ABI mapping '{mapping}' (expected format: AbiName:/path/to/file.json:0xAddress)",
                );
            }
        }
    }

    (mappings, valid_count)
}

/// Load a JSON file from disk and validate it parses as valid JSON
fn load_json_from_file(file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let json_str = std::fs::read_to_string(file_path)?;

    // Validate it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&json_str)?;

    Ok(json_str)
}

/// Parse IDL mapping format: `<Name:/path/to/file.json:ProgramId>`
///
/// Splits from the right to handle file paths containing colons (e.g., Windows paths
/// like `C:/path/to/file.json`). The last colon separates the program ID, the middle
/// section is the file path, and the first part is the name.
///
/// Returns: (`idl_name`, `program_id_str`, `file_path`)
fn parse_idl_file_mapping(mapping_str: &str) -> Option<(String, String, String)> {
    match mapping_parser::parse_mapping(mapping_str) {
        Ok(components) => Some((components.name, components.identifier, components.path)),
        Err(e) => {
            eprintln!("Error parsing IDL mapping: {e}");
            eprintln!("Expected format: Name:/path/to/idl.json:ProgramId");
            eprintln!(
                "Example: JupiterSwap:/home/user/jupiter.json:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
            );
            None
        }
    }
}

/// Load IDL JSON files and create `HashMap` for `SolanaMetadata`
fn build_idl_mappings_from_files(
    idl_json_mappings: &[String],
) -> (HashMap<String, generated::parser::Idl>, usize) {
    let mut mappings = HashMap::new();
    let mut valid_count = 0;

    for mapping in idl_json_mappings {
        match parse_idl_file_mapping(mapping) {
            Some((idl_name, program_id, file_path)) => match load_json_from_file(&file_path) {
                Ok(idl_json) => {
                    let idl = generated::parser::Idl {
                        value: idl_json,
                        idl_type: Some(SolanaIdlType::Anchor as i32),
                        idl_version: None,
                        signature: None,
                        program_name: Some(idl_name.clone()),
                    };
                    mappings.insert(program_id.clone(), idl);
                    valid_count += 1;
                    eprintln!(
                        "  Loaded IDL '{idl_name}' from {file_path} and mapped to {program_id}"
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: Failed to load IDL '{idl_name}' from '{file_path}': {e}");
                }
            },
            None => {
                eprintln!(
                    "  Warning: Invalid IDL mapping '{mapping}' (expected format: Name:ProgramId:/path/to/file.json)"
                );
            }
        }
    }

    (mappings, valid_count)
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

/// Creates chain-specific metadata from the `network` argument
///
/// The network can be specified as either:
/// - A chain ID number (e.g., 1, 137, 42161)
/// - A canonical network name (e.g., `ETHEREUM_MAINNET`, `POLYGON_MAINNET`)
///
/// Defaults to `ETHEREUM_MAINNET` if no network is specified for Ethereum chains.
/// Returns `None` if no network is specified and no IDL mappings are provided for Solana.
/// Prints an error and exits if the network identifier is invalid.
fn create_chain_metadata(
    chain: &Chain,
    network: Option<String>,
    abi_json_mappings: &[String],
    idl_json_mappings: &[String],
) -> Option<ChainMetadata> {
    // Parse network if provided, with Ethereum defaulting logic
    let network_id = if let Some(network) = network {
        let Some(network_id) = parse_network(&network) else {
            eprintln!(
                "Error: Invalid network '{network}'. Supported formats:\n\
                 - Chain ID (numeric): 1 (Ethereum), 137 (Polygon), 42161 (Arbitrum)\n\
                 - Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET\n\
                 \n\
                 Run with --help for full list of supported networks."
            );
            std::process::exit(1);
        };
        Some(network_id)
    } else if chain == &Chain::Ethereum {
        // Default to Ethereum Mainnet for Ethereum chains if no network specified
        eprintln!("Warning: No network specified, defaulting to ETHEREUM_MAINNET (chain_id: 1)");
        Some(parse_network("ETHEREUM_MAINNET").expect("ETHEREUM_MAINNET should always be valid"))
    } else {
        None
    };

    let metadata = if chain == &Chain::Solana {
        // Build IDL mappings if provided
        let idl_mappings = if idl_json_mappings.is_empty() {
            HashMap::new()
        } else {
            eprintln!("Loading custom IDLs:");
            let (mappings, valid_count) = build_idl_mappings_from_files(idl_json_mappings);
            eprintln!(
                "Successfully loaded {}/{} IDL mappings\n",
                valid_count,
                idl_json_mappings.len()
            );
            mappings
        };

        // Only create metadata if we have network or IDL mappings
        if network_id.is_none() && idl_mappings.is_empty() {
            return None;
        }

        Metadata::Solana(SolanaMetadata {
            network_id,
            idl: None,
            idl_mappings,
        })
    } else {
        // For Ethereum and other chains, use EthereumMetadata structure
        // Ethereum requires network_id
        let network_id = network_id?;

        // Build ABI mappings if provided
        let abi_mappings = if abi_json_mappings.is_empty() || chain != &Chain::Ethereum {
            if !abi_json_mappings.is_empty() && chain != &Chain::Ethereum {
                eprintln!("Warning: --abi-json-mappings is only supported for Ethereum; ignoring");
            }
            HashMap::new()
        } else {
            eprintln!("Loading custom ABIs:");
            let (mappings, valid_count) = build_abi_mappings_from_files(abi_json_mappings);
            eprintln!(
                "Successfully loaded {}/{} ABI mappings\n",
                valid_count,
                abi_json_mappings.len()
            );
            mappings
        };

        Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
            abi_mappings,
        })
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
        let chain_metadata = create_chain_metadata(
            &chain,
            args.network,
            &args.abi_json_mappings,
            &args.idl_json_mappings,
        );

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
            options,
            args.output,
            args.condensed_only,
        );
    }
}
