//! NearNetwork enum and display names.
//!
//! NEAR transaction envelopes carry no numeric chain id, so the network is
//! supplied out of band (defaulting to mainnet for wallet display).

/// A NEAR network. Used for the rendered `Network` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NearNetwork {
    #[default]
    Mainnet,
    Testnet,
}

impl NearNetwork {
    /// Display name for the rendered `Network` field.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            NearNetwork::Mainnet => "NEAR Mainnet",
            NearNetwork::Testnet => "NEAR Testnet",
        }
    }

    /// Parses a canonical network identifier string (e.g. `"NEAR_MAINNET"`,
    /// `"NEAR_TESTNET"`). Case-insensitive.
    #[must_use]
    pub fn from_network_id(network_id: &str) -> Option<Self> {
        match network_id.to_uppercase().as_str() {
            "NEAR_MAINNET" => Some(NearNetwork::Mainnet),
            "NEAR_TESTNET" => Some(NearNetwork::Testnet),
            _ => None,
        }
    }
}

/// Extracts a [`NearNetwork`] from per-request [`ChainMetadata`], if present.
///
/// Returns `None` when metadata is absent, carries another chain's variant, or
/// its `network_id` doesn't parse -- callers fall back to whatever network the
/// converter was constructed with.
///
/// [`ChainMetadata`]: generated::parser::ChainMetadata
#[must_use]
pub fn extract_network_from_metadata(
    chain_metadata: Option<&generated::parser::ChainMetadata>,
) -> Option<NearNetwork> {
    use generated::parser::chain_metadata;

    let metadata = chain_metadata?;
    let inner_metadata = metadata.metadata.as_ref()?;

    let chain_metadata::Metadata::Near(near_metadata) = inner_metadata else {
        return None;
    };
    let network_id = near_metadata.network_id.as_ref()?;
    NearNetwork::from_network_id(network_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_mainnet() {
        assert_eq!(NearNetwork::default(), NearNetwork::Mainnet);
    }

    #[test]
    fn display_names() {
        assert_eq!(NearNetwork::Mainnet.display_name(), "NEAR Mainnet");
        assert_eq!(NearNetwork::Testnet.display_name(), "NEAR Testnet");
    }

    #[test]
    fn from_network_id_parses_known_ids_case_insensitively() {
        assert_eq!(
            NearNetwork::from_network_id("NEAR_MAINNET"),
            Some(NearNetwork::Mainnet)
        );
        assert_eq!(
            NearNetwork::from_network_id("near_testnet"),
            Some(NearNetwork::Testnet)
        );
        assert_eq!(NearNetwork::from_network_id("NEAR_DEVNET"), None);
    }

    #[test]
    fn extract_network_from_metadata_none_when_absent() {
        assert_eq!(extract_network_from_metadata(None), None);
    }

    #[test]
    fn extract_network_from_metadata_none_for_other_chain() {
        use generated::parser::{ChainMetadata, EthereumMetadata, chain_metadata};

        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: Default::default(),
            })),
        };
        assert_eq!(extract_network_from_metadata(Some(&metadata)), None);
    }

    #[test]
    fn extract_network_from_metadata_reads_near_testnet() {
        use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};

        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_TESTNET".to_string()),
            })),
        };
        assert_eq!(
            extract_network_from_metadata(Some(&metadata)),
            Some(NearNetwork::Testnet)
        );
    }
}
