//! NearNetwork enum and display names.
//!
//! NEAR transaction envelopes carry no numeric chain id, so the network is
//! supplied out of band (defaulting to mainnet for wallet display).

use visualsign::errors::VisualSignError;

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

    /// The canonical network identifier string, the inverse of
    /// [`Self::from_network_id`]. Signed scopes use this rather than a
    /// caller-supplied spelling, so a signature does not depend on the casing
    /// the request happened to send.
    #[must_use]
    pub fn network_id(self) -> &'static str {
        match self {
            NearNetwork::Mainnet => "NEAR_MAINNET",
            NearNetwork::Testnet => "NEAR_TESTNET",
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
/// Returns `Ok(None)` when metadata is absent or carries another chain's
/// variant -- callers fall back to whatever network the converter was
/// constructed with. Returns `Err` when a `network_id` is present but doesn't
/// parse, rather than treating it the same as "absent"; a caller-supplied but
/// unrecognized network must not silently resolve to Mainnet.
///
/// [`ChainMetadata`]: generated::parser::ChainMetadata
pub fn extract_network_from_metadata(
    chain_metadata: Option<&generated::parser::ChainMetadata>,
) -> Result<Option<NearNetwork>, VisualSignError> {
    use generated::parser::chain_metadata;

    let Some(metadata) = chain_metadata else {
        return Ok(None);
    };
    let Some(inner_metadata) = metadata.metadata.as_ref() else {
        return Ok(None);
    };

    let chain_metadata::Metadata::Near(near_metadata) = inner_metadata else {
        return Ok(None);
    };
    let Some(network_id) = near_metadata.network_id.as_ref() else {
        return Ok(None);
    };
    NearNetwork::from_network_id(network_id)
        .map(Some)
        .ok_or_else(|| {
            VisualSignError::ValidationError(format!("Invalid NEAR network_id: {network_id}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_mainnet() {
        assert_eq!(NearNetwork::default(), NearNetwork::Mainnet);
    }

    #[test]
    fn network_id_round_trips_through_from_network_id() {
        for network in [NearNetwork::Mainnet, NearNetwork::Testnet] {
            assert_eq!(
                NearNetwork::from_network_id(network.network_id()),
                Some(network)
            );
        }
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
        assert_eq!(extract_network_from_metadata(None), Ok(None));
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
        assert_eq!(extract_network_from_metadata(Some(&metadata)), Ok(None));
    }

    #[test]
    fn extract_network_from_metadata_reads_near_testnet() {
        use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};

        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_TESTNET".to_string()),
                token_mappings: Default::default(),
            })),
        };
        assert_eq!(
            extract_network_from_metadata(Some(&metadata)),
            Ok(Some(NearNetwork::Testnet))
        );
    }

    #[test]
    fn extract_network_from_metadata_rejects_unrecognized_network_id() {
        use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};

        // "testnet" is the canonical `networkId` near-api-js and the NEAR CLI
        // emit; it is not one of the two strings `from_network_id` accepts.
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("testnet".to_string()),
                token_mappings: Default::default(),
            })),
        };
        assert!(extract_network_from_metadata(Some(&metadata)).is_err());
    }
}
