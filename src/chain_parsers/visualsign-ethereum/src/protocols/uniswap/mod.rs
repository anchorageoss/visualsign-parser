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

    let ur_address = UniswapConfig::universal_router_address();

    // Register Universal Router on all supported chains
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<UniswapUniversalRouter>(
            chain_id,
            vec![ur_address],
        );
    }

    // Register Permit2 (same address on all chains)
    let permit2_address = UniswapConfig::permit2_address();
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<Permit2Contract>(
            chain_id,
            vec![permit2_address],
        );
    }

    // Register common tokens (WETH, USDC, USDT, DAI, etc.)
    UniswapConfig::register_common_tokens(contract_reg);

    // Register visualizers
    visualizer_reg.register(Box::new(UniversalRouterContractVisualizer::new()));
    visualizer_reg.register(Box::new(Permit2ContractVisualizer::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use crate::protocols::uniswap::config::UniswapUniversalRouter;
    use crate::registry::ContractType;

    #[test]
    fn test_register_uniswap_contracts() {
        let mut contract_reg = ContractRegistry::new();
        let mut visualizer_reg = EthereumVisualizerRegistryBuilder::new();

        register(&mut contract_reg, &mut visualizer_reg);

        let universal_router_address: Address = "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD"
            .parse()
            .unwrap();

        // Verify Universal Router is registered on all supported chains
        for chain_id in [1, 10, 137, 8453, 42161] {
            let contract_type = contract_reg
                .get_contract_type(chain_id, universal_router_address)
                .expect(&format!(
                    "Universal Router should be registered on chain {}",
                    chain_id
                ));
            assert_eq!(contract_type, UniswapUniversalRouter::short_type_id());
        }
    }
}
