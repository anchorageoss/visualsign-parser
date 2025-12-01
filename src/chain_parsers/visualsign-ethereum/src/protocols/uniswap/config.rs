//! Uniswap protocol configuration
//!
//! Contains contract addresses, chain deployments, and protocol metadata.
//!
//! # Deployment Addresses
//!
//! Official Uniswap Universal Router deployments are documented at:
//! <https://github.com/Uniswap/universal-router/tree/67553d8b067249dd7841d9d1b0eb2997b19d4bf9/deploy-addresses>
//!
//! Each network has a JSON file (e.g., mainnet.json, optimism.json) containing:
//! - `UniversalRouterV1`: Legacy V1 router
//! - `UniversalRouterV1_2_V2Support`: V1.2 with V2 support
//! - `UniversalRouterV2`: Latest V2 router
//!
//! Currently, only V1.2 is implemented. Future versions should be added as separate
//! contract type markers below.

use crate::registry::{ContractRegistry, ContractType};
use crate::token_metadata::{ErcStandard, TokenMetadata};
use alloy_primitives::Address;

/// Re-export chain ID constants from crate::networks::id
///
/// This provides access to chain constants like `networks::ethereum::MAINNET`
/// for use in Uniswap configuration.
///
/// Note: Not all networks in `crate::networks::id` have Universal Router V1.2 deployments.
/// See `UniswapConfig::universal_router_chains()` for the list of supported networks.
pub use crate::networks::id as networks;

/// Error type for Uniswap configuration operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniswapConfigError {
    /// Chain ID is not supported for Universal Router V1.2
    UnsupportedChain(u64),
    /// Address string failed to parse (should never happen with hardcoded addresses)
    InvalidAddress(String),
}

impl std::fmt::Display for UniswapConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UniswapConfigError::UnsupportedChain(chain_id) => {
                write!(
                    f,
                    "Unsupported chain ID for Universal Router V1.2: {chain_id}"
                )
            }
            UniswapConfigError::InvalidAddress(addr) => {
                write!(f, "Invalid address: {addr}")
            }
        }
    }
}

impl std::error::Error for UniswapConfigError {}

/// Contract type marker for Uniswap Universal Router V1.2
///
/// This is the V1.2 router with V2 support. Addresses vary by chain.
///
/// Reference: <https://github.com/Uniswap/universal-router/tree/67553d8b067249dd7841d9d1b0eb2997b19d4bf9/deploy-addresses>
#[derive(Debug, Clone, Copy)]
pub struct UniswapUniversalRouter;

impl ContractType for UniswapUniversalRouter {}

/// Contract type marker for Permit2
///
/// Permit2 is a token approval contract that unifies the approval experience across all applications.
/// It is deployed at the same address (0x000000000022D473030F116dDEE9F6B43aC78BA3) on all chains.
///
/// Reference: <https://github.com/Uniswap/permit2>
#[derive(Debug, Clone, Copy)]
pub struct Permit2Contract;

