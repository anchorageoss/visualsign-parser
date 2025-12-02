use crate::token_metadata::{ChainMetadata, TokenMetadata, parse_network_id};
use alloy_primitives::{Address, utils::format_units};
use std::collections::HashMap;

/// Type alias for chain ID to avoid depending on external chain types
pub type ChainId = u64;

/// Well-known addresses that protocols can register and look up
///
/// This enum provides type safety for well-known contract addresses,
/// preventing typos and enabling compile-time checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnownAddress {
    /// Permit2 contract - universal across all chains
    Permit2,
    /// WETH contract - chain-specific addresses
    Weth,
    /// USDC contract - chain-specific addresses
    Usdc,
}

impl WellKnownAddress {
    /// Returns the string identifier for this address type
    pub fn as_str(&self) -> &'static str {
        match self {
            WellKnownAddress::Permit2 => "permit2",
            WellKnownAddress::Weth => "weth",
            WellKnownAddress::Usdc => "usdc",
        }
    }
}

/// Trait for contract type markers
///
/// Implement this trait on unit structs to create compile-time unique contract type identifiers.
/// The type name is automatically used as the contract type string.
///
/// # Example
/// ```ignore
/// pub struct UniswapUniversalRouter;
/// impl ContractType for UniswapUniversalRouter {}
///
/// // The type_id is automatically "UniswapUniversalRouter"
/// ```
///
/// # Compile-time Uniqueness
/// Because Rust doesn't allow duplicate type names in the same scope, this provides
/// compile-time guarantees that contract types are unique. If someone copies a protocol
/// directory and forgets to rename the type, the code won't compile.
pub trait ContractType: 'static {
    /// Returns the unique identifier for this contract type
    ///
    /// By default, uses the Rust type name. Can be overridden for custom strings.
    fn type_id() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Returns a shortened type ID without module path
    ///
    /// Strips the module path to get just the struct name.
    /// Example: "visualsign_ethereum::protocols::uniswap::UniswapUniversalRouter" -> "UniswapUniversalRouter"
    fn short_type_id() -> &'static str {
        let full_name = Self::type_id();
        full_name.rsplit("::").next().unwrap_or(full_name)
    }
}

/// Registry for managing Ethereum contract types and token metadata
///
/// Maintains two types of mappings:
/// 1. Contract type registry: Maps (chain_id, address) to contract type (e.g., "UniswapV3Router")
/// 2. Token metadata registry: Maps (chain_id, token_address) to token information
///
/// # TODO
/// Extract a ChainRegistry trait that all chains can implement for handling token metadata,
/// contract types, and other chain-specific information. This will allow Solana, Tron, Sui,
/// and other chains to use the same interface pattern.
#[derive(Clone)]
pub struct ContractRegistry {
    /// Maps (chain_id, address) to contract type
    address_to_type: HashMap<(ChainId, Address), String>,
    /// Maps (chain_id, contract_type) to list of addresses
    type_to_addresses: HashMap<(ChainId, String), Vec<Address>>,
    /// Maps (chain_id, token_address) to token metadata
    token_metadata: HashMap<(ChainId, Address), TokenMetadata>,
    /// Maps (well_known_address, optional_chain_id) to address
    /// For chain-specific addresses, use Some(chain_id); for universal addresses, use None
    well_known_addresses: HashMap<(WellKnownAddress, Option<ChainId>), Address>,
}

impl ContractRegistry {
    /// Creates a new empty registry
    pub fn new() -> Self {
        Self {
            address_to_type: HashMap::new(),
            type_to_addresses: HashMap::new(),
            token_metadata: HashMap::new(),
            well_known_addresses: HashMap::new(),
        }
    }

    /// Creates a new registry with default protocols registered
    ///
    /// This is the recommended way to create a ContractRegistry with
    /// built-in support for known protocols like Uniswap, Aave, etc.
    ///
    /// Returns both the ContractRegistry and EthereumVisualizerRegistryBuilder since
    /// protocol registration populates both registries. Discarding either would be wasteful.
    pub fn with_default_protocols() -> (Self, crate::visualizer::EthereumVisualizerRegistryBuilder)
    {
        let mut registry = Self::new();
        let mut visualizer_builder = crate::visualizer::EthereumVisualizerRegistryBuilder::new();
        crate::protocols::register_all(&mut registry, &mut visualizer_builder);
        (registry, visualizer_builder)
    }

