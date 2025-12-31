use alloy_primitives::U256;
use alloy_sol_types::{SolCall, sol};
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::contracts::core::DynamicAbiVisualizer;
use visualsign_ethereum::embedded_abis::register_embedded_abi;
use visualsign_ethereum::visualizer::CalldataVisualizer;

sol! {
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

// Embed real contract ABIs
const USDC_ABI: &str = include_str!("contracts/USDC.abi.json");
const USDT_ABI: &str = include_str!("contracts/USDT.abi.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and populate registry
    let mut registry = AbiRegistry::new();
    register_embedded_abi(&mut registry, "USDC", USDC_ABI)?;
    register_embedded_abi(&mut registry, "USDT", USDT_ABI)?;

    // Map to known addresses (Ethereum mainnet)
    let usdc_addr: alloy_primitives::Address =
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse()?;
    let usdt_addr: alloy_primitives::Address =
        "0xdac17f958d2ee523a2206206994597c13d831ec7".parse()?;

    registry.map_address(1, usdc_addr, "USDC");
    registry.map_address(1, usdt_addr, "USDT");

    println!("Registry created with 2 ABIs:");
    println!("  - USDC: {usdc_addr}");
    println!("  - USDT: {usdt_addr}");
    println!();

    // Test: Decode USDC transfer
    println!("Testing USDC transfer decoding...");
    if let Some(abi) = registry.get_abi_for_address(1, usdc_addr) {
        let visualizer = DynamicAbiVisualizer::new(abi);

        // transfer(address to, uint256 amount) - using typesafe encoding
        let recipient: alloy_primitives::Address =
            "0x1234567890123456789012345678901234567890".parse()?;
        let amount = 1_000_000u128; // 1 USDC (6 decimals)

        // Build calldata using typesafe alloy encoder
        let call = IERC20::transferCall {
            to: recipient,
            amount: U256::from(amount),
        };
        let calldata = IERC20::transferCall::abi_encode(&call);

        if let Some(field) = visualizer.visualize_calldata(&calldata, 1, None) {
            println!("✓ Successfully visualized USDC transfer");
            println!("  Field: {field:#?}");
        } else {
            println!("✗ Could not visualize");
        }
    }

    Ok(())
}
