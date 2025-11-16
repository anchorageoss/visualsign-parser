use crate::registry::{ContractRegistry, ContractType};
use alloy_primitives::Address;

/// Morpho Bundler3 contract type identifier
pub struct Bundler3Contract;

impl ContractType for Bundler3Contract {
    fn short_type_id() -> &'static str {
        "morpho_bundler3"
    }
}

/// Configuration for Morpho protocol contracts
pub struct MorphoConfig;

impl MorphoConfig {
    /// Returns the Bundler3 contract address (same on all chains)
    /// Source: https://docs.morpho.org/contracts/addresses
    pub fn bundler3_address() -> Address {
        "0x6566194141eefa99Af43Bb5Aa71460Ca2Dc90245"
            .parse()
            .unwrap()
    }

    /// Returns the list of chain IDs where Bundler3 is deployed
    pub fn bundler3_chains() -> &'static [u64] {
        &[
            1,     // Ethereum Mainnet
            10,    // Optimism
            8453,  // Base
            42161, // Arbitrum One
        ]
    }

    /// Registers Morpho protocol contracts in the registry
    pub fn register_contracts(registry: &mut ContractRegistry) {
        let bundler3_address = Self::bundler3_address();

        for &chain_id in Self::bundler3_chains() {
            registry.register_contract_typed::<Bundler3Contract>(chain_id, vec![bundler3_address]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundler3_address() {
        let addr = MorphoConfig::bundler3_address();
        assert_eq!(
            format!("{:?}", addr).to_lowercase(),
            "0x6566194141eefa99af43bb5aa71460ca2dc90245"
        );
    }

    #[test]
    fn test_bundler3_chains() {
        let chains = MorphoConfig::bundler3_chains();
        assert!(chains.contains(&1)); // Ethereum
        assert!(chains.contains(&10)); // Optimism
        assert!(chains.contains(&8453)); // Base
        assert!(chains.contains(&42161)); // Arbitrum
    }
}
