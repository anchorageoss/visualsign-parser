//! ABI Registry for compile-time embedded JSON ABIs
//!
//! This module provides a registry for storing and looking up contract ABIs
//! that are embedded at compile-time using `include_str!` macro.
//!
//! ABIs must be embedded at compile-time (like `sol!` macro) for:
//! - Security: ABIs validated during compilation, not runtime
//! - Performance: No file I/O or JSON parsing overhead
//! - Determinism: Same binary always uses same ABIs

use std::collections::BTreeMap;
use std::sync::Arc;

use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;

/// Type alias for chain ID
pub type ChainId = u64;

/// Classifies what kind of contract an ABI describes.
///
/// Named `AbiKind` to avoid colliding with the generated proto `AbiType` enum.
/// Extraction maps the proto `abi_type` onto this; `Unspecified` collapses into
/// `Implementation` so the default (no type set) keeps today's behaviour.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AbiKind {
    /// The ABI decodes the destination's calldata directly (today's assumption).
    #[default]
    Implementation,
    /// The destination is a proxy that delegates to an implementation; the
    /// calldata should be decoded against the implementation's ABI.
    Proxy,
}

/// Per-address mapping entry: which registered ABI a contract uses, what kind of
/// contract it is, and (for proxies) the implementation address whose ABI decodes
/// the calldata.
#[derive(Clone)]
struct AddressMapping {
    /// Name of the registered ABI (key into `abis`).
    abi_name: String,
    /// Kind of contract at this address.
    abi_kind: AbiKind,
    /// For proxies, the implementation address to resolve for calldata decoding.
    implementation: Option<Address>,
}

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
    abis: Arc<BTreeMap<String, Arc<JsonAbi>>>,
    /// Maps (chain_id, contract_address) -> mapping entry (ABI name + kind + impl link)
    address_mappings: Arc<BTreeMap<(ChainId, Address), AddressMapping>>,
}

