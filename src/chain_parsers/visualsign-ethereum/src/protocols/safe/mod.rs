pub mod config;
pub mod contracts;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;
use config::SafeConfig;
use contracts::SafeWalletVisualizer;

/// Register Safe protocol contracts and visualizers
pub fn register(
    contract_reg: &mut ContractRegistry,
    visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
) {
    // Register known Safe deployment addresses
    SafeConfig::register_contracts(contract_reg);

    // Register the visualizer for all Safe wallets
    visualizer_reg.register(Box::new(SafeWalletVisualizer::new()));
}
