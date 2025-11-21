pub mod morpho;
pub mod safe;
pub mod uniswap;

use crate::registry::ContractRegistry;
use crate::visualizer::EthereumVisualizerRegistryBuilder;

/// Registers all available protocol contracts and visualizers
///
/// # Arguments
/// * `contract_reg` - The contract registry to register addresses
/// * `visualizer_reg` - The visualizer registry to register visualizers
pub fn register_all(
    contract_reg: &mut ContractRegistry,
    visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
) {
    // Register Morpho protocol
    morpho::register(contract_reg, visualizer_reg);

    // Register Safe protocol
    safe::register(contract_reg, visualizer_reg);

    // Register Uniswap protocol
    uniswap::register(contract_reg, visualizer_reg);
}
