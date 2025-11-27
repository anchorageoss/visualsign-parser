//! Core contract standards (ERC20, ERC721, etc.)

pub mod erc20;
pub mod erc721;
pub mod fallback;

pub use erc20::ERC20Visualizer;
pub use erc721::ERC721Visualizer;
pub use fallback::FallbackVisualizer;
