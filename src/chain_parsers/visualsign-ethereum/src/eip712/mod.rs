//! EIP-712 typed-data signing with ERC-7730 descriptor-driven rendering.

pub mod descriptor;
pub mod encoding;
pub mod fallback;
pub mod format;
pub mod payload;
pub mod visualizer;

// Re-export the public surface.
pub use payload::{Domain, Eip712Payload};
pub use visualizer::{Eip712TransactionWrapper, Eip712VisualSignConverter};
