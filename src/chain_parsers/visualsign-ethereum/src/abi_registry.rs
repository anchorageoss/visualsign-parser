//! ABI Registry for compile-time embedded JSON ABIs
//!
//! This module provides a registry for storing and looking up contract ABIs
//! that are embedded at compile-time using `include_str!` macro.
//!
//! ABIs must be embedded at compile-time (like `sol!` macro) for:
//! - Security: ABIs validated during compilation, not runtime
//! - Performance: No file I/O or JSON parsing overhead
//! - Determinism: Same binary always uses same ABIs

use std::collections::HashMap;
use std::sync::Arc;

use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;

/// Type alias for chain ID
pub type ChainId = u64;

/// Registry for compile-time embedded ABIs
///
/// Stores parsed JsonAbi instances and maps contract addresses to ABI names.
///
/// # Example
///
/// ```ignore
/// const MY_CONTRACT_ABI: &str = include_str!("contract.abi.json");
///
/// let mut registry = AbiRegistry::new();
/// registry.register_abi("MyContract", MY_CONTRACT_ABI)?;
/// registry.map_address(1, address, "MyContract");
///
/// let abi = registry.get_abi_for_address(1, address);
/// ```
#[derive(Clone)]
pub struct AbiRegistry {
    /// Maps ABI name -> parsed JsonAbi
    abis: Arc<HashMap<String, Arc<JsonAbi>>>,
    /// Maps (chain_id, contract_address) -> ABI name
    address_mappings: Arc<HashMap<(ChainId, Address), String>>,
}

impl AbiRegistry {
    /// Creates a new empty ABI registry
    pub fn new() -> Self {
        Self {
            abis: Arc::new(HashMap::new()),
            address_mappings: Arc::new(HashMap::new()),
        }
    }

    /// Registers an ABI with the given name
    ///
    /// The ABI JSON string should be embedded at compile-time using `include_str!`.
    ///
    /// # Arguments
    /// * `name` - Identifier for this ABI (e.g., "SimpleToken", "UniswapV3")
    /// * `abi_json` - JSON string containing the ABI definition
    ///
    /// # Returns
    /// * `Ok(())` if ABI was successfully parsed and registered
    /// * `Err` if JSON parsing fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut registry = AbiRegistry::new();
    /// const ABI_JSON: &str = include_str!("abi.json");
    /// registry.register_abi("MyContract", ABI_JSON)?;
    /// ```
    pub fn register_abi(&mut self, name: &str, abi_json: &str) -> Result<(), Box<dyn std::error::Error>> {
        let abi = serde_json::from_str::<JsonAbi>(abi_json)?;
        Arc::get_mut(&mut self.abis)
            .expect("ABI map should be mutable")
            .insert(name.to_string(), Arc::new(abi));
        Ok(())
    }

    /// Maps a contract address to an ABI name for a specific chain
    ///
    /// # Arguments
    /// * `chain_id` - The blockchain chain ID (e.g., 1 for Ethereum Mainnet)
    /// * `address` - The contract address
    /// * `abi_name` - The ABI name (must be previously registered)
    pub fn map_address(&mut self, chain_id: ChainId, address: Address, abi_name: &str) {
        Arc::get_mut(&mut self.address_mappings)
            .expect("Address mappings should be mutable")
            .insert((chain_id, address), abi_name.to_string());
    }

    /// Gets the ABI for a specific contract address on a given chain
    ///
    /// # Arguments
    /// * `chain_id` - The blockchain chain ID
    /// * `address` - The contract address
    ///
    /// # Returns
    /// * `Some(Arc<JsonAbi>)` if address is mapped and ABI is registered
    /// * `None` if address is not mapped or ABI not found
    pub fn get_abi_for_address(&self, chain_id: ChainId, address: Address) -> Option<Arc<JsonAbi>> {
        let abi_name = self.address_mappings.get(&(chain_id, address))?;
        self.abis.get(abi_name).cloned()
    }

    /// Gets an ABI by name
    ///
    /// # Arguments
    /// * `name` - The ABI name (as registered with `register_abi`)
    ///
    /// # Returns
    /// * `Some(Arc<JsonAbi>)` if ABI is registered
    /// * `None` if ABI not found
    pub fn get_abi(&self, name: &str) -> Option<Arc<JsonAbi>> {
        self.abis.get(name).cloned()
    }

    /// Lists all registered ABI names
    pub fn list_abis(&self) -> Vec<&str> {
        self.abis.keys().map(|s| s.as_str()).collect()
    }

    /// Lists all address mappings for a given chain
    pub fn list_mappings_for_chain(&self, chain_id: ChainId) -> Vec<(Address, &str)> {
        self.address_mappings
            .iter()
            .filter(|((cid, _), _)| *cid == chain_id)
            .map(|((_, addr), name)| (*addr, name.as_str()))
            .collect()
    }
}

impl Default for AbiRegistry {
    fn default() -> Self {
        Self::new()
    }
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
    fn test_register_and_retrieve_abi() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("TestToken", TEST_ABI).expect("Failed to register ABI");

        let abi = registry.get_abi("TestToken");
        assert!(abi.is_some());
    }

    #[test]
    fn test_invalid_json_fails() {
        let mut registry = AbiRegistry::new();
        let result = registry.register_abi("Invalid", "not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_address_mapping() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("TestToken", TEST_ABI).expect("Failed to register ABI");

        let addr = "0x1234567890123456789012345678901234567890"
            .parse::<Address>()
            .unwrap();
        registry.map_address(1, addr, "TestToken");

        let abi = registry.get_abi_for_address(1, addr);
        assert!(abi.is_some());
    }

    #[test]
    fn test_address_not_mapped() {
        let registry = AbiRegistry::new();
        let addr = "0x1234567890123456789012345678901234567890"
            .parse::<Address>()
            .unwrap();

        let abi = registry.get_abi_for_address(1, addr);
        assert!(abi.is_none());
    }

    #[test]
    fn test_different_chains_separate() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("TestToken", TEST_ABI).expect("Failed to register ABI");

        let addr = "0x1234567890123456789012345678901234567890"
            .parse::<Address>()
            .unwrap();

        registry.map_address(1, addr, "TestToken");
        registry.map_address(137, addr, "TestToken");

        // Same address on different chains
        assert!(registry.get_abi_for_address(1, addr).is_some());
        assert!(registry.get_abi_for_address(137, addr).is_some());
        assert!(registry.get_abi_for_address(42161, addr).is_none());
    }

    #[test]
    fn test_list_abis() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("TokenA", TEST_ABI).expect("Failed to register");
        registry.register_abi("TokenB", TEST_ABI).expect("Failed to register");

        let abis = registry.list_abis();
        assert_eq!(abis.len(), 2);
        assert!(abis.contains(&"TokenA"));
        assert!(abis.contains(&"TokenB"));
    }
}
