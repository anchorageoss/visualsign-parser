use std::collections::BTreeMap;
use std::str::FromStr;
use visualsign::registry::Chain;

fn chain_string_mapping() -> BTreeMap<&'static str, Chain> {
    let mut mapping = BTreeMap::new();
    mapping.insert("solana", Chain::Solana);
    mapping.insert("ethereum", Chain::Ethereum);
    mapping.insert("bitcoin", Chain::Bitcoin);
    mapping.insert("sui", Chain::Sui);
    mapping.insert("aptos", Chain::Aptos);
    mapping.insert("polkadot", Chain::Polkadot);
    mapping.insert("tron", Chain::Tron);
    mapping
}

/// Parses a chain string into a `Chain`.
///
/// Known chain names (case-insensitive) map to their dedicated variant,
/// including `"unspecified"` -> `Chain::Unspecified`. Any other string maps
/// to `Chain::Custom`, so a chain contributed by an external
/// [`crate::ChainPlugin`] is selected by the plugin that registered under
/// the matching `Chain::Custom`.
#[must_use]
pub fn parse_chain(chain_str: &str) -> Chain {
    Chain::from_str(chain_str).unwrap_or_else(|()| Chain::Custom(chain_str.to_string()))
}

/// Returns a vector of all available chain names as string slices.
#[must_use]
pub fn available_chains() -> Vec<&'static str> {
    chain_string_mapping().keys().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chain_maps_builtins() {
        assert_eq!(parse_chain("ethereum"), Chain::Ethereum);
        assert_eq!(parse_chain("solana"), Chain::Solana);
    }

    #[test]
    fn parse_chain_builtins_are_case_insensitive() {
        assert_eq!(parse_chain("Ethereum"), Chain::Ethereum);
        assert_eq!(parse_chain("SOLANA"), Chain::Solana);
    }

    #[test]
    fn parse_chain_maps_unknown_to_custom() {
        assert_eq!(parse_chain("near"), Chain::Custom("near".to_string()));
    }
}
