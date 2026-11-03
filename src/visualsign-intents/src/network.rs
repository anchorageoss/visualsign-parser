//! The network NEAR Intents settle on.
//!
//! Always a NEAR network, whichever chain's key signed: a `DefusePayload` names
//! NEAR accounts and is verified by a NEAR contract regardless of whether the
//! signature arrived under NEP-413, raw ed25519 or ERC-191.

/// The settlement network. Two variants because `intents.near` and
/// `intents.testnet` are the only deployments.
///
/// No `Default`: a caller must name the network, so a missing one can't
/// silently become mainnet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementNetwork {
    Mainnet,
    Testnet,
}

impl SettlementNetwork {
    /// The canonical network identifier. Token-metadata signatures are bound to
    /// this rather than to a caller-supplied spelling, so a signature does not
    /// depend on the casing a request happened to send.
    #[must_use]
    pub fn network_id(self) -> &'static str {
        match self {
            Self::Mainnet => "NEAR_MAINNET",
            Self::Testnet => "NEAR_TESTNET",
        }
    }

    /// The NEAR Intents verifier contract deployed on this network.
    #[must_use]
    pub fn verifier(self) -> &'static str {
        match self {
            Self::Mainnet => crate::INTENTS_RECEIVER,
            Self::Testnet => "intents.testnet",
        }
    }
}

/// Detects an account whose top-level suffix contradicts the resolved network
/// (`.testnet` under Mainnet, or `.near` under Testnet). `role` names which
/// account failed. Implicit 64-hex accounts carry no suffix and are not guarded.
///
/// One implementation on purpose: it is applied both to a NEAR transaction's own
/// accounts and to every rendered envelope's accounts, so the same accounts
/// cannot be a hard error on one path and a clean payload on another.
/// `visualsign-near` delegates its own `network_mismatch` here rather than
/// keeping a second copy that could drift.
#[must_use]
pub fn account_network_mismatch(
    role: &str,
    account_id: &str,
    network: SettlementNetwork,
) -> Option<String> {
    let mismatched = match network {
        SettlementNetwork::Mainnet => account_id.ends_with(".testnet"),
        SettlementNetwork::Testnet => account_id.ends_with(".near"),
    };
    mismatched.then(|| {
        format!("{role} account '{account_id}' does not match resolved network {network:?}")
    })
}
