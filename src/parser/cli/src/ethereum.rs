use std::collections::HashMap;

use clap::Args as ClapArgs;
use generated::parser::{Abi, AbiType, ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
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

    /// Declare that a proxy address delegates to an implementation address. The
    /// proxy's calldata is decoded against the implementation's ABI. Both addresses
    /// should also be supplied via `--abi-json-mappings` (a proxy with no ABI file
    /// gets an empty ABI synthesized). Format: `0xProxy:0xImpl`. Repeatable.
    #[arg(long = "abi-proxy-mappings", value_name = "0xPROXY:0xIMPL")]
    pub abi_proxy_mappings: Vec<String>,
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
        create_chain_metadata(
            network,
            &self.args.abi_json_mappings,
            &self.args.abi_proxy_mappings,
        )
    }
}

/// Validate an Ethereum address string: `0x` prefix + 40 hex chars.
fn validate_eth_address(addr: &str) -> Result<(), String> {
    let hex = addr
        .strip_prefix("0x")
        .or_else(|| addr.strip_prefix("0X"))
        .ok_or("must start with 0x")?;
    if hex.len() != 40 {
        return Err(format!(
            "expected 0x + 40 hex chars, got {} hex chars",
            hex.len()
        ));
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("contains non-hex characters".to_string());
    }
    Ok(())
}

/// Load ABI JSON files and build mappings for `EthereumMetadata.abi_mappings`.
fn build_abi_mappings_from_files(abi_json_mappings: &[String]) -> (HashMap<String, Abi>, usize) {
    mapping_parser::load_mappings(
        abi_json_mappings,
        "ABI",
        "UniswapV2:path/to/uniswap.json:0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f",
        "ContractAddress",
        validate_eth_address,
        |_components, json| Abi {
            value: json,
            signature: None,
            ..Default::default()
        },
    )
}

/// Apply `--abi-proxy-mappings` links onto the loaded ABI mappings.
///
/// Each `0xProxy:0xImpl` entry stamps `abi_type = ABI_TYPE_PROXY` and
/// `implementation_address` onto the proxy's `Abi`. If the proxy address has no ABI
/// file entry, an empty (`[]`) ABI is synthesized so resolution still works off the
/// link. Malformed links are skipped with a warning.
fn apply_proxy_mappings(abi_mappings: &mut HashMap<String, Abi>, proxy_mappings: &[String]) {
    for mapping in proxy_mappings {
        let Some((proxy, implementation)) = mapping.split_once(':') else {
            eprintln!(
                "  Warning: Skipping proxy mapping '{mapping}': expected format 0xProxy:0xImpl"
            );
            continue;
        };
        if let Err(e) = validate_eth_address(proxy) {
            eprintln!("  Warning: Skipping proxy mapping '{mapping}': invalid proxy address: {e}");
            continue;
        }
        if let Err(e) = validate_eth_address(implementation) {
            eprintln!(
                "  Warning: Skipping proxy mapping '{mapping}': invalid implementation address: {e}"
            );
            continue;
        }

        let entry = abi_mappings.entry(proxy.to_string()).or_insert_with(|| {
            eprintln!("  Note: proxy '{proxy}' has no ABI file; synthesizing an empty proxy ABI");
            Abi {
                value: "[]".to_string(),
                signature: None,
                ..Default::default()
            }
        });
        entry.abi_type = Some(AbiType::Proxy as i32);
        entry.implementation_address = Some(implementation.to_string());
        eprintln!("  Linked proxy '{proxy}' to implementation '{implementation}'");
    }
}

