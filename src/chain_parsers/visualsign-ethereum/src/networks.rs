//! EVM chain definitions and utilities
//!
//! This module provides chain ID constants and name lookups for EVM-compatible chains.
//!
//! Chain ID source: <https://github.com/DefiLlama/chainlist/tree/main/constants/additionalChainRegistry>
//! For additional chains, consult the DefiLlama chainlist repository.

/// Macro to define network constants and generate lookup functions from a single source.
///
/// Each entry: (chain_module, network_const, chain_id, display_name)
/// Generates:
/// - `id::{chain_module}::{network_const}` constants
/// - `get_network_name()` - chain_id -> display name
/// - `chain_id_to_network_id()` - chain_id -> canonical ID (e.g., "ETHEREUM_MAINNET")
/// - `network_id_to_chain_id()` - canonical ID -> chain_id
macro_rules! define_networks {
    (
        $(
            $chain:ident {
                $( $network:ident = $id:expr => $display:expr ),* $(,)?
            }
        ),* $(,)?
    ) => {
        /// Chain ID constants grouped by network family
        ///
        /// Use these constants instead of magic numbers throughout the codebase.
        /// Example: `id::ethereum::MAINNET` instead of `1u64`
        pub mod id {
            $(
                pub mod $chain {
                    $( pub const $network: u64 = $id; )*
                }
            )*
        }

        /// Returns a human-readable network name from chain ID.
        ///
        /// Only includes networks defined in the `id` module.
        pub fn get_network_name(chain_id: Option<u64>) -> String {
            match chain_id {
                $($(
                    Some(id::$chain::$network) => $display.to_string(),
                )*)*
                Some(chain_id) => format!("Unknown Network (Chain ID: {chain_id})"),
                None => "Unknown Network".to_string(),
            }
        }

        /// Converts a chain ID to its canonical network identifier string.
        ///
        /// Returns the standardized network identifier (e.g., "ETHEREUM_MAINNET") for known chains,
        /// or `None` for unknown chain IDs.
        pub fn chain_id_to_network_id(chain_id: u64) -> Option<&'static str> {
            match chain_id {
                $($(
                    id::$chain::$network => Some(concat!(stringify!($chain), "_", stringify!($network)).to_uppercase().leak()),
                )*)*
                _ => None,
            }
        }

        /// Converts a canonical network identifier string to its chain ID.
        ///
        /// This is the inverse of `chain_id_to_network_id`. Case-insensitive.
        pub fn network_id_to_chain_id(network_id: &str) -> Option<u64> {
            // Compare case-insensitively
            let input = network_id.to_uppercase();
            $($(
                if input == concat!(stringify!($chain), "_", stringify!($network)).to_uppercase() {
                    return Some(id::$chain::$network);
                }
            )*)*
            None
        }
    };
}

// Define all supported networks
// Format: chain { NETWORK = chain_id => "Display Name" }
define_networks! {
    // L1 Chains
    ethereum {
        MAINNET = 1 => "Ethereum Mainnet",
        SEPOLIA = 11155111 => "Ethereum Sepolia",
        GOERLI = 5 => "Ethereum Goerli (deprecated)",
        HOLESKY = 17000 => "Ethereum Holesky",
    },
    bsc {
        MAINNET = 56 => "BNB Smart Chain Mainnet",
        TESTNET = 97 => "BNB Smart Chain Testnet",
    },
    polygon {
        MAINNET = 137 => "Polygon Mainnet",
        AMOY = 80002 => "Polygon Amoy",
    },
    avalanche {
        MAINNET = 43114 => "Avalanche C-Chain",
        FUJI = 43113 => "Avalanche Fuji Testnet",
    },
    fantom {
        MAINNET = 250 => "Fantom Opera",
    },
    gnosis {
        MAINNET = 100 => "Gnosis Chain",
    },
    celo {
        MAINNET = 42220 => "Celo Mainnet",
        ALFAJORES = 44787 => "Celo Alfajores Testnet",
    },

    // L2 Chains - Optimistic Rollups
    optimism {
        MAINNET = 10 => "OP Mainnet",
        SEPOLIA = 11155420 => "OP Sepolia",
    },
    arbitrum {
        MAINNET = 42161 => "Arbitrum One",
        SEPOLIA = 421614 => "Arbitrum Sepolia",
    },
    base {
        MAINNET = 8453 => "Base",
        SEPOLIA = 84532 => "Base Sepolia",
    },
    blast {
        MAINNET = 81457 => "Blast",
    },
    mantle {
        MAINNET = 5000 => "Mantle",
    },
    worldchain {
        MAINNET = 480 => "World Chain",
    },

    // L2 Chains - ZK Rollups
    zksync {
        MAINNET = 324 => "zkSync Era",
    },
    linea {
        MAINNET = 59144 => "Linea",
    },
    scroll {
        MAINNET = 534352 => "Scroll",
    },

    // App-Specific Chains
    zora {
        MAINNET = 7777777 => "Zora",
    },
    unichain {
        MAINNET = 130 => "Unichain",
    },
}

