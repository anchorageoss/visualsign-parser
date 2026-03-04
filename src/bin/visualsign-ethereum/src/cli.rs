use clap::Parser;
use generated::parser::{ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
use parser_cli::display::{OutputFormat, print_payload};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MappingComponents {
    name: String,
    path: String,
    identifier: String,
}

fn parse_mapping(mapping_str: &str) -> Result<MappingComponents, String> {
    let (name_and_path, identifier) = mapping_str.rsplit_once(':').ok_or_else(|| {
        format!("Invalid mapping format (expected name:path:identifier): {mapping_str}")
    })?;
    let (name, path) = name_and_path.split_once(':').ok_or_else(|| {
        format!("Invalid mapping format (expected name:path:identifier): {mapping_str}")
    })?;
    if name.is_empty() || path.is_empty() || identifier.is_empty() {
        return Err(format!("Mapping components cannot be empty: {mapping_str}"));
    }
    Ok(MappingComponents {
        name: name.to_string(),
        path: path.to_string(),
        identifier: identifier.to_string(),
    })
}

use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::embedded_abis::load_and_map_abi;
use visualsign_ethereum::networks::parse_network;

#[derive(Parser, Debug)]
#[command(name = "visualsign-ethereum")]
#[command(version = "1.0")]
#[command(about = "Converts raw Ethereum transactions to visual signing properties")]
struct Args {
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
}

fn build_registry() -> TransactionConverterRegistry {
    let mut registry = TransactionConverterRegistry::new();
    registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
        Chain::Ethereum,
        visualsign_ethereum::EthereumVisualSignConverter::new(),
    );
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}

fn create_chain_metadata(network: Option<String>) -> ChainMetadata {
    let network_id = if let Some(network) = network {
        let Some(id) = parse_network(&network) else {
            eprintln!(
                "Error: Invalid network '{network}'. Supported formats:\n\
                 - Chain ID (numeric): 1 (Ethereum), 137 (Polygon), 42161 (Arbitrum)\n\
                 - Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET\n\
                 \n\
                 Run with --help for full list of supported networks."
            );
            std::process::exit(1);
        };
        id
    } else {
        eprintln!("Warning: No network specified, defaulting to ETHEREUM_MAINNET (chain_id: 1)");
        parse_network("ETHEREUM_MAINNET").expect("ETHEREUM_MAINNET should always be valid")
    };

    ChainMetadata {
        metadata: Some(Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
            abi: None,
        })),
    }
}

fn build_abi_registry(abi_json_mappings: &[String], chain_id: u64) -> (AbiRegistry, usize) {
    let mut registry = AbiRegistry::new();
    let mut valid_count = 0;

    for mapping in abi_json_mappings {
        let components = match parse_mapping(mapping) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  Warning: Invalid ABI mapping '{mapping}': {e}");
                continue;
            }
        };
        match load_and_map_abi(
            &mut registry,
            &components.name,
            &components.path,
            chain_id,
            &components.identifier,
        ) {
            Ok(()) => {
                valid_count += 1;
                eprintln!(
                    "  Loaded ABI '{}' from {} and mapped to {}",
                    components.name, components.path, components.identifier
                );
            }
            Err(e) => {
                eprintln!(
                    "  Warning: Failed to load/map ABI '{}': {e}",
                    components.name
                );
            }
        }
    }

    (registry, valid_count)
}

/// Ethereum parser CLI.
pub struct Cli;

impl Cli {
    /// Parse and display an Ethereum transaction.
    pub fn execute() {
        let args = Args::parse();

        let metadata = create_chain_metadata(args.network);
        let chain_id =
            visualsign_ethereum::networks::extract_chain_id_from_metadata(Some(&metadata));

        let mut options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: Some(metadata),
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
            abi_registry: None,
        };

        if !args.abi_json_mappings.is_empty() {
            eprintln!("Registering custom ABIs:");
            let (registry, valid_count) =
                build_abi_registry(&args.abi_json_mappings, chain_id.unwrap_or(1));
            eprintln!(
                "Successfully registered {}/{} ABI mappings\n",
                valid_count,
                args.abi_json_mappings.len()
            );
            options.abi_registry = Some(Arc::new(registry));
        }

        let registry = build_registry();
        match registry.convert_transaction(&Chain::Ethereum, &args.transaction, options) {
            Ok(payload) => print_payload(&payload, args.output, args.condensed_only),
            Err(err) => eprintln!("Error: {err:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mapping_valid() {
        let result = parse_mapping("MyToken:/path/to/file.json:0xabcd1234")
            .expect("Valid mapping should parse successfully");
        assert_eq!(result.name, "MyToken");
        assert_eq!(result.path, "/path/to/file.json");
        assert_eq!(result.identifier, "0xabcd1234");
    }

    #[test]
    fn test_parse_mapping_windows_path() {
        let result = parse_mapping("MyToken:C:/Users/name/file.json:0xabcd1234")
            .expect("Windows path mapping should parse successfully");
        assert_eq!(result.name, "MyToken");
        assert_eq!(result.path, "C:/Users/name/file.json");
        assert_eq!(result.identifier, "0xabcd1234");
    }

    #[test]
    fn test_parse_mapping_ethereum_address() {
        let result =
            parse_mapping("USDC:/path/to/abi.json:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
                .expect("Ethereum address mapping should parse successfully");
        assert_eq!(result.name, "USDC");
        assert_eq!(result.path, "/path/to/abi.json");
        assert_eq!(
            result.identifier,
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
    }

    #[test]
    fn test_parse_mapping_invalid_format() {
        assert!(parse_mapping("NoColons").is_err());
        assert!(parse_mapping("OnlyOne:Colon").is_err());
        assert!(parse_mapping("::EmptyComponents").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_name() {
        assert!(parse_mapping(":/path/to/file.json:0xabcd").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_path() {
        assert!(parse_mapping("MyToken::0xabcd").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_identifier() {
        assert!(parse_mapping("MyToken:/path/to/file.json:").is_err());
    }
}
