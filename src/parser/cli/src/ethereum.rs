use std::sync::Arc;

use clap::Args as ClapArgs;
use generated::parser::{ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::embedded_abis::load_and_map_abi;
use visualsign_ethereum::networks::{extract_chain_id_from_metadata, parse_network};

use crate::mapping_parser;

/// CLI arguments specific to Ethereum.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct EthereumArgs {
    /// Map custom ABI JSON file to contract address.
    /// Format: `AbiName:/path/to/abi.json:0xAddress`. Can be used multiple times.
    #[arg(
        long = "abi-json-mappings",
        value_name = "ABI_NAME:FILE_PATH:0xADDRESS"
    )]
    pub abi_json_mappings: Vec<String>,
}

/// [`crate::ChainPlugin`] implementation for Ethereum.
pub struct EthereumPlugin {
    args: EthereumArgs,
}

impl EthereumPlugin {
    /// Creates a new `EthereumPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: EthereumArgs) -> Self {
        Self { args }
    }
}

impl crate::ChainPlugin for EthereumPlugin {
    fn chain(&self) -> Chain {
        Chain::Ethereum
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
            Chain::Ethereum,
            visualsign_ethereum::EthereumVisualSignConverter::new(),
        );
    }

    fn create_metadata(&self, network: Option<String>) -> Option<ChainMetadata> {
        create_chain_metadata(network)
    }

    fn apply_options(&self, options: VisualSignOptions) -> VisualSignOptions {
        apply_abi_registry(options, &self.args.abi_json_mappings)
    }
}

fn build_abi_registry_from_mappings(
    abi_json_mappings: &[String],
    chain_id: u64,
) -> (AbiRegistry, usize) {
    let mut registry = AbiRegistry::new();
    let mut valid_count = 0;

    for mapping in abi_json_mappings {
        match mapping_parser::parse_mapping(mapping) {
            Ok(components) => {
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
            Err(e) => {
                eprintln!("Error parsing ABI mapping: {e}");
                eprintln!("Expected format: Name:/path/to/abi.json:ContractAddress");
                eprintln!(
                    "Example: UniswapV2:/home/user/uniswap.json:0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"
                );
            }
        }
    }

    (registry, valid_count)
}

fn apply_abi_registry(
    mut options: VisualSignOptions,
    abi_json_mappings: &[String],
) -> VisualSignOptions {
    if abi_json_mappings.is_empty() {
        return options;
    }

    let chain_id = if let Some(ref metadata) = options.metadata {
        extract_chain_id_from_metadata(Some(metadata))
    } else {
        eprintln!("Warning: No metadata provided for ABI registry, defaulting to chain_id 1");
        Some(1)
    };

    eprintln!("Registering custom ABIs:");
    let (registry, valid_count) =
        build_abi_registry_from_mappings(abi_json_mappings, chain_id.unwrap_or(1));
    eprintln!(
        "Successfully registered {}/{} ABI mappings\n",
        valid_count,
        abi_json_mappings.len()
    );
    options.abi_registry = Some(Arc::new(registry));
    options
}

/// Creates Ethereum chain metadata from the network argument.
/// Defaults to `ETHEREUM_MAINNET` if no network is specified.
/// Exits with an error if the network identifier is invalid.
///
/// # Panics
///
/// Panics if `ETHEREUM_MAINNET` cannot be parsed (should never happen).
#[must_use]
pub fn create_chain_metadata(network: Option<String>) -> Option<ChainMetadata> {
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
        network_id
    } else {
        eprintln!("Warning: No network specified, defaulting to ETHEREUM_MAINNET (chain_id: 1)");
        parse_network("ETHEREUM_MAINNET").expect("ETHEREUM_MAINNET should always be valid")
    };

    Some(ChainMetadata {
        metadata: Some(Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
            abi: None,
            abi_mappings: {
                #[allow(clippy::default_trait_access)]
                Default::default()
            },
        })),
    })
}