/// Parses a network identifier from either a chain ID (as string) or network name.
///
/// Accepts:
/// - Numeric chain ID as string: "1", "137", "42161"
/// - Canonical network name: "ETHEREUM_MAINNET", "POLYGON_MAINNET" (case-insensitive)
///
/// Returns the canonical network identifier string, or `None` if not recognized.
///
/// # Examples
/// ```
/// use visualsign_ethereum::networks::parse_network;
///
/// // By chain ID
/// assert_eq!(parse_network("1"), Some("ETHEREUM_MAINNET".to_string()));
/// assert_eq!(parse_network("137"), Some("POLYGON_MAINNET".to_string()));
///
/// // By name (case-insensitive)
/// assert_eq!(parse_network("ethereum_mainnet"), Some("ETHEREUM_MAINNET".to_string()));
/// ```
pub fn parse_network(input: &str) -> Option<String> {
    // First, try parsing as a chain ID
    if let Ok(chain_id) = input.parse::<u64>() {
        return chain_id_to_network_id(chain_id).map(|s| s.to_string());
    }

    // Otherwise, validate as a known network name (case-insensitive)
    if network_id_to_chain_id(input).is_some() {
        // Return the canonical form from chain_id_to_network_id for consistency
        let chain_id = network_id_to_chain_id(input)?;
        chain_id_to_network_id(chain_id).map(|s| s.to_string())
    } else {
        None
    }
}

