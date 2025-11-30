//! Reusable Ethereum decoder utilities for DApp protocols
//!
//! This module provides shared utilities for decoding Solidity contract calls and creating
//! visualizations. These utilities are designed to be reusable across any DApp that uses
//! Solidity contracts, making it easy to add support for new protocols (e.g., Aave, Curve, etc).

pub mod address_utils;

pub use address_utils::*;
