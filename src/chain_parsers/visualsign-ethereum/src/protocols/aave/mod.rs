//! Aave v3 Protocol Decoders
//!
//! This module provides transaction decoders for Aave v3 protocol operations.
//!
//! # Supported Operations
//! - `supply`: Supply assets to Aave to earn interest
//! - `withdraw`: Withdraw supplied assets from Aave
//! - `borrow`: Borrow assets against collateral
//! - `repay`: Repay borrowed assets
//! - `liquidationCall`: Liquidate undercollateralized positions
//!
//! # Example
//! ```rust,ignore
//! use crate::protocols::aave::contracts::PoolVisualizer;
//!
//! let visualizer = PoolVisualizer {};
//! let result = visualizer.visualize_pool_operation(calldata, chain_id, Some(&registry));
//! ```

pub mod config;
pub mod contracts;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;

pub use config::{AaveV3Config, AaveV3PoolContract};
pub use contracts::{
    AaveTokenVisualizer, PoolContractVisualizer, PoolVisualizer, VotingMachineVisualizer,
};

/// Registers all Aave v3 protocol contracts and visualizers
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
    // Register Pool contracts on all supported chains
    AaveV3Config::register_contracts(contract_reg);

    // Register visualizers
    visualizer_reg.register(Box::new(PoolContractVisualizer::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ContractType;
    use alloy_primitives::Address;

    #[test]
    fn test_register_aave_contracts() {
        let mut contract_reg = ContractRegistry::new();
        let mut visualizer_reg = EthereumVisualizerRegistryBuilder::new();

        register(&mut contract_reg, &mut visualizer_reg);

        // Verify Pool is registered on Ethereum mainnet
        let eth_pool: Address = "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"
            .parse()
            .unwrap();
        let contract_type = contract_reg
            .get_contract_type(1, eth_pool)
            .expect("Pool should be registered on Ethereum mainnet");
        assert_eq!(contract_type, AaveV3PoolContract::short_type_id());

        // Verify Pool is registered on Base
        let base_pool: Address = "0xA238Dd80C259a72e81d7e4664a9801593F98d1c5"
            .parse()
            .unwrap();
        let contract_type = contract_reg
            .get_contract_type(8453, base_pool)
            .expect("Pool should be registered on Base");
        assert_eq!(contract_type, AaveV3PoolContract::short_type_id());
    }
}
