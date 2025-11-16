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
use crate::registry::ContractType;

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
