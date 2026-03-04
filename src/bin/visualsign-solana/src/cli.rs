use clap::Parser;
use generated::parser::{ChainMetadata, Idl, SolanaIdlType, SolanaMetadata, chain_metadata::Metadata};
use parser_cli::display::{OutputFormat, print_payload};
use std::collections::HashMap;

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

fn load_json_file(path: &str) -> Result<String, String> {
    let json_content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file at {path}: {e}"))?;
    serde_json::from_str::<serde_json::Value>(&json_content)
        .map_err(|e| format!("Invalid JSON in file {path}: {e}"))?;
    Ok(json_content)
}
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};

#[derive(Parser, Debug)]
#[command(name = "visualsign-solana")]
#[command(version = "1.0")]
#[command(about = "Converts raw Solana transactions to visual signing properties")]
struct Args {
    #[arg(short, long, value_name = "RAW_TX", help = "Raw transaction hex string")]
    transaction: String,

    #[arg(short, long, default_value = "text", help = "Output format")]
    output: OutputFormat,

    #[arg(long, help = "Show only condensed view (what hardware wallets display)")]
    condensed_only: bool,

    #[arg(
        long = "idl-json-mappings",
        value_name = "IDL_NAME:FILE_PATH:PROGRAM_ID",
        help = "Map custom IDL JSON file to Solana program. Format: IdlName:/path/to/idl.json:base58_program_id. Can be used multiple times"
    )]
    idl_json_mappings: Vec<String>,
}

fn build_registry() -> TransactionConverterRegistry {
    let mut registry = TransactionConverterRegistry::new();
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

fn build_idl_mappings(idl_json_mappings: &[String]) -> HashMap<String, Idl> {
    if idl_json_mappings.is_empty() {
        return HashMap::new();
    }

    eprintln!("Loading custom IDLs:");
    let mut mappings = HashMap::new();
    let mut valid_count = 0;

    for mapping in idl_json_mappings {
        let components = match parse_mapping(mapping) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  Warning: Invalid IDL mapping '{mapping}': {e}");
                continue;
            }
        };
        match load_json_file(&components.path) {
            Ok(json_str) => {
                let idl = Idl {
                    value: json_str,
                    idl_type: Some(SolanaIdlType::Anchor as i32),
                    idl_version: None,
                    signature: None,
                    program_name: Some(components.name.clone()),
                };
                mappings.insert(components.identifier.clone(), idl);
                valid_count += 1;
                eprintln!(
                    "  Loaded IDL '{}' from {} for program {}",
                    components.name, components.path, components.identifier
                );
            }
            Err(e) => {
                eprintln!(
                    "  Warning: Failed to load IDL '{}' from '{}': {e}",
                    components.name, components.path
                );
            }
        }
    }

    eprintln!(
        "Successfully loaded {}/{} IDL mappings\n",
        valid_count,
        idl_json_mappings.len()
    );
    mappings
}

fn create_chain_metadata(idl_json_mappings: &[String]) -> Option<ChainMetadata> {
    let idl_mappings = build_idl_mappings(idl_json_mappings);
    if idl_mappings.is_empty() {
        return None;
    }
    Some(ChainMetadata {
        metadata: Some(Metadata::Solana(SolanaMetadata {
            network_id: None,
            idl: None,
            idl_mappings,
        })),
    })
}

/// Solana parser CLI.
pub struct Cli;

impl Cli {
    /// Parse and display a Solana transaction.
    pub fn execute() {
        let args = Args::parse();

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: create_chain_metadata(&args.idl_json_mappings),
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
            abi_registry: None,
        };

        let registry = build_registry();
        match registry.convert_transaction(&Chain::Solana, &args.transaction, options) {
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
    fn test_parse_mapping_solana_program_id() {
        let result =
            parse_mapping("Jupiter:/path/to/idl.json:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
                .expect("Solana program ID mapping should parse successfully");
        assert_eq!(result.name, "Jupiter");
        assert_eq!(result.path, "/path/to/idl.json");
        assert_eq!(result.identifier, "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
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
