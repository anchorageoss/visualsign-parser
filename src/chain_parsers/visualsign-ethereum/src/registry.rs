use alloy_primitives::{Address, utils::format_units};
use std::collections::HashMap;

/// Type alias for chain ID to avoid depending on external chain types
pub type ChainId = u64;

/// Metadata for an ERC-20 token
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMetadata {
    /// The token's symbol (e.g., "USDC", "WETH")
    pub symbol: String,
    /// The token's decimal places (e.g., 6 for USDC, 18 for WETH)
    pub decimals: u8,
    /// The token's full name (e.g., "USD Coin")
    pub name: String,
}

/// Registry for managing Ethereum contract types and token metadata
///
/// Maintains two types of mappings:
/// 1. Contract type registry: Maps (chain_id, address) to contract type (e.g., "UniswapV3Router")
/// 2. Token metadata registry: Maps (chain_id, token_address) to token information
pub struct ContractRegistry {
    /// Maps (chain_id, address) to contract type
    address_to_type: HashMap<(ChainId, Address), String>,
    /// Maps (chain_id, contract_type) to list of addresses
    type_to_addresses: HashMap<(ChainId, String), Vec<Address>>,
    /// Maps (chain_id, token_address) to token metadata
    token_metadata: HashMap<(ChainId, Address), TokenMetadata>,
}

impl ContractRegistry {
    /// Creates a new empty registry
    pub fn new() -> Self {
        Self {
            address_to_type: HashMap::new(),
            type_to_addresses: HashMap::new(),
            token_metadata: HashMap::new(),
        }
    }

    /// Registers a contract type on a specific chain
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
    /// * `address` - The token's contract address
    /// * `symbol` - The token's symbol (e.g., "USDC")
    /// * `decimals` - The token's decimal places
    /// * `name` - The token's full name
    pub fn register_token(
        &mut self,
        chain_id: ChainId,
        address: Address,
        symbol: impl Into<String>,
        decimals: u8,
        name: impl Into<String>,
    ) {
        let metadata = TokenMetadata {
            symbol: symbol.into(),
            decimals,
            name: name.into(),
        };

        self.token_metadata.insert((chain_id, address), metadata);
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

    /// Loads token metadata from a ChainMetadata structure
    ///
    /// This method parses network_id to determine the chain ID and registers
    /// all tokens from the metadata's assets collection.
    ///
    /// # Arguments
    /// * `chain_metadata` - Reference to ChainMetadata containing token information
    pub fn load_chain_metadata(&mut self, chain_metadata: &ChainMetadata) {
        let chain_id = chain_metadata.network_id;

        for (token_address, asset_info) in &chain_metadata.assets {
            self.register_token(
                chain_id,
                *token_address,
                asset_info.symbol.clone(),
                asset_info.decimals,
                asset_info.name.clone(),
            );
        }
    }
}

impl Default for ContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// ChainMetadata structure representing network and token information
///
/// This is typically loaded from wallet metadata and contains all the information
/// needed to properly format and display transaction details.
#[derive(Debug, Clone)]
pub struct ChainMetadata {
    /// Network ID corresponding to chain ID (1 for Ethereum, 137 for Polygon, etc.)
    pub network_id: ChainId,
    /// Map of token addresses to their metadata
    pub assets: HashMap<Address, AssetInfo>,
}

/// Information about a token asset
#[derive(Debug, Clone)]
pub struct AssetInfo {
    /// Token symbol (e.g., "USDC")
    pub symbol: String,
    /// Token decimal places
    pub decimals: u8,
    /// Token full name
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_registry_new() {
        let registry = ContractRegistry::new();
        assert_eq!(registry.address_to_type.len(), 0);
        assert_eq!(registry.type_to_addresses.len(), 0);
        assert_eq!(registry.token_metadata.len(), 0);
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
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");

        assert_eq!(registry.token_metadata.len(), 1);
        assert_eq!(
            registry.get_token_symbol(1, usdc_address()),
            Some("USDC".to_string())
        );
    }

    #[test]
    fn test_format_token_amount_6_decimals() {
        let mut registry = ContractRegistry::new();
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");

        // Test: 1.5 USDC = 1_500_000 in raw units
        let result = registry.format_token_amount(1, usdc_address(), 1_500_000);
        assert_eq!(result, Some(("1.500000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_format_token_amount_18_decimals() {
        let mut registry = ContractRegistry::new();
        registry.register_token(1, weth_address(), "WETH", 18, "Wrapped Ether");

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
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");

        // Test: 1 USDC = 1_000_000 in raw units
        let result = registry.format_token_amount(1, usdc_address(), 1_000_000);
        assert_eq!(result, Some(("1.000000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_format_token_amount_multiple_decimals() {
        let mut registry = ContractRegistry::new();
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");

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
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");

        // Test: 0 USDC
        let result = registry.format_token_amount(1, usdc_address(), 0);
        assert_eq!(result, Some(("0.000000".to_string(), "USDC".to_string())));
    }

    #[test]
    fn test_load_chain_metadata() {
        let mut registry = ContractRegistry::new();

        let mut assets = HashMap::new();
        assets.insert(
            usdc_address(),
            AssetInfo {
                symbol: "USDC".to_string(),
                decimals: 6,
                name: "USD Coin".to_string(),
            },
        );
        assets.insert(
            dai_address(),
            AssetInfo {
                symbol: "DAI".to_string(),
                decimals: 18,
                name: "Dai Stablecoin".to_string(),
            },
        );

        let metadata = ChainMetadata {
            network_id: 1,
            assets,
        };

        registry.load_chain_metadata(&metadata);

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

        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");
        registry.register_token(1, weth_address(), "WETH", 18, "Wrapped Ether");
        registry.register_token(1, dai_address(), "DAI", 18, "Dai Stablecoin");

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

        // Register USDC on Ethereum (chain 1) and Polygon (chain 137)
        registry.register_token(1, usdc_address(), "USDC", 6, "USD Coin");
        registry.register_token(
            137,
            "0x2791bca1f2de4661ed88a30c99a7a9449aa84174"
                .parse()
                .unwrap(),
            "USDC",
            6,
            "USD Coin",
        );

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
}
