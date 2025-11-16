//! Uniswap protocol configuration
//!
//! Contains contract addresses, chain deployments, and protocol metadata.
//!
//! # Deployment Addresses
//!
//! Official Uniswap Universal Router deployments are documented at:
//! <https://github.com/Uniswap/universal-router/tree/main/deploy-addresses>
//!
//! Each network has a JSON file (e.g., mainnet.json, optimism.json) containing:
//! - `UniversalRouterV1`: Legacy V1 router
//! - `UniversalRouterV1_2_V2Support`: V1.2 with V2 support (0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD)
//! - `UniversalRouterV2`: Latest V2 router
//!
//! Currently, only V1.2 is implemented. Future versions should be added as separate
//! contract type markers below.

use alloy_primitives::Address;
use crate::registry::{ContractRegistry, ContractType};
use crate::token_metadata::{TokenMetadata, ErcStandard};

/// Contract type marker for Uniswap Universal Router V1.2
///
/// This is the V1.2 router with V2 support, deployed at 0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD
/// across multiple chains (Mainnet, Optimism, Polygon, Base, Arbitrum).
///
/// Reference: <https://github.com/Uniswap/universal-router/tree/main/deploy-addresses>
#[derive(Debug, Clone, Copy)]
pub struct UniswapUniversalRouter;

impl ContractType for UniswapUniversalRouter {}

// TODO: Add contract type markers for other Universal Router versions
//
// /// Universal Router V1 (legacy) - 0xEf1c6E67703c7BD7107eed8303Fbe6EC2554BF6B
// #[derive(Debug, Clone, Copy)]
// pub struct UniswapUniversalRouterV1;
// impl ContractType for UniswapUniversalRouterV1 {}
//
// /// Universal Router V2 (latest) - 0x66a9893cc07d91d95644aedd05d03f95e1dba8af
// #[derive(Debug, Clone, Copy)]
// pub struct UniswapUniversalRouterV2;
// impl ContractType for UniswapUniversalRouterV2 {}

// TODO: Add V4 PoolManager contract type
//
// V4 requires the PoolManager contract for liquidity pool management.
// Deployments: <https://docs.uniswap.org/contracts/v4/deployments>
//
// /// Uniswap V4 PoolManager
// #[derive(Debug, Clone, Copy)]
// pub struct UniswapV4PoolManager;
// impl ContractType for UniswapV4PoolManager {}

/// Uniswap protocol configuration
pub struct UniswapConfig;

impl UniswapConfig {
    /// Returns the Universal Router V1.2 address
    ///
    /// This is the `UniversalRouterV1_2_V2Support` address from Uniswap's deployment files.
    /// It is deployed at the same address across multiple chains.
    ///
    /// Source: <https://github.com/Uniswap/universal-router/tree/main/deploy-addresses>
    pub fn universal_router_address() -> Address {
        "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD"
            .parse()
            .expect("Valid Universal Router address")
    }

    /// Returns the chain IDs where Universal Router V1.2 is deployed
    ///
    /// Supported chains:
    /// - 1 = Ethereum Mainnet
    /// - 10 = Optimism
    /// - 137 = Polygon
    /// - 8453 = Base
    /// - 42161 = Arbitrum One
    ///
    /// Note: Other chains may be supported. See deployment files:
    /// <https://github.com/Uniswap/universal-router/tree/main/deploy-addresses>
    pub fn universal_router_chains() -> &'static [u64] {
        &[1, 10, 137, 8453, 42161]
    }

    // TODO: Add methods for other Universal Router versions
    //
    // Source: https://github.com/Uniswap/universal-router/tree/main/deploy-addresses
    //
    // pub fn universal_router_v1_address() -> Address {
    //     "0xEf1c6E67703c7BD7107eed8303Fbe6EC2554BF6B".parse().unwrap()
    // }
    // pub fn universal_router_v1_chains() -> &'static [u64] { ... }
    //
    // pub fn universal_router_v2_address() -> Address {
    //     "0x66a9893cc07d91d95644aedd05d03f95e1dba8af".parse().unwrap()
    // }
    // pub fn universal_router_v2_chains() -> &'static [u64] { ... }

    // TODO: Add methods for V4 PoolManager
    //
    // Source: https://docs.uniswap.org/contracts/v4/deployments
    //
    // pub fn v4_pool_manager_address() -> Address { ... }
    // pub fn v4_pool_manager_chains() -> &'static [u64] { ... }

    /// Registers common tokens used in Uniswap transactions
    ///
    /// This registers tokens like WETH across multiple chains so they can be
    /// resolved by symbol during transaction visualization.
    pub fn register_common_tokens(registry: &mut ContractRegistry) {
        // WETH on Ethereum Mainnet
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

        // WETH on Optimism
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

        // WETH on Polygon
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

        // WETH on Base
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

        // WETH on Arbitrum
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

        // Add common tokens on Ethereum Mainnet
        // USDC
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

        // USDT
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

        // DAI
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

        // SETH (Sonne Ethereum - or other SETH variant)
        registry.register_token(
            1,
            TokenMetadata {
                symbol: "SETH".to_string(),
                name: "SETH".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xe71bdfe1df69284f00ee185cf0d95d0c7680c0d4".to_string(),
                decimals: 18,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_universal_router_address() {
        let expected: Address = "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD"
            .parse()
            .unwrap();
        assert_eq!(UniswapConfig::universal_router_address(), expected);
    }

    #[test]
    fn test_universal_router_chains() {
        let chains = UniswapConfig::universal_router_chains();
        assert_eq!(chains, &[1, 10, 137, 8453, 42161]);
    }

    #[test]
    fn test_contract_type_id() {
        let type_id = UniswapUniversalRouter::short_type_id();
        assert_eq!(type_id, "UniswapUniversalRouter");
    }
}