/// Creates Ethereum chain metadata from the network argument.
/// Defaults to `ETHEREUM_MAINNET` if no network is specified.
/// Returns an error if the network identifier is invalid.
///
/// # Panics
///
/// Panics if `ETHEREUM_MAINNET` cannot be parsed (should never happen).
pub(crate) fn create_chain_metadata(
    network: Option<String>,
    abi_json_mappings: &[String],
    abi_proxy_mappings: &[String],
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

    let mut abi_mappings = if abi_json_mappings.is_empty() {
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

    if !abi_proxy_mappings.is_empty() {
        eprintln!("Applying proxy mappings:");
        apply_proxy_mappings(&mut abi_mappings, abi_proxy_mappings);
        eprintln!();
    }

    Ok(Some(ChainMetadata {
        metadata: Some(Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
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
        let meta = create_chain_metadata(None, &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ETHEREUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_network_name() {
        let meta = create_chain_metadata(Some("POLYGON_MAINNET".to_string()), &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "POLYGON_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_chain_id() {
        let meta = create_chain_metadata(Some("42161".to_string()), &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ARBITRUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_empty_abi_mappings() {
        let meta = create_chain_metadata(None, &[], &[])
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
        let mappings = vec![format!(
            "Uniswap:{}:0xdAC17F958D2ee523a2206206994597C13D831ec7",
            path.display()
        )];

        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 1);
        let abi = eth
            .abi_mappings
            .get("0xdAC17F958D2ee523a2206206994597C13D831ec7")
            .expect("mapping present");
        assert!(abi.value.contains("swap"));
        assert!(abi.signature.is_none());
    }

    #[test]
    fn test_create_chain_metadata_invalid_abi_file_skipped() {
        let mappings = vec![
            "BadABI:/nonexistent/abi.json:0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
        ];
        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert!(eth.abi_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_multiple_abi_mappings() {
        let path1 = write_temp_json("abi_a.json", r#"[{"type":"function","name":"a"}]"#);
        let path2 = write_temp_json("abi_b.json", r#"[{"type":"function","name":"b"}]"#);
        let mappings = vec![
            format!(
                "A:{}:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                path1.display()
            ),
            format!(
                "B:{}:0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                path2.display()
            ),
        ];

        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 2);
        assert!(
            eth.abi_mappings
                .contains_key("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
        );
        assert!(
            eth.abi_mappings
                .contains_key("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
        );
    }

    #[test]
    fn test_create_chain_metadata_invalid_network_returns_error() {
        let result = create_chain_metadata(Some("INVALID_NETWORK".to_string()), &[], &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid network"));
    }

    const PROXY: &str = "0x1111111111111111111111111111111111111111";
    const IMPL: &str = "0x2222222222222222222222222222222222222222";

    #[test]
    fn test_proxy_mapping_stamps_type_on_existing_entry() {
        let proxy_path =
            write_temp_json("proxy.json", r#"[{"type":"function","name":"upgradeTo"}]"#);
        let impl_path = write_temp_json("impl.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![
            format!("Proxy:{}:{PROXY}", proxy_path.display()),
            format!("Impl:{}:{IMPL}", impl_path.display()),
        ];
        let proxy_links = vec![format!("{PROXY}:{IMPL}")];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };

        let proxy_abi = eth.abi_mappings.get(PROXY).expect("proxy entry present");
        assert_eq!(proxy_abi.abi_type, Some(AbiType::Proxy as i32));
        assert_eq!(proxy_abi.implementation_address.as_deref(), Some(IMPL));
        // The proxy keeps its own ABI file content.
        assert!(proxy_abi.value.contains("upgradeTo"));

        // The implementation entry is untouched (defaults to implementation).
        let impl_abi = eth.abi_mappings.get(IMPL).expect("impl entry present");
        assert_eq!(impl_abi.abi_type, None);
    }

    #[test]
    fn test_proxy_mapping_synthesizes_entry_when_no_abi_file() {
        // Only the implementation has an ABI file; the proxy is declared purely
        // via --abi-proxy-mappings.
        let impl_path = write_temp_json("impl2.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![format!("Impl:{}:{IMPL}", impl_path.display())];
        let proxy_links = vec![format!("{PROXY}:{IMPL}")];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };

        let proxy_abi = eth
            .abi_mappings
            .get(PROXY)
            .expect("synthesized proxy entry");
        assert_eq!(proxy_abi.value, "[]");
        assert_eq!(proxy_abi.abi_type, Some(AbiType::Proxy as i32));
        assert_eq!(proxy_abi.implementation_address.as_deref(), Some(IMPL));
    }

    #[test]
    fn test_proxy_mapping_invalid_link_skipped() {
        let impl_path = write_temp_json("impl3.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![format!("Impl:{}:{IMPL}", impl_path.display())];
        // Malformed: missing impl, and bad proxy address.
        let proxy_links = vec!["0xnothex:0xalsobad".to_string(), "noseparator".to_string()];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        // No proxy entry was created; only the implementation remains.
        assert_eq!(eth.abi_mappings.len(), 1);
        assert!(eth.abi_mappings.contains_key(IMPL));
    }
}
