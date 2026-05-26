use std::collections::HashMap;
use visualsign::registry::Chain;

fn chain_string_mapping() -> HashMap<&'static str, Chain> {
    let mut mapping = HashMap::new();
    mapping.insert("solana", Chain::Solana);
    mapping.insert("ethereum", Chain::Ethereum);
    mapping.insert("bitcoin", Chain::Bitcoin);
    mapping.insert("sui", Chain::Sui);
    mapping.insert("aptos", Chain::Aptos);
    mapping.insert("polkadot", Chain::Polkadot);
    mapping.insert("tron", Chain::Tron);
    mapping
}

/// Parses a chain string into a Chain enum value.
/// Returns `Chain::Unspecified` if the chain string is not recognized.
#[must_use]
pub fn parse_chain(chain_str: &str) -> Chain {
    chain_string_mapping()
        .get(chain_str)
        .cloned()
        .unwrap_or(Chain::Unspecified)
}

/// Returns a vector of all available chain names as string slices.
#[must_use]
pub fn available_chains() -> Vec<&'static str> {
    chain_string_mapping().keys().copied().collect()
}