    /// Registers a contract type on a specific chain (type-safe version)
    ///
    /// This is the preferred method for registering contracts. It uses the ContractType
    /// trait to ensure compile-time uniqueness of contract type identifiers.
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID (1 for Ethereum, 137 for Polygon, etc.)
    /// * `addresses` - List of contract addresses on this chain
    ///
    /// # Example
    /// ```ignore
    /// pub struct UniswapUniversalRouter;
    /// impl ContractType for UniswapUniversalRouter {}
    ///
    /// registry.register_contract_typed::<UniswapUniversalRouter>(1, vec![address]);
    /// ```
    pub fn register_contract_typed<T: ContractType>(
        &mut self,
        chain_id: ChainId,
        addresses: Vec<Address>,
    ) {
        let contract_type_str = T::short_type_id().to_string();

        for address in &addresses {
            self.address_to_type
                .insert((chain_id, *address), contract_type_str.clone());
        }

        self.type_to_addresses
            .insert((chain_id, contract_type_str), addresses);
    }

    /// Registers a contract type on a specific chain (string version)
    ///
    /// This method is kept for backward compatibility and dynamic registration.
    /// Prefer `register_contract_typed` for compile-time safety.
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID (1 for Ethereum, 137 for Polygon, etc.)
    /// * `contract_type` - The contract type identifier (e.g., "UniswapV3Router", "Aave")
    /// * `addresses` - List of contract addresses on this chain
    pub fn register_contract(
        &mut self,
        chain_id: ChainId,
        contract_type: impl Into<String>,
        addresses: Vec<Address>,
    ) {
        let contract_type_str = contract_type.into();

        for address in &addresses {
            self.address_to_type
                .insert((chain_id, *address), contract_type_str.clone());
        }

        self.type_to_addresses
            .insert((chain_id, contract_type_str), addresses);
    }

    /// Registers token metadata for a specific token
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID
    /// * `metadata` - The TokenMetadata containing all token information
    ///
    /// # Errors
    /// Returns an error if the contract address cannot be parsed as a valid Ethereum address
    pub fn register_token(
        &mut self,
        chain_id: ChainId,
        metadata: TokenMetadata,
    ) -> Result<(), String> {
        let address: Address = metadata
            .contract_address
            .parse()
            .map_err(|_| format!("Invalid contract address: {}", metadata.contract_address))?;
        self.token_metadata.insert((chain_id, address), metadata);
        Ok(())
    }

    /// Gets the contract type for a specific address on a chain
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID
    /// * `address` - The contract address
    ///
    /// # Returns
    /// `Some(contract_type)` if the address is registered, `None` otherwise
    pub fn get_contract_type(&self, chain_id: ChainId, address: Address) -> Option<String> {
        self.address_to_type.get(&(chain_id, address)).cloned()
    }

    /// Gets the symbol for a specific token on a chain
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID
    /// * `token` - The token's contract address
    ///
    /// # Returns
    /// `Some(symbol)` if the token is registered, `None` otherwise
    pub fn get_token_symbol(&self, chain_id: ChainId, token: Address) -> Option<String> {
        self.token_metadata
            .get(&(chain_id, token))
            .map(|m| m.symbol.clone())
    }

    /// Registers a well-known address that exists on all chains at the same address
    ///
    /// # Arguments
    /// * `address_type` - The well-known address type
    /// * `address` - The address (same across all chains)
    ///
    /// # Example
    /// ```ignore
    /// registry.register_universal_address(WellKnownAddress::Permit2, "0x000000000022d473030f116ddee9f6b43ac78ba3".parse().unwrap());
    /// ```
    pub fn register_universal_address(&mut self, address_type: WellKnownAddress, address: Address) {
        self.well_known_addresses
            .insert((address_type, None), address);
    }

    /// Registers a well-known address for a specific chain
    ///
    /// # Arguments
    /// * `address_type` - The well-known address type
    /// * `chain_id` - The specific chain ID
    /// * `address` - The address on that chain
    ///
    /// # Example
    /// ```ignore
    /// registry.register_chain_specific_address(WellKnownAddress::Weth, 1, "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".parse().unwrap());
    /// ```
    pub fn register_chain_specific_address(
        &mut self,
        address_type: WellKnownAddress,
        chain_id: ChainId,
        address: Address,
    ) {
        self.well_known_addresses
            .insert((address_type, Some(chain_id)), address);
    }

