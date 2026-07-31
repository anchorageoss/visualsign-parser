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
}