/// Extracts chain_id from ChainMetadata with fallback to ETHEREUM_MAINNET
///
/// Primary function for getting chain_id from metadata in both CLI and gRPC paths.
/// Logs warnings to stderr if network_id is missing or invalid.
///
/// # Arguments
/// * `chain_metadata` - Optional ChainMetadata from VisualSignOptions
///
/// # Returns
/// Chain ID as u64. Defaults to ETHEREUM_MAINNET (chain_id = 1) on error with stderr warning.
///
/// # Examples
/// ```ignore
/// use visualsign_ethereum::networks::extract_chain_id_from_metadata;
/// use generated::parser::{ChainMetadata, EthereumMetadata, chain_metadata};
///
/// let metadata = ChainMetadata {
///     metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
///         network_id: Some("POLYGON_MAINNET".to_string()),
///         abi: None,
///     })),
/// };
///
/// let chain_id = extract_chain_id_from_metadata(Some(&metadata));
/// assert_eq!(chain_id, 137);
/// ```
pub fn extract_chain_id_from_metadata(
    chain_metadata: Option<&generated::parser::ChainMetadata>,
) -> u64 {
    use generated::parser::chain_metadata;

    const DEFAULT_CHAIN_ID: u64 = id::ethereum::MAINNET;
    const DEFAULT_NETWORK_NAME: &str = "ETHEREUM_MAINNET";

    let Some(metadata) = chain_metadata else {
        eprintln!(
            "Warning: No chain metadata provided, defaulting to {DEFAULT_NETWORK_NAME} (chain_id: {DEFAULT_CHAIN_ID})"
        );
        return DEFAULT_CHAIN_ID;
    };

    let Some(ref inner_metadata) = metadata.metadata else {
        eprintln!(
            "Warning: Chain metadata is empty, defaulting to {DEFAULT_NETWORK_NAME} (chain_id: {DEFAULT_CHAIN_ID})"
        );
        return DEFAULT_CHAIN_ID;
    };

    match inner_metadata {
        chain_metadata::Metadata::Ethereum(eth_metadata) => {
            let network_id = match &eth_metadata.network_id {
                Some(id) => id.as_str(),
                None => {
                    eprintln!(
                        "Warning: Ethereum metadata missing network_id, defaulting to {DEFAULT_NETWORK_NAME} (chain_id: {DEFAULT_CHAIN_ID})"
                    );
                    return DEFAULT_CHAIN_ID;
                }
            };

            match network_id_to_chain_id(network_id) {
                Some(chain_id) => chain_id,
                None => {
                    eprintln!(
                        "Warning: Unknown network_id '{network_id}', defaulting to {DEFAULT_NETWORK_NAME} (chain_id: {DEFAULT_CHAIN_ID})"
                    );
                    DEFAULT_CHAIN_ID
                }
            }
        }
        chain_metadata::Metadata::Solana(_) => {
            eprintln!(
                "Warning: Solana metadata provided for Ethereum parser, defaulting to {DEFAULT_NETWORK_NAME} (chain_id: {DEFAULT_CHAIN_ID})"
            );
            DEFAULT_CHAIN_ID
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All 28 networks for parametrized testing
    const ALL_NETWORKS: &[(u64, &str, &str)] = &[
        (
            id::ethereum::MAINNET,
            "ETHEREUM_MAINNET",
            "Ethereum Mainnet",
        ),
        (
            id::ethereum::SEPOLIA,
            "ETHEREUM_SEPOLIA",
            "Ethereum Sepolia",
        ),
        (
            id::ethereum::GOERLI,
            "ETHEREUM_GOERLI",
            "Ethereum Goerli (deprecated)",
        ),
        (
            id::ethereum::HOLESKY,
            "ETHEREUM_HOLESKY",
            "Ethereum Holesky",
        ),
        (id::bsc::MAINNET, "BSC_MAINNET", "BNB Smart Chain Mainnet"),
        (id::bsc::TESTNET, "BSC_TESTNET", "BNB Smart Chain Testnet"),
        (id::polygon::MAINNET, "POLYGON_MAINNET", "Polygon Mainnet"),
        (id::polygon::AMOY, "POLYGON_AMOY", "Polygon Amoy"),
        (
            id::avalanche::MAINNET,
            "AVALANCHE_MAINNET",
            "Avalanche C-Chain",
        ),
        (
            id::avalanche::FUJI,
            "AVALANCHE_FUJI",
            "Avalanche Fuji Testnet",
        ),
        (id::fantom::MAINNET, "FANTOM_MAINNET", "Fantom Opera"),
        (id::gnosis::MAINNET, "GNOSIS_MAINNET", "Gnosis Chain"),
        (id::celo::MAINNET, "CELO_MAINNET", "Celo Mainnet"),
        (
            id::celo::ALFAJORES,
            "CELO_ALFAJORES",
            "Celo Alfajores Testnet",
        ),
        (id::optimism::MAINNET, "OPTIMISM_MAINNET", "OP Mainnet"),
        (id::optimism::SEPOLIA, "OPTIMISM_SEPOLIA", "OP Sepolia"),
        (id::arbitrum::MAINNET, "ARBITRUM_MAINNET", "Arbitrum One"),
        (
            id::arbitrum::SEPOLIA,
            "ARBITRUM_SEPOLIA",
            "Arbitrum Sepolia",
        ),
        (id::base::MAINNET, "BASE_MAINNET", "Base"),
        (id::base::SEPOLIA, "BASE_SEPOLIA", "Base Sepolia"),
        (id::blast::MAINNET, "BLAST_MAINNET", "Blast"),
        (id::mantle::MAINNET, "MANTLE_MAINNET", "Mantle"),
        (id::worldchain::MAINNET, "WORLDCHAIN_MAINNET", "World Chain"),
        (id::zksync::MAINNET, "ZKSYNC_MAINNET", "zkSync Era"),
        (id::linea::MAINNET, "LINEA_MAINNET", "Linea"),
        (id::scroll::MAINNET, "SCROLL_MAINNET", "Scroll"),
        (id::zora::MAINNET, "ZORA_MAINNET", "Zora"),
        (id::unichain::MAINNET, "UNICHAIN_MAINNET", "Unichain"),
    ];

    #[test]
    fn test_all_networks_get_network_name() {
        for &(chain_id, _, display_name) in ALL_NETWORKS {
            assert_eq!(get_network_name(Some(chain_id)), display_name);
        }
    }

    #[test]
    fn test_all_networks_chain_id_to_network_id() {
        for &(chain_id, network_id, _) in ALL_NETWORKS {
            assert_eq!(chain_id_to_network_id(chain_id), Some(network_id));
        }
    }

    #[test]
    fn test_all_networks_network_id_to_chain_id() {
        for &(chain_id, network_id, _) in ALL_NETWORKS {
            assert_eq!(network_id_to_chain_id(network_id), Some(chain_id));
        }
    }

    #[test]
    fn test_all_networks_parse_network_by_chain_id() {
        for &(chain_id, network_id, _) in ALL_NETWORKS {
            assert_eq!(
                parse_network(&chain_id.to_string()),
                Some(network_id.to_string())
            );
        }
    }

    #[test]
    fn test_all_networks_parse_network_by_name() {
        for &(_, network_id, _) in ALL_NETWORKS {
            assert_eq!(parse_network(network_id), Some(network_id.to_string()));
        }
    }

    #[test]
    fn test_all_networks_roundtrip() {
        for &(chain_id, network_id, _) in ALL_NETWORKS {
            assert_eq!(chain_id_to_network_id(chain_id), Some(network_id));
            assert_eq!(network_id_to_chain_id(network_id), Some(chain_id));
            assert_eq!(
                parse_network(&chain_id.to_string()),
                Some(network_id.to_string())
            );
        }
    }

    #[test]
    fn test_network_id_case_insensitive() {
        // Test case insensitivity for representative networks
        assert_eq!(
            network_id_to_chain_id("ethereum_mainnet"),
            Some(id::ethereum::MAINNET)
        );
        assert_eq!(
            network_id_to_chain_id("Ethereum_Mainnet"),
            Some(id::ethereum::MAINNET)
        );
        assert_eq!(
            network_id_to_chain_id("POLYGON_mainnet"),
            Some(id::polygon::MAINNET)
        );
        assert_eq!(
            parse_network("arbitrum_mainnet"),
            Some("ARBITRUM_MAINNET".to_string())
        );
        assert_eq!(
            parse_network("Base_Mainnet"),
            Some("BASE_MAINNET".to_string())
        );
    }

    #[test]
    fn test_get_network_name_none() {
        assert_eq!(get_network_name(None), "Unknown Network");
    }

    #[test]
    fn test_get_network_name_unknown_chain_id() {
        let result = get_network_name(Some(999999999));
        assert!(result.contains("Unknown Network"));
        assert!(result.contains("999999999"));
    }

    #[test]
    fn test_chain_id_to_network_id_unknown() {
        assert_eq!(chain_id_to_network_id(999999999), None);
        assert_eq!(chain_id_to_network_id(0), None);
        assert_eq!(chain_id_to_network_id(u64::MAX), None);
    }

    #[test]
    fn test_network_id_to_chain_id_unknown() {
        assert_eq!(network_id_to_chain_id("UNKNOWN_NETWORK"), None);
        assert_eq!(network_id_to_chain_id(""), None);
        assert_eq!(network_id_to_chain_id("ETHEREUM_TESTNET"), None);
        assert_eq!(network_id_to_chain_id("POLYGON_SEPOLIA"), None);
    }

    #[test]
    fn test_parse_network_invalid_chain_id() {
        assert_eq!(parse_network("999999999"), None);
        assert_eq!(parse_network("0"), None);
    }

    #[test]
    fn test_parse_network_invalid_name() {
        assert_eq!(parse_network("UNKNOWN_NETWORK"), None);
        assert_eq!(parse_network("ETHEREUM_TESTNET"), None);
        assert_eq!(parse_network(""), None);
    }

    #[test]
    fn test_parse_network_non_numeric() {
        assert_eq!(parse_network("not_a_number"), None);
        assert_eq!(parse_network("chain-1"), None);
        assert_eq!(parse_network("0x1"), None);
    }

    #[test]
    fn test_constants_are_unique() {
        let mut seen_ids = std::collections::HashSet::new();
        for &(chain_id, _, _) in ALL_NETWORKS {
            assert!(seen_ids.insert(chain_id));
        }

        let mut seen_names = std::collections::HashSet::new();
        for &(_, network_id, _) in ALL_NETWORKS {
            assert!(seen_names.insert(network_id));
        }

        let mut seen_displays = std::collections::HashSet::new();
        for &(_, _, display) in ALL_NETWORKS {
            assert!(seen_displays.insert(display));
        }
    }

    #[test]
    fn test_network_id_format_consistency() {
        for &(_, network_id, _) in ALL_NETWORKS {
            assert!(network_id.contains('_'));
            let parts: Vec<&str> = network_id.split('_').collect();
            assert_eq!(parts.len(), 2);
            let (chain_part, network_part) = (parts[0], parts[1]);
            assert!(
                chain_part
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_numeric())
            );
            assert!(
                network_part
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_numeric())
            );
        }
    }
}