    /// Gets a well-known address
    ///
    /// First checks for chain-specific address, then falls back to universal address.
    ///
    /// # Arguments
    /// * `address_type` - The well-known address type
    /// * `chain_id` - The chain ID to look up
    ///
    /// # Returns
    /// `Some(address)` if found, `None` otherwise
    ///
    /// # Example
    /// ```ignore
    /// let permit2_addr = registry.get_well_known_address(WellKnownAddress::Permit2, 1)?; // Universal address
    /// let weth_addr = registry.get_well_known_address(WellKnownAddress::Weth, 1)?;     // Chain-specific address
    /// ```
    pub fn get_well_known_address(
        &self,
        address_type: WellKnownAddress,
        chain_id: ChainId,
    ) -> Option<Address> {
        // Try chain-specific first
        if let Some(addr) = self
            .well_known_addresses
            .get(&(address_type, Some(chain_id)))
        {
            return Some(*addr);
        }
        // Fall back to universal
        self.well_known_addresses
            .get(&(address_type, None))
            .copied()
    }

    /// Formats a raw token amount with the proper number of decimal places
    ///
    /// This method:
    /// 1. Looks up the token metadata for the given address
    /// 2. Uses Alloy's format_units to convert raw amount to decimal representation
    /// 3. Returns (formatted_amount, symbol) tuple
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID
    /// * `token` - The token's contract address
    /// * `raw_amount` - The raw amount in the token's smallest units
    ///
    /// # Returns
    /// `Some((formatted_amount, symbol))` if token is registered and format succeeds
    /// `None` if token is not registered
    ///
    /// # Examples
    /// ```ignore
    /// // USDC with 6 decimals
    /// registry.format_token_amount(1, usdc_addr, 1_500_000);
    /// // Returns: Some(("1.5", "USDC"))
    ///
    /// // WETH with 18 decimals
    /// registry.format_token_amount(1, weth_addr, 1_000_000_000_000_000_000);
    /// // Returns: Some(("1", "WETH"))
    /// ```
    pub fn format_token_amount(
        &self,
        chain_id: ChainId,
        token: Address,
        raw_amount: u128,
    ) -> Option<(String, String)> {
        let metadata = self.token_metadata.get(&(chain_id, token))?;

        // Use Alloy's format_units to format the amount
        let formatted = format_units(raw_amount, metadata.decimals).ok()?;

        Some((formatted, metadata.symbol.clone()))
    }

