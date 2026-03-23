use std::collections::BTreeMap;

use clap::Args as ClapArgs;
use generated::parser::{Abi, ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign_ethereum::networks::parse_network;

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

    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        create_chain_metadata(network, &self.args.abi_json_mappings)
    }
}

/// Load ABI JSON files and create `BTreeMap` for `EthereumMetadata.abi_mappings`
fn build_abi_mappings_from_files(abi_json_mappings: &[String]) -> (BTreeMap<String, Abi>, usize) {
    mapping_parser::load_mappings(
        abi_json_mappings,
        "ABI",
        "UniswapV2:/home/user/uniswap.json:0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f",
        "ContractAddress",
        |_components, json| Abi {
            value: json,
            signature: None,
        },
    )
}

/// Creates Ethereum chain metadata from the network argument.
/// Defaults to `ETHEREUM_MAINNET` if no network is specified.
/// Returns an error if the network identifier is invalid.
///
/// # Panics
///
/// Panics if `ETHEREUM_MAINNET` cannot be parsed (should never happen).
pub fn create_chain_metadata(
    network: Option<String>,
    abi_json_mappings: &[String],
) -> Result<Option<ChainMetadata>, String> {
    let network_id = if let Some(network) = network {
        let Some(network_id) = parse_network(&network) else {
            return Err(format!(
                "Invalid network '{network}'. Supported formats:\n\
                 - Chain ID (numeric): 1 (Ethereum), 137 (Polygon), 42161 (Arbitrum)\n\
                 - Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET\n\
                 \n\
                 Run with --help for full list of supported networks."
            ));
        };
        network_id
    } else {
        eprintln!("Warning: No network specified, defaulting to ETHEREUM_MAINNET (chain_id: 1)");
        parse_network("ETHEREUM_MAINNET").expect("ETHEREUM_MAINNET should always be valid")
    };

    let abi_mappings = if abi_json_mappings.is_empty() {
        BTreeMap::new()
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

    Ok(Some(ChainMetadata {
        metadata: Some(Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
            abi: None,
            abi_mappings,
        })),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn write_temp_json(name: &str, content: &str) -> std::path::PathBuf {
        crate::test_utils::write_temp_json("vsp_eth_tests", name, content)
    }

    #[test]
    fn test_create_chain_metadata_defaults_to_mainnet() {
        let meta = create_chain_metadata(None, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ETHEREUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_network_name() {
        let meta = create_chain_metadata(Some("POLYGON_MAINNET".to_string()), &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "POLYGON_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_chain_id() {
        let meta = create_chain_metadata(Some("42161".to_string()), &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ARBITRUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_empty_abi_mappings() {
        let meta = create_chain_metadata(None, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert!(eth.abi_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_with_abi_mappings() {
        let path = write_temp_json("eth_abi.json", r#"[{"type":"function","name":"swap"}]"#);
        let mappings = vec![format!("Uniswap:{}:0xABCD", path.display())];

        let meta = create_chain_metadata(None, &mappings)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 1);
        let abi = eth.abi_mappings.get("0xABCD").expect("mapping present");
        assert!(abi.value.contains("swap"));
        assert!(abi.signature.is_none());
    }

    #[test]
    fn test_create_chain_metadata_invalid_abi_file_skipped() {
        let mappings = vec!["BadABI:/nonexistent/abi.json:0xDEAD".to_string()];
        let meta = create_chain_metadata(None, &mappings)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert!(eth.abi_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_multiple_abi_mappings() {
        let path1 = write_temp_json("abi_a.json", r#"{"fn":"a"}"#);
        let path2 = write_temp_json("abi_b.json", r#"{"fn":"b"}"#);
        let mappings = vec![
            format!("A:{}:0x1111", path1.display()),
            format!("B:{}:0x2222", path2.display()),
        ];

        let meta = create_chain_metadata(None, &mappings)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 2);
        assert!(eth.abi_mappings.contains_key("0x1111"));
        assert!(eth.abi_mappings.contains_key("0x2222"));
    }

    #[test]
    fn test_create_chain_metadata_invalid_network_returns_error() {
        let result = create_chain_metadata(Some("INVALID_NETWORK".to_string()), &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid network"));
    }
}
