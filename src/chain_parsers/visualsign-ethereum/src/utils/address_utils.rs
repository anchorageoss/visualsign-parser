//! Ethereum address utilities and well-known contract addresses
//!
//! This module provides canonical addresses for contracts like WETH and USDC
//! that may not be in the registry. For most tokens, prefer using the registry.
//!
//! # Example
//!
//! ```rust,ignore
//! use visualsign_ethereum::utils::address_utils::WellKnownAddresses;
//!
//! let weth = WellKnownAddresses::weth(1)?; // Ethereum mainnet WETH
//! ```

use alloy_primitives::Address;
use std::collections::HashMap;

/// Well-known contract addresses by token name and chain ID
///
/// These are contracts that may not be in a custom registry but are canonical
/// across all chains (e.g., WETH, USDC). For protocol-specific tokens, prefer
/// using the ContractRegistry instead.
pub struct WellKnownAddresses;

impl WellKnownAddresses {
    /// Get WETH address for a chain
    pub fn weth(chain_id: u64) -> Option<Address> {
        WETH_ADDRESSES
            .get(&chain_id)
            .and_then(|addr_str| addr_str.parse().ok())
    }

    /// Get USDC address for a chain
    pub fn usdc(chain_id: u64) -> Option<Address> {
        USDC_ADDRESSES
            .get(&chain_id)
            .and_then(|addr_str| addr_str.parse().ok())
    }

    /// Get Permit2 address (same on all chains)
    pub fn permit2() -> Address {
        // Permit2 is deployed at the same address on all chains
        "0x000000000022d473030f116ddee9f6b43ac78ba3"
            .parse()
            .expect("Valid PERMIT2 address")
    }

    /// Get all addresses for a token across all chains
    pub fn all_addresses(token: &str) -> HashMap<u64, String> {
        let mut map = HashMap::new();
        match token {
            "WETH" => {
                for (&chain_id, &addr) in WETH_ADDRESSES.entries() {
                    map.insert(chain_id, addr.to_string());
                }
            }
            "USDC" => {
                for (&chain_id, &addr) in USDC_ADDRESSES.entries() {
                    map.insert(chain_id, addr.to_string());
                }
            }
            _ => {}
        }
        map
    }
}

// WETH addresses by chain ID
// Sourced from official Uniswap documentation and chain explorers
pub static WETH_ADDRESSES: phf::Map<u64, &str> = phf::phf_map! {
    1u64 => "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",      // Ethereum Mainnet
    10u64 => "0x4200000000000000000000000000000000000006",     // Optimism
    137u64 => "0x7ceb23fd6bc0add59e62ac25578270cff1b9f619",    // Polygon
    8453u64 => "0x4200000000000000000000000000000000000006",   // Base
    42161u64 => "0x82af49447d8a07e3bd95bd0d56f35241523fbab1",  // Arbitrum
};

// USDC addresses by chain ID (using the canonical USDC Bridge)
pub static USDC_ADDRESSES: phf::Map<u64, &str> = phf::phf_map! {
    1u64 => "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",      // Ethereum Mainnet
    10u64 => "0x0b2c639c533813f4aa9d7837caf62653d097ff85",     // Optimism
    137u64 => "0x2791bca1f2de4661ed88a30c99a7a9449aa84174",    // Polygon
    8453u64 => "0x833589fcd6edb6e08f4c7c32d4f71b1566469c3d",   // Base
    42161u64 => "0xff970a61a04b1ca14834a43f5de4533ebddb5f86",  // Arbitrum
};
