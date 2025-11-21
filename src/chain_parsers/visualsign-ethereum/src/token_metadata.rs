use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Standard for ERC token types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErcStandard {
    /// ERC20 fungible token standard
    #[serde(rename = "ERC20")]
    Erc20,
    /// ERC721 non-fungible token standard
    #[serde(rename = "ERC721")]
    Erc721,
    /// ERC1155 multi-token standard
    #[serde(rename = "ERC1155")]
    Erc1155,
}

impl std::fmt::Display for ErcStandard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErcStandard::Erc20 => write!(f, "ERC20"),
            ErcStandard::Erc721 => write!(f, "ERC721"),
            ErcStandard::Erc1155 => write!(f, "ERC1155"),
        }
    }
}

/// Information about a token asset
///
/// This represents a single token in the blockchain, with its metadata.
/// Used in both the Anchorage format (gRPC ChainMetadata) and internally
/// by the ContractRegistry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenMetadata {
    /// Token symbol (e.g., "USDC", "WETH")
    pub symbol: String,
    /// Token name (e.g., "USD Coin")
    pub name: String,
    /// ERC standard this token implements
    pub erc_standard: ErcStandard,
    /// Contract address of the token
    pub contract_address: String,
    /// Number of decimal places for token amounts
    pub decimals: u8,
}

/// Chain metadata representing network and token information
///
/// This is the canonical format for wallets to send token metadata.
/// Network ID is sent as a string (e.g., "ETHEREUM_MAINNET") and is converted
/// to a numeric chain ID by parse_network_id().
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainMetadata {
    /// Network identifier as string (e.g., "ETHEREUM_MAINNET")
    pub network_id: String,
    /// Map of token symbol to token metadata
    pub assets: HashMap<String, TokenMetadata>,
}

/// Error type for token metadata operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenMetadataError {
    /// Unknown network ID
    UnknownNetworkId(String),
    /// Hash computation error
    HashError(String),
}

impl std::fmt::Display for TokenMetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenMetadataError::UnknownNetworkId(id) => write!(f, "Unknown network ID: {}", id),
            TokenMetadataError::HashError(msg) => write!(f, "Hash error: {}", msg),
        }
    }
}

impl std::error::Error for TokenMetadataError {}

/// Parses a network ID string to its corresponding chain ID
///
/// # Arguments
/// * `network_id` - The network identifier string (e.g., "ETHEREUM_MAINNET")
///
/// # Returns
/// `Ok(chain_id)` for known networks, `Err(TokenMetadataError)` otherwise
///
/// # Supported Networks
/// - "ETHEREUM_MAINNET" -> 1
/// - "POLYGON_MAINNET" -> 137
/// - "ARBITRUM_MAINNET" -> 42161
/// - "OPTIMISM_MAINNET" -> 10
/// - "BASE_MAINNET" -> 8453
///
/// # Examples
/// ```
/// use visualsign_ethereum::token_metadata::parse_network_id;
///
/// assert_eq!(parse_network_id("ETHEREUM_MAINNET"), Ok(1));
/// assert_eq!(parse_network_id("POLYGON_MAINNET"), Ok(137));
/// ```
pub fn parse_network_id(network_id: &str) -> Result<u64, TokenMetadataError> {
    match network_id {
        "ETHEREUM_MAINNET" => Ok(1),
        "POLYGON_MAINNET" => Ok(137),
        "ARBITRUM_MAINNET" => Ok(42161),
        "OPTIMISM_MAINNET" => Ok(10),
        "BASE_MAINNET" => Ok(8453),
        _ => Err(TokenMetadataError::UnknownNetworkId(network_id.to_string())),
    }
}

