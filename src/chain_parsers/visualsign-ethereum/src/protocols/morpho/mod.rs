//! Morpho protocol implementation
//!
//! This module contains contract visualizers, configuration, and registration
//! logic for the Morpho lending protocol.
//!
//! Morpho is a decentralized lending protocol that optimizes interest rates
//! through peer-to-peer matching while maintaining liquidity pool fallbacks.

pub mod config;
pub mod contracts;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;

pub use config::{Bundler3Contract, MorphoConfig};
pub use contracts::{BundlerContractVisualizer, BundlerVisualizer};

/// Registers all Morpho protocol contracts and visualizers
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
    // Register Bundler3 contract on all supported chains
    MorphoConfig::register_contracts(contract_reg);

    // Register visualizers
    visualizer_reg.register(Box::new(BundlerContractVisualizer::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ContractType;

    #[test]
    fn test_register_morpho_contracts() {
        let mut contract_reg = ContractRegistry::new();
        let mut visualizer_reg = EthereumVisualizerRegistryBuilder::new();

        register(&mut contract_reg, &mut visualizer_reg);

        // Verify Bundler3 is registered on all supported chains
        for chain_id in [1, 8453, 42161] {
            let expected_address = MorphoConfig::bundler3_address(chain_id)
                .expect(&format!("Should have valid address for chain {}", chain_id));
            
            let contract_type = contract_reg
                .get_contract_type(chain_id, expected_address)
                .expect(&format!(
                    "Bundler3 should be registered on chain {}",
                    chain_id
                ));
            assert_eq!(contract_type, Bundler3Contract::short_type_id());
        }
    }
}
