use crate::registry::{ContractRegistry, ContractType};
use crate::token_metadata::{ErcStandard, TokenMetadata};
use alloy_primitives::Address;

/// Aave v3 Pool contract type identifier
pub struct AaveV3PoolContract;

impl ContractType for AaveV3PoolContract {
    fn short_type_id() -> &'static str {
        "aave_v3_pool"
    }
}

/// Configuration for Aave v3 protocol contracts
///
/// Source: https://docs.aave.com/developers/deployed-contracts/v3-mainnet
pub struct AaveV3Config;

impl AaveV3Config {
    /// Returns the Aave v3 Pool contract address for the given chain
    pub fn pool_address(chain_id: u64) -> Option<Address> {
        match chain_id {
            // Ethereum Mainnet
            1 => Some(
                "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"
                    .parse()
                    .unwrap(),
            ),
            // Polygon
            137 => Some(
                "0x794a61358D6845594F94dc1DB02A252b5b4814aD"
                    .parse()
                    .unwrap(),
            ),
            // Arbitrum
            42161 => Some(
                "0x794a61358D6845594F94dc1DB02A252b5b4814aD"
                    .parse()
                    .unwrap(),
            ),
            // Optimism
            10 => Some(
                "0x794a61358D6845594F94dc1DB02A252b5b4814aD"
                    .parse()
                    .unwrap(),
            ),
            // Base
            8453 => Some(
                "0xA238Dd80C259a72e81d7e4664a9801593F98d1c5"
                    .parse()
                    .unwrap(),
            ),
            // Avalanche
            43114 => Some(
                "0x794a61358D6845594F94dc1DB02A252b5b4814aD"
                    .parse()
                    .unwrap(),
            ),
            // Gnosis
            100 => Some(
                "0xb50201558B00496A145fE76f7424749556E326D8"
                    .parse()
                    .unwrap(),
            ),
            // BNB Chain
            56 => Some(
                "0x6807dc923806fE8Fd134338EABCA509979a7e0cB"
                    .parse()
                    .unwrap(),
            ),
            _ => None,
        }
    }

    /// Returns the list of chain IDs where Aave v3 is deployed
    pub fn supported_chains() -> &'static [u64] {
        &[
            1,     // Ethereum Mainnet
            137,   // Polygon
            42161, // Arbitrum
            10,    // Optimism
            8453,  // Base
            43114, // Avalanche
            100,   // Gnosis
            56,    // BNB Chain
        ]
    }

    /// Registers Aave v3 protocol contracts in the registry
    pub fn register_contracts(registry: &mut ContractRegistry) {
        for &chain_id in Self::supported_chains() {
            if let Some(pool_address) = Self::pool_address(chain_id) {
                registry
                    .register_contract_typed::<AaveV3PoolContract>(chain_id, vec![pool_address]);
            }
        }

        // Register common tokens used with Aave
        Self::register_common_tokens(registry);
    }

    /// Registers common tokens used in Aave transactions
    ///
    /// This registers tokens like USDC, USDT, DAI, and WETH across multiple chains
    /// so they can be resolved by symbol during transaction visualization.
    pub fn register_common_tokens(registry: &mut ContractRegistry) {
        // Ethereum Mainnet tokens
        registry.register_token(
            1,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            1,
            TokenMetadata {
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xdac17f958d2ee523a2206206994597c13d831ec7".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            1,
            TokenMetadata {
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x6b175474e89094c44da98b954eedeac495271d0f".to_string(),
                decimals: 18,
            },
        );

        registry.register_token(
            1,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".to_string(),
                decimals: 18,
            },
        );

        // Base tokens
        registry.register_token(
            8453,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            8453,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x4200000000000000000000000000000000000006".to_string(),
                decimals: 18,
            },
        );

        // Polygon tokens
        registry.register_token(
            137,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x2791bca1f2de4661ed88a30c99a7a9449aa84174".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            137,
            TokenMetadata {
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xc2132d05d31c914a87c6611c10748aeb04b58e8f".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            137,
            TokenMetadata {
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x8f3cf7ad23cd3cadbd9735aff958023239c6a063".to_string(),
                decimals: 18,
            },
        );

        registry.register_token(
            137,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619".to_string(),
                decimals: 18,
            },
        );

        // Arbitrum tokens
        registry.register_token(
            42161,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            42161,
            TokenMetadata {
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            42161,
            TokenMetadata {
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xda10009cbd5d07dd0cecc66161fc93d7c9000da1".to_string(),
                decimals: 18,
            },
        );

        registry.register_token(
            42161,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x82af49447d8a07e3bd95bd0d56f35241523fbab1".to_string(),
                decimals: 18,
            },
        );

        // Optimism tokens
        registry.register_token(
            10,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x0b2c639c533813f4aa9d7837caf62653d097ff85".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            10,
            TokenMetadata {
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x94b008aa00579c1307b0ef2c499ad98a8ce58e58".to_string(),
                decimals: 6,
            },
        );

        registry.register_token(
            10,
            TokenMetadata {
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xda10009cbd5d07dd0cecc66161fc93d7c9000da1".to_string(),
                decimals: 18,
            },
        );

        registry.register_token(
            10,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "Wrapped Ether".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x4200000000000000000000000000000000000006".to_string(),
                decimals: 18,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_addresses() {
        // Ethereum mainnet
        let eth_pool = AaveV3Config::pool_address(1).unwrap();
        assert_eq!(
            format!("{:?}", eth_pool).to_lowercase(),
            "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2"
        );

        // Base
        let base_pool = AaveV3Config::pool_address(8453).unwrap();
        assert_eq!(
            format!("{:?}", base_pool).to_lowercase(),
            "0xa238dd80c259a72e81d7e4664a9801593f98d1c5"
        );
    }

    #[test]
    fn test_unsupported_chain() {
        assert!(AaveV3Config::pool_address(999999).is_none());
    }

    #[test]
    fn test_supported_chains() {
        let chains = AaveV3Config::supported_chains();
        assert!(chains.contains(&1)); // Ethereum
        assert!(chains.contains(&8453)); // Base
        assert!(chains.len() >= 8);
    }
}
