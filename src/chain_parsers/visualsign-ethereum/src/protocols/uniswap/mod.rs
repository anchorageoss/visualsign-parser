//! Uniswap protocol implementation
//!
//! This module contains contract visualizers, configuration, and registration
//! logic for the Uniswap decentralized exchange protocol.

pub mod config;
pub mod contracts;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;
use crate::registry::ContractType;

pub use config::UniswapConfig;
pub use contracts::{
    Permit2ContractVisualizer, Permit2Visualizer, UniversalRouterContractVisualizer,
    UniversalRouterVisualizer, V4PoolManagerVisualizer,
};

// Wrapper for V4PoolManagerVisualizer to implement ContractVisualizer
pub struct V4PoolManagerContractVisualizer {
    inner: V4PoolManagerVisualizer,
}

impl V4PoolManagerContractVisualizer {
    pub fn new() -> Self {
        Self {
            inner: V4PoolManagerVisualizer,
        }
    }
}

impl Default for V4PoolManagerContractVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::visualizer::ContractVisualizer for V4PoolManagerContractVisualizer {
    fn contract_type(&self) -> &str {
        crate::protocols::uniswap::config::UniswapV4PoolManager::short_type_id()
    }

    fn visualize(
        &self,
        context: &crate::context::VisualizerContext,
    ) -> Result<Option<Vec<visualsign::AnnotatedPayloadField>>, visualsign::vsptrait::VisualSignError>
    {
        // Workaround: Create a fresh registry instance since context.registry is a trait object
        // that doesn't support symbol lookups yet.
        // TODO: Fix this when ContractRegistry architecture is refactored (see lib.rs)
        let registry = crate::registry::ContractRegistry::with_default_protocols();

        // For V4 we might not need the registry yet, but passing it is good practice
        if let Some(field) = self.inner.visualize_tx_commands(
            &context.calldata,
            context.chain_id,
            Some(&registry),
        ) {
            let annotated = visualsign::AnnotatedPayloadField {
                signable_payload_field: field,
                static_annotation: None,
                dynamic_annotation: None,
            };

            Ok(Some(vec![annotated]))
        } else {
            Ok(None)
        }
    }
}

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
    use config::{Permit2Contract, UniswapUniversalRouter, UniswapV4PoolManager};

    let ur_address = UniswapConfig::universal_router_address();

    // Register Universal Router on all supported chains
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<UniswapUniversalRouter>(chain_id, vec![ur_address]);
    }

    // Register Permit2 (same address on all chains)
    let permit2_address = UniswapConfig::permit2_address();
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<Permit2Contract>(chain_id, vec![permit2_address]);
    }

    // Register V4 PoolManager
    for &chain_id in UniswapConfig::v4_pool_manager_chains() {
        if let Some(pm_address) = UniswapConfig::v4_pool_manager_address(chain_id) {
            contract_reg.register_contract_typed::<UniswapV4PoolManager>(chain_id, vec![pm_address]);
        }
    }

    // Register common tokens (WETH, USDC, USDT, DAI, etc.)
    UniswapConfig::register_common_tokens(contract_reg);

    // Register visualizers
    visualizer_reg.register(Box::new(UniversalRouterContractVisualizer::new()));
    visualizer_reg.register(Box::new(Permit2ContractVisualizer::new()));
    visualizer_reg.register(Box::new(V4PoolManagerContractVisualizer::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::uniswap::config::{UniswapUniversalRouter, UniswapV4PoolManager};
    use crate::registry::ContractType;
    use alloy_primitives::Address;

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

        // Verify V4 PoolManager is registered on Sepolia
        if let Some(pm_address) = UniswapConfig::v4_pool_manager_address(11155111) {
            let pm_type = contract_reg
                .get_contract_type(11155111, pm_address)
                .expect("PoolManager should be registered on Sepolia");
            assert_eq!(pm_type, UniswapV4PoolManager::short_type_id());
        }
    }
}
