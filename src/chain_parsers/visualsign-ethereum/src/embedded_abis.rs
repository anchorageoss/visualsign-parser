//! Helpers for embedding and managing compile-time ABI JSON files
//!
//! This module provides utilities for applications to register compile-time
//! embedded ABI JSON files for custom contract visualization.
//!
//! # Example: Embedding an ABI at compile-time
//!
//! ```ignore
//! use visualsign_ethereum::embedded_abis::register_embedded_abi;
//! use visualsign_ethereum::abi_registry::AbiRegistry;
//!
//! const MY_CONTRACT_ABI: &str = include_str!("contracts/MyContract.abi.json");
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut registry = AbiRegistry::new();
//!     register_embedded_abi(&mut registry, "MyContract", MY_CONTRACT_ABI)?;
//!     registry.map_address(1, "0x1234...".parse()?, "MyContract");
//!     Ok(())
//! }
//! ```

use alloy_primitives::Address;

use crate::abi_registry::AbiRegistry;

/// Error type for ABI embedding operations
#[derive(Debug, thiserror::Error)]
pub enum AbiEmbeddingError {
    /// Invalid JSON in ABI file
    #[error("Invalid ABI JSON: {0}")]
    InvalidJson(String),
    /// File I/O error
    #[error("Failed to read ABI file: {0}")]
    FileError(String),
}

/// Registers a compile-time embedded ABI JSON string with an AbiRegistry
///
/// # Arguments
/// * `registry` - The ABI registry to register with (mutable)
/// * `name` - Canonical name for this ABI (e.g., "Uniswap V3 Router")
/// * `abi_json` - Embedded ABI JSON string from `include_str!()`
///
/// # Returns
/// * `Ok(())` on successful registration
/// * `Err(AbiEmbeddingError)` if JSON is invalid
///
/// # Example
/// ```ignore
/// const TOKEN_ABI: &str = include_str!("SimpleToken.abi.json");
/// register_embedded_abi(&mut registry, "SimpleToken", TOKEN_ABI)?;
/// ```
pub fn register_embedded_abi(
    registry: &mut AbiRegistry,
    name: &str,
    abi_json: &str,
) -> Result<(), AbiEmbeddingError> {
    registry
        .register_abi(name, abi_json)
        .map_err(|e| AbiEmbeddingError::InvalidJson(e.to_string()))
}

/// Maps a contract address to a registered ABI name for a specific chain
///
/// # Arguments
/// * `registry` - The ABI registry (mutable)
/// * `chain_id` - Blockchain network ID (1 for Ethereum mainnet, etc.)
/// * `address` - Contract address to map
/// * `abi_name` - Name of previously registered ABI
///
/// # Example
/// ```ignore
/// let my_address: Address = "0x1234567890123456789012345678901234567890".parse()?;
/// map_abi_address(&mut registry, 1, my_address, "SimpleToken");
/// ```
pub fn map_abi_address(
    registry: &mut AbiRegistry,
    chain_id: u64,
    address: Address,
    abi_name: &str,
) {
    registry.map_address(chain_id, address, abi_name);
}

/// Parses an ABI address mapping string like "AbiName:0xAddress"
///
/// # Format
/// The input should be in the format: `abi_name:0xaddress`
///
/// # Arguments
/// * `mapping_str` - String in format "AbiName:0xAddress"
///
/// # Returns
/// * `Some((abi_name, address))` if valid
/// * `None` if format is invalid or address fails to parse
///
/// # Example
/// ```ignore
/// if let Some((name, addr)) = parse_abi_address_mapping("SimpleToken:0x1234...") {
///     registry.map_address(chain_id, addr, name);
/// }
/// ```
pub fn parse_abi_address_mapping(mapping_str: &str) -> Option<(&str, Address)> {
    let (abi_name, addr_str) = mapping_str.split_once(':')?;
    let address = addr_str.parse().ok()?;
    Some((abi_name, address))
}

/// Loads an ABI JSON from a file and registers it with the given name
///
/// # Arguments
/// * `registry` - The ABI registry to register with (mutable)
/// * `name` - Name for this ABI (e.g., "MyToken")
/// * `file_path` - Path to the ABI JSON file
///
/// # Returns
/// * `Ok(())` on successful registration
/// * `Err(AbiEmbeddingError)` if file cannot be read or JSON is invalid
pub fn load_abi_from_file(
    registry: &mut AbiRegistry,
    name: &str,
    file_path: &str,
) -> Result<(), AbiEmbeddingError> {
    let abi_json = std::fs::read_to_string(file_path)
        .map_err(|e| AbiEmbeddingError::FileError(format!("{file_path}: {e}")))?;
    register_embedded_abi(registry, name, &abi_json)
}