impl ContractType for Permit2Contract {}

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
    /// Returns the Universal Router V1.2 address for a specific chain
    ///
    /// Source: <https://github.com/Uniswap/universal-router/tree/67553d8b067249dd7841d9d1b0eb2997b19d4bf9/deploy-addresses>
    pub fn universal_router_address(chain_id: u64) -> Result<Address, UniswapConfigError> {
        let addr_str = match chain_id {
            // Mainnets
            networks::ethereum::MAINNET => "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
            networks::optimism::MAINNET => "0xCb1355ff08Ab38bBCE60111F1bb2B784bE25D7e8",
            networks::bsc::MAINNET => "0x4Dae2f939ACf50408e13d58534Ff8c2776d45265",
            networks::polygon::MAINNET => "0xec7BE89e9d109e7e3Fec59c222CF297125FEFda2",
            networks::worldchain::MAINNET => "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
            networks::base::MAINNET => "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
            networks::arbitrum::MAINNET => "0x5E325eDA8064b456f4781070C0738d849c824258",
            networks::celo::MAINNET => "0x643770e279d5d0733f21d6dc03a8efbabf3255b4",
            networks::avalanche::MAINNET => "0x4Dae2f939ACf50408e13d58534Ff8c2776d45265",
            networks::blast::MAINNET => "0x643770E279d5D0733F21d6DC03A8efbABf3255B4",
            // Testnets
            networks::ethereum::SEPOLIA => "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
            _ => return Err(UniswapConfigError::UnsupportedChain(chain_id)),
        };
        addr_str
            .parse()
            .map_err(|_| UniswapConfigError::InvalidAddress(addr_str.to_string()))
    }

    /// Returns the chain IDs where Universal Router V1.2 is deployed
    ///
    /// Source: <https://github.com/Uniswap/universal-router/tree/67553d8b067249dd7841d9d1b0eb2997b19d4bf9/deploy-addresses>
    pub fn universal_router_chains() -> &'static [u64] {
        &[
            // Mainnets
            networks::ethereum::MAINNET,
            networks::optimism::MAINNET,
            networks::bsc::MAINNET,
            networks::polygon::MAINNET,
            networks::worldchain::MAINNET,
            networks::base::MAINNET,
            networks::arbitrum::MAINNET,
            networks::celo::MAINNET,
            networks::avalanche::MAINNET,
            networks::blast::MAINNET,
            // Testnets
            networks::ethereum::SEPOLIA,
        ]
    }

    /// Returns the Permit2 contract address
    ///
    /// Permit2 is deployed at the same address across all chains.
    /// This method provides backward compatibility - prefer using the registry's
    /// get_well_known_address("permit2", chain_id) method.
    ///
    /// Source: <https://github.com/Uniswap/permit2>
    pub fn permit2_address() -> Address {
        "0x000000000022d473030f116ddee9f6b43ac78ba3"
            .parse()
            .expect("Valid PERMIT2 address")
    }

    /// Registers well-known addresses used by Uniswap protocols
    ///
    /// This should be called during registry initialization to populate
    /// well-known addresses that Uniswap protocols depend on.
    pub fn register_well_known_addresses(registry: &mut ContractRegistry) {
        use crate::registry::WellKnownAddress;

        // Permit2 is universal across all chains
        registry.register_universal_address(
            WellKnownAddress::Permit2,
            "0x000000000022d473030f116ddee9f6b43ac78ba3"
                .parse()
                .expect("Valid PERMIT2 address"),
        );

        // WETH addresses are chain-specific
        let weth_addresses = [
            (
                networks::ethereum::MAINNET,
                "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
            ),
            (
                networks::optimism::MAINNET,
                "0x4200000000000000000000000000000000000006",
            ),
            (
                networks::polygon::MAINNET,
                "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619",
            ),
            (
                networks::base::MAINNET,
                "0x4200000000000000000000000000000000000006",
            ),
            (
                networks::arbitrum::MAINNET,
                "0x82af49447d8a07e3bd95bd0d56f35241523fbab1",
            ),
        ];

        for (chain_id, address_str) in weth_addresses {
            registry.register_chain_specific_address(
                WellKnownAddress::Weth,
                chain_id,
                address_str.parse().expect("Valid WETH address"),
            );
        }
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

    /// Returns the WETH address for a given chain
    ///
    /// WETH (Wrapped ETH) addresses vary by chain. This method returns the canonical
    /// WETH address for supported chains.
    pub fn weth_address(chain_id: u64) -> Option<Address> {
        let addr_str = match chain_id {
            networks::ethereum::MAINNET => "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
            networks::optimism::MAINNET => "0x4200000000000000000000000000000000000006",
            networks::polygon::MAINNET => "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619",
            networks::base::MAINNET => "0x4200000000000000000000000000000000000006",
            networks::arbitrum::MAINNET => "0x82af49447d8a07e3bd95bd0d56f35241523fbab1",
            _ => return None,
        };
        addr_str.parse().ok()
    }

    /// Registers common tokens used in Uniswap transactions
    ///
    /// This registers tokens like WETH across multiple chains so they can be
    /// resolved by symbol during transaction visualization.
    pub fn register_common_tokens(registry: &mut ContractRegistry) {
        // WETH on Ethereum Mainnet (WETH9 contract)
        let _ = registry.register_token(
            networks::ethereum::MAINNET,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "WETH9".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".to_string(),
                decimals: 18,
            },
        );

        // WETH on Optimism
        let _ = registry.register_token(
            networks::optimism::MAINNET,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "WETH9".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x4200000000000000000000000000000000000006".to_string(),
                decimals: 18,
            },
        );

        // WETH on Polygon
        let _ = registry.register_token(
            networks::polygon::MAINNET,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "WETH9".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619".to_string(),
                decimals: 18,
            },
        );

        // WETH on Base
        let _ = registry.register_token(
            networks::base::MAINNET,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "WETH9".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x4200000000000000000000000000000000000006".to_string(),
                decimals: 18,
            },
        );

        // WETH on Arbitrum
        let _ = registry.register_token(
            networks::arbitrum::MAINNET,
            TokenMetadata {
                symbol: "WETH".to_string(),
                name: "WETH9".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x82af49447d8a07e3bd95bd0d56f35241523fbab1".to_string(),
                decimals: 18,
            },
        );

        // Add common tokens on Ethereum Mainnet
        // USDC
        let _ = registry.register_token(
            networks::ethereum::MAINNET,
            TokenMetadata {
                symbol: "USDC".to_string(),
                name: "USD Coin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
                decimals: 6,
            },
        );

        // USDT
        let _ = registry.register_token(
            networks::ethereum::MAINNET,
            TokenMetadata {
                symbol: "USDT".to_string(),
                name: "Tether USD".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0xdac17f958d2ee523a2206206994597c13d831ec7".to_string(),
                decimals: 6,
            },
        );

        // DAI
        let _ = registry.register_token(
            networks::ethereum::MAINNET,
            TokenMetadata {
                symbol: "DAI".to_string(),
                name: "Dai Stablecoin".to_string(),
                erc_standard: ErcStandard::Erc20,
                contract_address: "0x6b175474e89094c44da98b954eedeac495271d0f".to_string(),
                decimals: 18,
            },
        );

        // SETH (Sonne Ethereum - or other SETH variant)
        let _ = registry.register_token(
            networks::ethereum::MAINNET,
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
    fn test_universal_router_address_ethereum() {
        let expected: Address = "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD"
            .parse()
            .unwrap();
        assert_eq!(
            UniswapConfig::universal_router_address(networks::ethereum::MAINNET).unwrap(),
            expected
        );
    }

    #[test]
    fn test_universal_router_address_arbitrum() {
        let expected: Address = "0x5E325eDA8064b456f4781070C0738d849c824258"
            .parse()
            .unwrap();
        assert_eq!(
            UniswapConfig::universal_router_address(networks::arbitrum::MAINNET).unwrap(),
            expected
        );
    }

    #[test]
    fn test_universal_router_address_optimism() {
        let expected: Address = "0xCb1355ff08Ab38bBCE60111F1bb2B784bE25D7e8"
            .parse()
            .unwrap();
        assert_eq!(
            UniswapConfig::universal_router_address(networks::optimism::MAINNET).unwrap(),
            expected
        );
    }

    #[test]
    fn test_universal_router_address_unsupported_chain() {
        let result = UniswapConfig::universal_router_address(999999);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            UniswapConfigError::UnsupportedChain(999999)
        );
    }

    #[test]
    fn test_universal_router_chains() {
        let chains = UniswapConfig::universal_router_chains();
        assert!(chains.contains(&networks::ethereum::MAINNET));
        assert!(chains.contains(&networks::optimism::MAINNET));
        assert!(chains.contains(&networks::arbitrum::MAINNET));
        assert!(chains.contains(&networks::base::MAINNET));
        assert!(chains.contains(&networks::polygon::MAINNET));
        assert!(chains.contains(&networks::ethereum::SEPOLIA)); // testnet
    }

    #[test]
    fn test_contract_type_id() {
        let type_id = UniswapUniversalRouter::short_type_id();
        assert_eq!(type_id, "UniswapUniversalRouter");
    }

    #[test]
    fn test_all_chains_have_valid_addresses() {
        for &chain_id in UniswapConfig::universal_router_chains() {
            let result = UniswapConfig::universal_router_address(chain_id);
            assert!(
                result.is_ok(),
                "Chain {chain_id} should have a valid address"
            );
        }
    }
}