/// Computes a deterministic SHA256 hash of protobuf bytes
///
/// This function takes the raw protobuf bytes directly (as received from gRPC)
/// and computes a SHA256 hash. The same bytes will always produce the same hash,
/// making this deterministic without needing to reserialize.
///
/// # Arguments
/// * `protobuf_bytes` - The raw protobuf bytes representing ChainMetadata
///
/// # Returns
/// A hex-encoded SHA256 hash string
///
/// # Examples
/// ```
/// use visualsign_ethereum::token_metadata::compute_metadata_hash;
///
/// let bytes = b"example protobuf bytes";
/// let hash1 = compute_metadata_hash(bytes);
/// let hash2 = compute_metadata_hash(bytes);
/// assert_eq!(hash1, hash2); // Same bytes = same hash
/// ```
pub fn compute_metadata_hash(protobuf_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(protobuf_bytes);
    let hash = hasher.finalize();
    format!("{:x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_network_id_ethereum() {
        assert_eq!(parse_network_id("ETHEREUM_MAINNET"), Ok(1));
    }

    #[test]
    fn test_parse_network_id_polygon() {
        assert_eq!(parse_network_id("POLYGON_MAINNET"), Ok(137));
    }

    #[test]
    fn test_parse_network_id_arbitrum() {
        assert_eq!(parse_network_id("ARBITRUM_MAINNET"), Ok(42161));
    }

    #[test]
    fn test_parse_network_id_optimism() {
        assert_eq!(parse_network_id("OPTIMISM_MAINNET"), Ok(10));
    }

    #[test]
    fn test_parse_network_id_base() {
        assert_eq!(parse_network_id("BASE_MAINNET"), Ok(8453));
    }

    #[test]
    fn test_parse_network_id_unknown() {
        let result = parse_network_id("UNKNOWN_NETWORK");
        assert!(result.is_err());
        assert_eq!(
            result,
            Err(TokenMetadataError::UnknownNetworkId(
                "UNKNOWN_NETWORK".to_string()
            ))
        );
    }

    #[test]
    fn test_parse_network_id_empty() {
        let result = parse_network_id("");
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_metadata_hash_deterministic() {
        let bytes = b"example protobuf bytes";
        let hash1 = compute_metadata_hash(bytes);
        let hash2 = compute_metadata_hash(bytes);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_metadata_hash_different_bytes() {
        let bytes1 = b"protobuf bytes 1";
        let bytes2 = b"protobuf bytes 2";

        let hash1 = compute_metadata_hash(bytes1);
        let hash2 = compute_metadata_hash(bytes2);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_metadata_hash_format() {
        let bytes = b"example protobuf bytes";
        let hash = compute_metadata_hash(bytes);

        // SHA256 produces 256 bits = 32 bytes = 64 hex characters
        assert_eq!(hash.len(), 64);
        // Verify it's valid hex
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_compute_metadata_hash_empty_bytes() {
        let bytes = b"";
        let hash = compute_metadata_hash(bytes);

        // Empty bytes should still produce valid SHA256 hash
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_token_metadata_serialization() {
        let token = TokenMetadata {
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            erc_standard: ErcStandard::Erc20,
            contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            decimals: 6,
        };

        let json = serde_json::to_string(&token).expect("Failed to serialize");
        let deserialized: TokenMetadata =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(token, deserialized);
    }

    #[test]
    fn test_chain_metadata_serialization() {
        let mut metadata = ChainMetadata {
            network_id: "ETHEREUM_MAINNET".to_string(),
            assets: HashMap::new(),
        };

        let usdc = TokenMetadata {
            symbol: "USDC".to_string(),
            name: "USD Coin".to_string(),
            erc_standard: ErcStandard::Erc20,
            contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            decimals: 6,
        };

        metadata.assets.insert("USDC".to_string(), usdc);

        let json = serde_json::to_string(&metadata).expect("Failed to serialize");
        let deserialized: ChainMetadata =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(metadata, deserialized);
    }

    #[test]
    fn test_erc_standard_display() {
        assert_eq!(ErcStandard::Erc20.to_string(), "ERC20");
        assert_eq!(ErcStandard::Erc721.to_string(), "ERC721");
        assert_eq!(ErcStandard::Erc1155.to_string(), "ERC1155");
    }

    #[test]
    fn test_erc_standard_serialization() {
        let erc20 = ErcStandard::Erc20;
        let json = serde_json::to_string(&erc20).expect("Failed to serialize");
        let deserialized: ErcStandard = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(erc20, deserialized);
    }
}
