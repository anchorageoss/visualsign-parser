//! Script to simulate Uniswap V4 transactions using foundry's `cast` and `anvil`
//!
//! This script demonstrates how to simulate transactions against a local fork
//! to understand what happens during execution (events, traces, etc.)
//! which helps in building accurate visualizers.

use std::process::Command;
use std::str;

/// The V4 PoolManager address on Sepolia
const POOL_MANAGER: &str = "0x000000000004444c5dc75cB358380D2e3dE08A90";

fn main() {
    println!("Simulating Uniswap V4 transactions...");
    println!("Target Contract: {}", POOL_MANAGER);

    // 1. Initialize Pool
    println!("\n--- Simulating initialize() ---");
    let calldata_init = "0x695c5bf5000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000bb8000000000000000000000000000000000000000000000000000000000000003c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000000";
    
    simulate_transaction(calldata_init);

    // 2. Swap
    println!("\n--- Simulating swap() ---");
    let calldata_swap = "0xf3cd914c000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000bb8000000000000000000000000000000000000000000000000000000000000003c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000de0b6b3a7640000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000000";

    simulate_transaction(calldata_swap);
}

fn simulate_transaction(calldata: &str) {
    // Use cast call with --trace to get execution details
    // We assume anvil is running on localhost:8545 (or use a public RPC if not)
    // For better traces, using a local fork is best.
    
    // Using a public RPC for demonstration if anvil isn't reachable, 
    // but the user should ideally point this to their local anvil instance
    // created with `anvil --fork-url ...`
    let rpc_url = "http://127.0.0.1:8545"; 

    println!("Executing cast call --trace...");
    
    let output = Command::new("cast")
        .arg("call")
        .arg(POOL_MANAGER)
        .arg(calldata)
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--trace") // Request full trace
        .output()
        .expect("Failed to execute cast command");

    if output.status.success() {
        println!("Transaction simulated successfully!");
        let stdout = str::from_utf8(&output.stdout).unwrap();
        println!("Output (Trace truncated):");
        // Print first 20 lines of output to avoid spamming
        for line in stdout.lines().take(20) {
            println!("{}", line);
        }
        if stdout.lines().count() > 20 {
            println!("... (rest of trace hidden) ...");
        }
    } else {
        println!("Simulation failed (expected if Anvil is not running or tx reverts):");
        let stderr = str::from_utf8(&output.stderr).unwrap();
        println!("{}", stderr);
        println!("Make sure you are running: anvil --fork-url <SEPOLIA_RPC>");
    }
}