impl AbiRegistry {
    /// Creates a new empty ABI registry
    pub fn new() -> Self {
        Self {
            abis: Arc::new(BTreeMap::new()),
            address_mappings: Arc::new(BTreeMap::new()),
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
    pub fn register_abi(
        &mut self,
        name: &str,
        abi_json: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let abi = serde_json::from_str::<JsonAbi>(abi_json)?;
        Arc::get_mut(&mut self.abis)
            .expect("ABI map should be mutable")
            .insert(name.to_string(), Arc::new(abi));
        Ok(())
    }

    /// Maps a contract address to an ABI name for a specific chain.
    ///
    /// The mapping defaults to `AbiKind::Implementation` with no proxy link, which
    /// preserves the prior behaviour for all existing callers (compile-time embedded
    /// ABIs are implementations by nature).
    ///
    /// # Arguments
    /// * `chain_id` - The blockchain chain ID (e.g., 1 for Ethereum Mainnet)
    /// * `address` - The contract address
    /// * `abi_name` - The ABI name (must be previously registered)
    pub fn map_address(&mut self, chain_id: ChainId, address: Address, abi_name: &str) {
        self.map_address_with_type(chain_id, address, abi_name, AbiKind::Implementation, None);
    }

    /// Maps a contract address to an ABI name with an explicit kind and optional
    /// proxy implementation link.
    ///
    /// # Arguments
    /// * `chain_id` - The blockchain chain ID
    /// * `address` - The contract address
    /// * `abi_name` - The ABI name (must be previously registered)
    /// * `abi_kind` - Whether this address is an implementation or a proxy
    /// * `implementation` - For proxies, the implementation address whose ABI
    ///   decodes the calldata (ignored for non-proxy kinds)
    pub fn map_address_with_type(
        &mut self,
        chain_id: ChainId,
        address: Address,
        abi_name: &str,
        abi_kind: AbiKind,
        implementation: Option<Address>,
    ) {
        Arc::get_mut(&mut self.address_mappings)
            .expect("Address mappings should be mutable")
            .insert(
                (chain_id, address),
                AddressMapping {
                    abi_name: abi_name.to_string(),
                    abi_kind,
                    implementation,
                },
            );
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
        let mapping = self.address_mappings.get(&(chain_id, address))?;
        self.abis.get(&mapping.abi_name).cloned()
    }

    /// Gets the declared kind (implementation vs proxy) for a mapped address.
    ///
    /// Returns `None` if the address is not mapped.
    pub fn get_abi_kind(&self, chain_id: ChainId, address: Address) -> Option<AbiKind> {
        self.address_mappings
            .get(&(chain_id, address))
            .map(|m| m.abi_kind)
    }

    /// Resolves a proxy address to its implementation: returns the implementation
    /// address and the ABI registered for it.
    ///
    /// Returns `None` if the address is not a proxy, has no implementation link, or
    /// the linked implementation address has no registered ABI.
    pub fn get_implementation_abi(
        &self,
        chain_id: ChainId,
        proxy: Address,
    ) -> Option<(Address, Arc<JsonAbi>)> {
        let mapping = self.address_mappings.get(&(chain_id, proxy))?;
        if mapping.abi_kind != AbiKind::Proxy {
            return None;
        }
        let implementation = mapping.implementation?;
        let abi = self.get_abi_for_address(chain_id, implementation)?;
        Some((implementation, abi))
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
            .map(|((_, addr), mapping)| (*addr, mapping.abi_name.as_str()))
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
        registry
            .register_abi("TestToken", TEST_ABI)
            .expect("Failed to register ABI");

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
        registry
            .register_abi("TestToken", TEST_ABI)
            .expect("Failed to register ABI");

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
        registry
            .register_abi("TestToken", TEST_ABI)
            .expect("Failed to register ABI");

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
        registry
            .register_abi("TokenA", TEST_ABI)
            .expect("Failed to register");
        registry
            .register_abi("TokenB", TEST_ABI)
            .expect("Failed to register");

        let abis = registry.list_abis();
        assert_eq!(abis.len(), 2);
        assert!(abis.contains(&"TokenA"));
        assert!(abis.contains(&"TokenB"));
    }

    fn addr(hex: &str) -> Address {
        hex.parse::<Address>().expect("valid address")
    }

    #[test]
    fn test_map_address_defaults_to_implementation() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("Token", TEST_ABI).unwrap();
        let a = addr("0x1234567890123456789012345678901234567890");
        registry.map_address(1, a, "Token");

        assert_eq!(registry.get_abi_kind(1, a), Some(AbiKind::Implementation));
        // Implementation entries never resolve as proxies.
        assert!(registry.get_implementation_abi(1, a).is_none());
    }

    #[test]
    fn test_proxy_resolves_implementation_abi() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("Impl", TEST_ABI).unwrap();
        registry.register_abi("Proxy", "[]").unwrap();

        let proxy = addr("0x1111111111111111111111111111111111111111");
        let implementation = addr("0x2222222222222222222222222222222222222222");

        registry.map_address(1, implementation, "Impl");
        registry.map_address_with_type(1, proxy, "Proxy", AbiKind::Proxy, Some(implementation));

        assert_eq!(registry.get_abi_kind(1, proxy), Some(AbiKind::Proxy));
        let (resolved_addr, resolved_abi) = registry
            .get_implementation_abi(1, proxy)
            .expect("proxy should resolve to implementation");
        assert_eq!(resolved_addr, implementation);
        // Resolved ABI is the implementation's (has `transfer`), not the empty proxy ABI.
        assert!(resolved_abi.functions().any(|f| f.name == "transfer"));
    }

    #[test]
    fn test_proxy_without_implementation_link_does_not_resolve() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("Proxy", TEST_ABI).unwrap();
        let proxy = addr("0x1111111111111111111111111111111111111111");
        registry.map_address_with_type(1, proxy, "Proxy", AbiKind::Proxy, None);

        assert_eq!(registry.get_abi_kind(1, proxy), Some(AbiKind::Proxy));
        assert!(registry.get_implementation_abi(1, proxy).is_none());
        // The proxy's own ABI is still directly retrievable for fallback decoding.
        assert!(registry.get_abi_for_address(1, proxy).is_some());
    }

    #[test]
    fn test_proxy_with_unregistered_implementation_does_not_resolve() {
        let mut registry = AbiRegistry::new();
        registry.register_abi("Proxy", "[]").unwrap();
        let proxy = addr("0x1111111111111111111111111111111111111111");
        let implementation = addr("0x2222222222222222222222222222222222222222");
        // Link points at an address that has no registered ABI.
        registry.map_address_with_type(1, proxy, "Proxy", AbiKind::Proxy, Some(implementation));

        assert!(registry.get_implementation_abi(1, proxy).is_none());
    }
}