/// Loads an ABI from a file and maps it to an address
///
/// Convenience function that combines loading and address mapping
pub fn load_and_map_abi(
    registry: &mut AbiRegistry,
    name: &str,
    file_path: &str,
    chain_id: u64,
    address_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    load_abi_from_file(registry, name, file_path)?;
    let address = address_str.parse::<Address>()?;
    registry.map_address(chain_id, address, name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        }
    ]"#;

    #[test]
    fn test_register_embedded_abi_valid() {
        let mut registry = AbiRegistry::new();
        let result = register_embedded_abi(&mut registry, "TestToken", TEST_ABI);
        assert!(result.is_ok());
    }

    #[test]
    fn test_register_embedded_abi_invalid_json() {
        let mut registry = AbiRegistry::new();
        let result = register_embedded_abi(&mut registry, "BadToken", "not valid json");
        assert!(matches!(result, Err(AbiEmbeddingError::InvalidJson(_))));
    }

    #[test]
    fn test_parse_abi_address_mapping_valid() {
        let result =
            parse_abi_address_mapping("TestToken:0x1234567890123456789012345678901234567890");
        assert!(result.is_some());
        let (name, _addr) = result.unwrap();
        assert_eq!(name, "TestToken");
    }

    #[test]
    fn test_parse_abi_address_mapping_invalid_format() {
        let result = parse_abi_address_mapping("NoColon");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_abi_address_mapping_invalid_address() {
        let result = parse_abi_address_mapping("TestToken:notanaddress");
        assert!(result.is_none());
    }

    #[test]
    fn test_map_abi_address() {
        let mut registry = AbiRegistry::new();
        register_embedded_abi(&mut registry, "TestToken", TEST_ABI).unwrap();

        let address: Address = "0x1234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        map_abi_address(&mut registry, 1, address, "TestToken");

        // Verify it was mapped
        assert!(registry.get_abi_for_address(1, address).is_some());
    }

    #[test]
    fn test_integration_register_and_retrieve() {
        const MULTI_ABI: &str = r#"[
            {
                "type": "function",
                "name": "mint",
                "inputs": [
                    {"name": "to", "type": "address"},
                    {"name": "amount", "type": "uint256"}
                ],
                "outputs": [],
                "stateMutability": "nonpayable"
            },
            {
                "type": "function",
                "name": "burn",
                "inputs": [{"name": "amount", "type": "uint256"}],
                "outputs": [],
                "stateMutability": "nonpayable"
            }
        ]"#;

        let mut registry = AbiRegistry::new();

        // Register multiple ABIs
        register_embedded_abi(&mut registry, "SimpleToken", TEST_ABI).unwrap();
        register_embedded_abi(&mut registry, "ExtendedToken", MULTI_ABI).unwrap();

        // Map addresses on different chains
        let addr1: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let addr2: Address = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();

        map_abi_address(&mut registry, 1, addr1, "SimpleToken");
        map_abi_address(&mut registry, 1, addr2, "ExtendedToken");
        map_abi_address(&mut registry, 137, addr1, "ExtendedToken");

        // Verify all mappings
        let abi1_on_mainnet = registry.get_abi_for_address(1, addr1);
        let abi2_on_mainnet = registry.get_abi_for_address(1, addr2);
        let abi1_on_polygon = registry.get_abi_for_address(137, addr1);

        assert!(abi1_on_mainnet.is_some());
        assert!(abi2_on_mainnet.is_some());
        assert!(abi1_on_polygon.is_some());

        // Verify they're different ABIs
        assert_ne!(abi1_on_mainnet, abi2_on_mainnet);

        // Verify unmapped addresses return None
        let unmapped: Address = "0x9999999999999999999999999999999999999999"
            .parse()
            .unwrap();
        assert!(registry.get_abi_for_address(1, unmapped).is_none());
    }

    #[test]
    fn test_integration_cli_abi_parsing() {
        // Simulate CLI argument parsing
        let mapping_strs = vec![
            "Token1:0x1111111111111111111111111111111111111111",
            "Token2:0x2222222222222222222222222222222222222222",
            "InvalidFormat", // Invalid mapping
            "Token3:0x3333333333333333333333333333333333333333",
        ];

        let mut valid_count = 0;
        for mapping_str in &mapping_strs {
            if let Some((_name, _address)) = parse_abi_address_mapping(mapping_str) {
                valid_count += 1;
            }
        }

        assert_eq!(valid_count, 3);
    }
}
