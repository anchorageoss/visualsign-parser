#![allow(dead_code)]

use sui_types::base_types::ObjectID;

// Every non-object argument of the configured functions is a PTB result or a
// vector, and the numbers the visualizer decodes belong to the producer
// commands, so the macro's typed getters do not apply and the index enums stay
// empty.
crate::chain_config! {
    config HASHI_CONFIG as Config;

    hashi_testnet => {
        package_id => 0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e,
        modules as HashiModules: {
            deposit as Deposit => DepositFunctions: {
                deposit as Deposit => DepositIndexes(),
            },
        }
    },
}

/// Bitcoin network a Hashi deployment settles on. The network lives in the
/// shared `Hashi` object's config, not in the PTB, so it is pinned per package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitcoinNetwork {
    Mainnet,
    Signet,
}

impl BitcoinNetwork {
    pub fn bech32_hrp(self) -> &'static str {
        match self {
            BitcoinNetwork::Mainnet => "bc",
            BitcoinNetwork::Signet => "tb",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            BitcoinNetwork::Mainnet => "Bitcoin",
            BitcoinNetwork::Signet => "Bitcoin Signet",
        }
    }

    /// Titles render verbatim as the operation name, so a test-network
    /// operation must say so there rather than only in the expanded fields.
    pub fn title_suffix(self) -> &'static str {
        match self {
            BitcoinNetwork::Mainnet => "",
            BitcoinNetwork::Signet => " (Bitcoin Signet)",
        }
    }
}

/// The Hashi package version the preview supports for one environment. Its
/// package id must match the one in `chain_config!` above.
///
/// A Sui package upgrade publishes a new `package_id`. Only the versions Hashi
/// keeps enabled on-chain are configured, since a call to a disabled version
/// aborts; an upgrade that disables the old version replaces `package_id` here
/// and above. Move types keep the id of the package that first defined them,
/// so `type_origin_id` stays the original id and `BTC` is always
/// `<type_origin_id>::btc::BTC`.
pub struct HashiDeployment {
    pub package_id: &'static str,
    pub type_origin_id: &'static str,
    pub bitcoin_network: BitcoinNetwork,
}

pub const DEPLOYMENTS: &[HashiDeployment] = &[HashiDeployment {
    package_id: "0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e",
    type_origin_id: "0xfcea10cadbb553c4874201584abf68771592678952efd957b2e82c010c7f4360",
    bitcoin_network: BitcoinNetwork::Signet,
}];

pub fn deployment_for(package: &ObjectID) -> Option<&'static HashiDeployment> {
    DEPLOYMENTS
        .iter()
        .find(|deployment| parse_id(deployment.package_id).is_some_and(|id| id == *package))
}

impl HashiDeployment {
    pub fn type_origin(&self) -> Option<ObjectID> {
        parse_id(self.type_origin_id)
    }

    /// True when `package` is a configured version of this deployment.
    pub fn owns_package(&self, package: &ObjectID) -> bool {
        let Some(origin) = self.type_origin() else {
            return false;
        };
        DEPLOYMENTS.iter().any(|deployment| {
            parse_id(deployment.type_origin_id) == Some(origin)
                && parse_id(deployment.package_id).is_some_and(|id| id == *package)
        })
    }
}

fn parse_id(literal: &str) -> Option<ObjectID> {
    ObjectID::from_hex_literal(literal).ok()
}
