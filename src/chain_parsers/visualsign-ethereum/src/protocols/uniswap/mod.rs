//! Uniswap protocol implementation
//!
//! This module contains contract visualizers, configuration, and registration
//! logic for the Uniswap decentralized exchange protocol.

pub mod config;
pub mod contracts;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;

pub use config::UniswapConfig;
pub use contracts::{
    Permit2ContractVisualizer, Permit2Visualizer, UniversalRouterContractVisualizer,
    UniversalRouterVisualizer, V4PoolManagerVisualizer,
};

/// Registers all Uniswap protocol contracts and visualizers
///
/// This function:
/// 1. Registers contract addresses in the ContractRegistry for address-to-type lookup
/// 2. Registers visualizers in the EthereumVisualizerRegistryBuilder for transaction visualization
///
/// # Arguments
/// * `contract_reg` - The contract registry to register addresses
/// * `visualizer_reg` - The visualizer registry to register visualizers
pub fn register(
    contract_reg: &mut ContractRegistry,
    visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
) {
    use config::{Permit2Contract, UniswapUniversalRouter};

    // Register Universal Router on each supported chain with correct address
    for &chain_id in UniswapConfig::universal_router_chains() {
        let addr = UniswapConfig::universal_router_address(chain_id)
            .expect("universal_router_chains should only contain valid chains");
        contract_reg.register_contract_typed::<UniswapUniversalRouter>(chain_id, vec![addr]);
    }

    // Register Permit2 (same address on all chains)
    let permit2_address = UniswapConfig::permit2_address();
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<Permit2Contract>(chain_id, vec![permit2_address]);
    }

    // Register well-known addresses used by Uniswap
    UniswapConfig::register_well_known_addresses(contract_reg);

    // Register common tokens (WETH, USDC, USDT, DAI, etc.)
    UniswapConfig::register_common_tokens(contract_reg);

    // Register visualizers
    visualizer_reg.register(Box::new(UniversalRouterContractVisualizer::new()));
    visualizer_reg.register(Box::new(Permit2ContractVisualizer::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::uniswap::config::{UniswapUniversalRouter, networks};
    use crate::registry::ContractType;

    #[test]
    fn test_register_uniswap_contracts() {
        let mut contract_reg = ContractRegistry::new();
        let mut visualizer_reg = EthereumVisualizerRegistryBuilder::new();

        register(&mut contract_reg, &mut visualizer_reg);

        // Verify Universal Router is registered on all supported chains with correct addresses
        for &chain_id in UniswapConfig::universal_router_chains() {
            let expected_addr = UniswapConfig::universal_router_address(chain_id)
                .expect("Chain should have valid address");
            let contract_type = contract_reg
                .get_contract_type(chain_id, expected_addr)
                .unwrap_or_else(|| {
                    panic!("Universal Router should be registered on chain {chain_id}")
                });
            assert_eq!(contract_type, UniswapUniversalRouter::short_type_id());
        }
    }

    #[test]
    fn test_different_addresses_per_chain() {
        // Verify that some chains have different addresses
        let eth_addr =
            UniswapConfig::universal_router_address(networks::ethereum::MAINNET).unwrap();
        let arb_addr =
            UniswapConfig::universal_router_address(networks::arbitrum::MAINNET).unwrap();
        let opt_addr =
            UniswapConfig::universal_router_address(networks::optimism::MAINNET).unwrap();

        // Ethereum and Base share the same address
        let base_addr = UniswapConfig::universal_router_address(networks::base::MAINNET).unwrap();
        assert_eq!(eth_addr, base_addr);

        // But Arbitrum and Optimism have different addresses
        assert_ne!(eth_addr, arb_addr);
        assert_ne!(eth_addr, opt_addr);
        assert_ne!(arb_addr, opt_addr);
    }
}
