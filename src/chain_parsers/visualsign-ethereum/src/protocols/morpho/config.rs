use crate::registry::{ContractRegistry, ContractType};
use alloy_primitives::Address;

/// Re-export chain ID constants from crate::networks::id
///
/// This provides access to chain constants like `networks::ethereum::MAINNET`
/// for use in Morpho configuration.
pub use crate::networks::id as networks;

/// Error type for Morpho configuration operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MorphoConfigError {
    /// Chain ID is not supported for Bundler3
    UnsupportedChain(u64),
    /// Address string failed to parse (should never happen with hardcoded addresses)
    InvalidAddress(String),
}

impl std::fmt::Display for MorphoConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MorphoConfigError::UnsupportedChain(chain_id) => {
                write!(f, "Unsupported chain ID for Morpho Bundler3: {chain_id}")
            }
            MorphoConfigError::InvalidAddress(addr) => {
                write!(f, "Invalid address: {addr}")
            }
        }
    }
}

impl std::error::Error for MorphoConfigError {}

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
    /// Returns the Bundler3 contract address for a specific chain
    ///
    /// Morpho Bundler3 contracts are deployed at different addresses on different chains.
    /// Source: https://docs.morpho.org/get-started/resources/addresses/
    ///
    /// Verified deployments:
    /// - Ethereum Mainnet: 0x6566194141eefa99Af43Bb5Aa71460Ca2Dc90245
    /// - Base: 0x6BFd8137e702540E7A42B74178A4a49Ba43920C4
    /// - Arbitrum One: 0x1FA4431bC113D308beE1d46B0e98Cb805FB48C13
    pub fn bundler3_address(chain_id: u64) -> Result<Address, MorphoConfigError> {
        let addr_str = match chain_id {
            networks::ethereum::MAINNET => "0x6566194141eefa99Af43Bb5Aa71460Ca2Dc90245",
            networks::base::MAINNET => "0x6BFd8137e702540E7A42B74178A4a49Ba43920C4",
            networks::arbitrum::MAINNET => "0x1FA4431bC113D308beE1d46B0e98Cb805FB48C13",
            _ => return Err(MorphoConfigError::UnsupportedChain(chain_id)),
        };
        addr_str
            .parse()
            .map_err(|_| MorphoConfigError::InvalidAddress(addr_str.to_string()))
    }

    /// Returns the list of chain IDs where Bundler3 is deployed
    /// Source: https://docs.morpho.org/get-started/resources/addresses/
    pub fn bundler3_chains() -> &'static [u64] {
        &[
            networks::ethereum::MAINNET, // 1 - Ethereum Mainnet
            networks::base::MAINNET,     // 8453 - Base
            networks::arbitrum::MAINNET, // 42161 - Arbitrum One
        ]
    }

    /// Registers Morpho protocol contracts in the registry
    pub fn register_contracts(registry: &mut ContractRegistry) {
        for &chain_id in Self::bundler3_chains() {
            if let Ok(bundler3_address) = Self::bundler3_address(chain_id) {
                registry
                    .register_contract_typed::<Bundler3Contract>(chain_id, vec![bundler3_address]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundler3_address() {
        // Test Ethereum Mainnet
        let ethereum_addr = MorphoConfig::bundler3_address(networks::ethereum::MAINNET).unwrap();
        assert_eq!(
            format!("{:?}", ethereum_addr).to_lowercase(),
            "0x6566194141eefa99af43bb5aa71460ca2dc90245"
        );

        // Test Base 
        let base_addr = MorphoConfig::bundler3_address(networks::base::MAINNET).unwrap();
        assert_eq!(
            format!("{:?}", base_addr).to_lowercase(),
            "0x6bfd8137e702540e7a42b74178a4a49ba43920c4"
        );

        // Test Arbitrum One
        let arbitrum_addr = MorphoConfig::bundler3_address(networks::arbitrum::MAINNET).unwrap();
        assert_eq!(
            format!("{:?}", arbitrum_addr).to_lowercase(),
            "0x1fa4431bc113d308bee1d46b0e98cb805fb48c13"
        );
    }

    #[test]
    fn test_bundler3_address_unsupported_chain() {
        let result = MorphoConfig::bundler3_address(999999);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            MorphoConfigError::UnsupportedChain(999999)
        );
    }

    #[test]
    fn test_all_chains_have_valid_addresses() {
        for &chain_id in MorphoConfig::bundler3_chains() {
            let result = MorphoConfig::bundler3_address(chain_id);
            assert!(
                result.is_ok(),
                "Chain {chain_id} should have a valid address"
            );
        }
    }

    #[test]
    fn test_bundler3_chains() {
        let chains = MorphoConfig::bundler3_chains();
        assert!(chains.contains(&networks::ethereum::MAINNET)); // Ethereum
        assert!(chains.contains(&networks::base::MAINNET)); // Base
        assert!(chains.contains(&networks::arbitrum::MAINNET)); // Arbitrum
        assert_eq!(chains.len(), 3, "Should support exactly 3 chains");
    }
}