    /// Loads token metadata from wallet ChainMetadata structure
    ///
    /// This method parses network_id to determine the chain ID and registers
    /// all tokens from the metadata's assets collection.
    ///
    /// # Arguments
    /// * `chain_metadata` - Reference to ChainMetadata containing token information
    ///
    /// # Returns
    /// `Ok(())` on success, `Err(String)` if network_id is unknown or any token registration fails
    pub fn load_chain_metadata(&mut self, chain_metadata: &ChainMetadata) -> Result<(), String> {
        let chain_id = parse_network_id(&chain_metadata.network_id).map_err(|e| e.to_string())?;

        let errors: Vec<String> = chain_metadata
            .assets
            .values()
            .filter_map(|token_metadata| {
                self.register_token(chain_id, token_metadata.clone()).err()
            })
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Default for ContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_metadata::ErcStandard;

    fn usdc_address() -> Address {
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap()
    }

    fn weth_address() -> Address {
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap()
    }

    fn dai_address() -> Address {
        "0x6b175474e89094c44da98b954eedeac495271d0f"
            .parse()
            .unwrap()
    }

    fn create_token_metadata(
        symbol: &str,
        name: &str,
        address: &str,
        decimals: u8,
    ) -> TokenMetadata {
        TokenMetadata {
            symbol: symbol.to_string(),
            name: name.to_string(),
            erc_standard: ErcStandard::Erc20,
            contract_address: address.to_string(),
            decimals,
        }
    }

    #[test]
    fn test_registry_new() {
        let registry = ContractRegistry::new();
        assert_eq!(registry.address_to_type.len(), 0);
        assert_eq!(registry.type_to_addresses.len(), 0);
        assert_eq!(registry.token_metadata.len(), 0);
        assert_eq!(registry.well_known_addresses.len(), 0);
    }

    #[test]
    fn test_register_contract() {
        let mut registry = ContractRegistry::new();
        let addresses = vec![
            "0x68b3465833fb72B5A828cCEEaAF60b9Ab78ad723"
                .parse()
                .unwrap(),
            "0xE592427A0AEce92De3Edee1F18E0157C05861564"
                .parse()
                .unwrap(),
        ];

        registry.register_contract(1, "UniswapV3Router", addresses.clone());

        assert_eq!(registry.address_to_type.len(), 2);
        assert_eq!(registry.type_to_addresses.len(), 1);

        for addr in &addresses {
            assert_eq!(
                registry.get_contract_type(1, *addr),
                Some("UniswapV3Router".to_string())
            );
        }
    }

    #[test]
    fn test_register_token() {
        let mut registry = ContractRegistry::new();
        let usdc = create_token_metadata(
            "USDC",
            "USD Coin",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            6,
        );
        registry.register_token(1, usdc).unwrap();

        assert_eq!(registry.token_metadata.len(), 1);
        assert_eq!(
            registry.get_token_symbol(1, usdc_address()),
            Some("USDC".to_string())
        );
    }

    #[test]
    fn test_format_token_amount_6_decimals() {
        let mut registry = ContractRegistry::new();
        let usdc = create_token_metadata(
            "USDC",
            "USD Coin",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            6,
        );
        registry.register_token(1, usdc).unwrap();

        // Test: 1.5 USDC = 1_500_000 in raw units
        let result = registry.format_token_amount(1, usdc_address(), 1_500_000);
        assert_eq!(result, Some(("1.500000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_format_token_amount_18_decimals() {
        let mut registry = ContractRegistry::new();
        let weth = create_token_metadata(
            "WETH",
            "Wrapped Ether",
            "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
            18,
        );
        registry.register_token(1, weth).unwrap();

        // Test: 1 WETH = 1_000_000_000_000_000_000 in raw units
        let result = registry.format_token_amount(1, weth_address(), 1_000_000_000_000_000_000);
        assert_eq!(
            result,
            Some(("1.000000000000000000".to_string(), "WETH".to_string()))
        );
    }

    #[test]
    fn test_format_token_amount_with_trailing_zeros() {
        let mut registry = ContractRegistry::new();
        let usdc = create_token_metadata(
            "USDC",
            "USD Coin",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            6,
        );
        registry.register_token(1, usdc).unwrap();

        // Test: 1 USDC = 1_000_000 in raw units
        let result = registry.format_token_amount(1, usdc_address(), 1_000_000);
        assert_eq!(result, Some(("1.000000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_format_token_amount_multiple_decimals() {
        let mut registry = ContractRegistry::new();
        let usdc = create_token_metadata(
            "USDC",
            "USD Coin",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            6,
        );
        registry.register_token(1, usdc).unwrap();

        // Test: 12.345678 USDC (should trim to 6 decimals: 12.345678)
        let result = registry.format_token_amount(1, usdc_address(), 12_345_678);
        assert_eq!(result, Some(("12.345678".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_format_token_amount_unknown_token() {
        let registry = ContractRegistry::new();

        // Test: Unknown token returns None
        let result = registry.format_token_amount(1, usdc_address(), 1_000_000);
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_token_amount_zero_amount() {
        let mut registry = ContractRegistry::new();
        let usdc = create_token_metadata(
            "USDC",
            "USD Coin",
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            6,
        );
        registry.register_token(1, usdc).unwrap();

        // Test: 0 USDC
        let result = registry.format_token_amount(1, usdc_address(), 0);
        assert_eq!(result, Some(("0.000000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_load_chain_metadata() {
        let mut registry = ContractRegistry::new();

        let mut assets = HashMap::new();
        assets.insert(
            "USDC".to_string(),
            create_token_metadata(
                "USDC",
                "USD Coin",
                "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                6,
            ),
        );
        assets.insert(
            "DAI".to_string(),
            create_token_metadata(
                "DAI",
                "Dai Stablecoin",
                "0x6b175474e89094c44da98b954eedeac495271d0f",
                18,
            ),
        );

        let metadata = ChainMetadata {
            network_id: "ETHEREUM_MAINNET".to_string(),
            assets,
        };

        registry.load_chain_metadata(&metadata).unwrap();

        assert_eq!(registry.token_metadata.len(), 2);
        assert_eq!(
            registry.get_token_symbol(1, usdc_address()),
            Some("USDC".to_string())
        );
        assert_eq!(
            registry.get_token_symbol(1, dai_address()),
            Some("DAI".to_string())
        );
    }

    #[test]
    fn test_get_contract_type_not_found() {
        let registry = ContractRegistry::new();

        let result = registry.get_contract_type(1, usdc_address());
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_token_symbol_not_found() {
        let registry = ContractRegistry::new();

        let result = registry.get_token_symbol(1, usdc_address());
        assert_eq!(result, None);
    }

    #[test]
    fn test_register_multiple_tokens() {
        let mut registry = ContractRegistry::new();

        registry
            .register_token(
                1,
                create_token_metadata(
                    "USDC",
                    "USD Coin",
                    "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                    6,
                ),
            )
            .unwrap();
        registry
            .register_token(
                1,
                create_token_metadata(
                    "WETH",
                    "Wrapped Ether",
                    "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                    18,
                ),
            )
            .unwrap();
        registry
            .register_token(
                1,
                create_token_metadata(
                    "DAI",
                    "Dai Stablecoin",
                    "0x6b175474e89094c44da98b954eedeac495271d0f",
                    18,
                ),
            )
            .unwrap();

        assert_eq!(registry.token_metadata.len(), 3);

        // Verify each token was registered correctly
        let usdc_result = registry.format_token_amount(1, usdc_address(), 1_500_000);
        assert_eq!(
            usdc_result,
            Some(("1.500000".to_string(), "USDC".to_string()))
        );

        let weth_result =
            registry.format_token_amount(1, weth_address(), 2_000_000_000_000_000_000);
        assert_eq!(
            weth_result,
            Some(("2.000000000000000000".to_string(), "WETH".to_string()))
        );

        let dai_result = registry.format_token_amount(1, dai_address(), 3_500_000_000_000_000_000);
        assert_eq!(
            dai_result,
            Some(("3.500000000000000000".to_string(), "DAI".to_string()))
        );
    }

    #[test]
    fn test_same_token_different_chains() {
        let mut registry = ContractRegistry::new();

        // Register USDC on Ethereum (chain 1)
        registry
            .register_token(
                1,
                create_token_metadata(
                    "USDC",
                    "USD Coin",
                    "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                    6,
                ),
            )
            .unwrap();

        // Register USDC on Polygon (chain 137) with different address
        registry
            .register_token(
                137,
                create_token_metadata(
                    "USDC",
                    "USD Coin",
                    "0x2791bca1f2de4661ed88a30c99a7a9449aa84174",
                    6,
                ),
            )
            .unwrap();

        let eth_result = registry.format_token_amount(1, usdc_address(), 1_000_000);
        assert_eq!(
            eth_result,
            Some(("1.000000".to_string(), "USDC".to_string()))
        );

        let poly_usdc = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174"
            .parse()
            .unwrap();
        let poly_result = registry.format_token_amount(137, poly_usdc, 1_000_000);
        assert_eq!(
            poly_result,
            Some(("1.000000".to_string(), "USDC".to_string()))
        );
    }

    #[test]
    fn test_load_chain_metadata_with_invalid_addresses() {
        let mut registry = ContractRegistry::new();

        let mut assets = HashMap::new();
        // Valid token
        assets.insert(
            "USDC".to_string(),
            create_token_metadata(
                "USDC",
                "USD Coin",
                "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                6,
            ),
        );
        // Invalid address - too short
        assets.insert(
            "BAD1".to_string(),
            TokenMetadata {
                symbol: "BAD1".to_string(),
                name: "Bad Token 1".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xinvalid".to_string(),
                decimals: 18,
            },
        );
        // Invalid address - not hex
        assets.insert(
            "BAD2".to_string(),
            TokenMetadata {
                symbol: "BAD2".to_string(),
                name: "Bad Token 2".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "not_an_address".to_string(),
                decimals: 18,
            },
        );

        let metadata = ChainMetadata {
            network_id: "ETHEREUM_MAINNET".to_string(),
            assets,
        };

        let result = registry.load_chain_metadata(&metadata);
        assert!(result.is_err());

        let err = result.unwrap_err();
        // Verify both invalid addresses are mentioned in the error
        assert!(err.contains("0xinvalid"), "Error should mention 0xinvalid");
        assert!(
            err.contains("not_an_address"),
            "Error should mention not_an_address"
        );

        // Valid token should still be registered
        assert_eq!(registry.token_metadata.len(), 1);
        assert_eq!(
            registry.get_token_symbol(1, usdc_address()),
            Some("USDC".to_string())
        );
    }

    #[test]
    fn test_load_chain_metadata_unknown_network() {
        let mut registry = ContractRegistry::new();

        let metadata = ChainMetadata {
            network_id: "UNKNOWN_NETWORK".to_string(),
            assets: HashMap::new(),
        };

        let result = registry.load_chain_metadata(&metadata);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("UNKNOWN_NETWORK"));
    }

    #[test]
    fn test_register_universal_address() {
        let mut registry = ContractRegistry::new();
        let permit2_addr: Address = "0x000000000022d473030f116ddee9f6b43ac78ba3"
            .parse()
            .unwrap();

        registry.register_universal_address(WellKnownAddress::Permit2, permit2_addr);

        // Should work on any chain
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Permit2, 1),
            Some(permit2_addr)
        );
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Permit2, 137),
            Some(permit2_addr)
        );
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Permit2, 42161),
            Some(permit2_addr)
        );
    }

    #[test]
    fn test_register_chain_specific_address() {
        let mut registry = ContractRegistry::new();
        let weth_mainnet: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let weth_polygon: Address = "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619"
            .parse()
            .unwrap();

        registry.register_chain_specific_address(WellKnownAddress::Weth, 1, weth_mainnet);
        registry.register_chain_specific_address(WellKnownAddress::Weth, 137, weth_polygon);

        // Should return correct address for each chain
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 1),
            Some(weth_mainnet)
        );
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 137),
            Some(weth_polygon)
        );

        // Should return None for chains without this address
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 999),
            None
        );
    }

    #[test]
    fn test_well_known_address_priority() {
        let mut registry = ContractRegistry::new();
        let universal_addr: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let chain_specific_addr: Address = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();

        // Register both universal and chain-specific for the same address type
        registry.register_universal_address(WellKnownAddress::Weth, universal_addr);
        registry.register_chain_specific_address(WellKnownAddress::Weth, 1, chain_specific_addr);

        // Chain-specific should take priority over universal
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 1),
            Some(chain_specific_addr)
        );

        // Other chains should fall back to universal
        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 137),
            Some(universal_addr)
        );
    }

    #[test]
    fn test_get_well_known_address_not_found() {
        let registry = ContractRegistry::new();

        assert_eq!(
            registry.get_well_known_address(WellKnownAddress::Weth, 1),
            None
        );
    }

    #[test]
    fn test_all_well_known_addresses_resolvable() {
        // Create registry with default protocols (includes well-known addresses)
        let (registry, _) = ContractRegistry::with_default_protocols();

        // Test chains that should have comprehensive coverage
        let test_chains = [
            1u64,     // Ethereum Mainnet
            137u64,   // Polygon
            42161u64, // Arbitrum
            10u64,    // Optimism
            8453u64,  // Base
        ];

        // Test each variant of WellKnownAddress
        let well_known_variants = [
            WellKnownAddress::Permit2,
            WellKnownAddress::Weth,
            WellKnownAddress::Usdc,
        ];

        for &variant in &well_known_variants {
            let variant_name = variant.as_str();

            match variant {
                WellKnownAddress::Permit2 => {
                    // Permit2 should be universal - resolvable on any chain
                    for &chain_id in &test_chains {
                        let addr = registry
                            .get_well_known_address(variant, chain_id)
                            .unwrap_or_else(|| {
                                panic!("{variant_name} should be resolvable on chain {chain_id}",)
                            });

                        // Should be the same address on all chains (universal)
                        let expected_permit2: Address =
                            "0x000000000022d473030f116ddee9f6b43ac78ba3"
                                .parse()
                                .unwrap();
                        assert_eq!(
                            addr, expected_permit2,
                            "Permit2 address should be {expected_permit2} on chain {chain_id}",
                        );
                    }
                }
                WellKnownAddress::Weth => {
                    // WETH should be chain-specific - different addresses per chain
                    let mut addresses = std::collections::HashSet::new();

                    for &chain_id in &test_chains {
                        let addr = registry
                            .get_well_known_address(variant, chain_id)
                            .unwrap_or_else(|| {
                                panic!("{variant_name} should be resolvable on chain {chain_id}",)
                            });
                        addresses.insert(addr);
                    }

                    // Should have multiple different addresses (chain-specific)
                    assert!(
                        addresses.len() > 1,
                        "WETH should have different addresses on different chains, but got: {addresses:?}",
                    );
                }
                WellKnownAddress::Usdc => {
                    // USDC is chain-specific but may not be registered yet
                    // For now, we just verify the enum variant exists
                    // Future implementations can register USDC addresses

                    // Check that the variant has a string representation
                    assert_eq!(variant.as_str(), "usdc");

                    // Note: We don't require USDC to be registered yet,
                    // but the enum should be ready for future use
                }
            }
        }

        // Verify that all enum variants were tested
        assert_eq!(
            well_known_variants.len(),
            3,
            "Update this test when adding new WellKnownAddress variants"
        );
    }
}
