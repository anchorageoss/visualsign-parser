//! Generic contract standards
//!
//! This module contains generic contract standards that are used across
//! multiple protocols (e.g., ERC20, ERC721, ERC1155).
//!
//! Protocol-specific contracts are located in the `protocols` module.

pub mod core;

pub use core::*;
