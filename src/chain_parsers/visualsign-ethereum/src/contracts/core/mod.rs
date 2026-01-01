//! Core contract standards (ERC20, ERC721, etc.)

pub mod dynamic_abi;
pub mod erc20;
pub mod erc721;
pub mod fallback;

pub use dynamic_abi::DynamicAbiVisualizer;
pub use erc20::ERC20Visualizer;
pub use erc721::ERC721Visualizer;
pub use fallback::FallbackVisualizer;
