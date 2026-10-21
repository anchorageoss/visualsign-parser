#![allow(dead_code)]

use crate::core::{SuiIntegrationConfig, SuiIntegrationConfigData};

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::types::ObjectID;
use visualsign::errors::VisualSignError;

const DEPOSIT_MODULE: &str = "deposit";
const WITHDRAW_MODULE: &str = "withdraw";
const DEPOSIT_FUNCTION: &str = "deposit";
const REQUEST_WITHDRAWAL_FUNCTION: &str = "request_withdrawal";
const CANCEL_WITHDRAWAL_FUNCTION: &str = "cancel_withdrawal";

/// Hashi modules the preview supports. Every deployment in [`DEPLOYMENTS`]
/// shares this layout, so it is declared once here rather than per package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashiModules {
    Deposit,
    Withdraw,
}

impl TryFrom<&str> for HashiModules {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            DEPOSIT_MODULE => Ok(Self::Deposit),
            WITHDRAW_MODULE => Ok(Self::Withdraw),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported module name: {value}"
            ))),
        }
    }
}

/// Supported functions of the `deposit` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositFunctions {
    Deposit,
}

impl TryFrom<&str> for DepositFunctions {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            DEPOSIT_FUNCTION => Ok(Self::Deposit),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported function name: {value}"
            ))),
        }
    }
}

impl DepositFunctions {
    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![DEPOSIT_FUNCTION]
    }
}

/// Supported functions of the `withdraw` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawFunctions {
    RequestWithdrawal,
    CancelWithdrawal,
}

impl TryFrom<&str> for WithdrawFunctions {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            REQUEST_WITHDRAWAL_FUNCTION => Ok(Self::RequestWithdrawal),
            CANCEL_WITHDRAWAL_FUNCTION => Ok(Self::CancelWithdrawal),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported function name: {value}"
            ))),
        }
    }
}

impl WithdrawFunctions {
    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![REQUEST_WITHDRAWAL_FUNCTION, CANCEL_WITHDRAWAL_FUNCTION]
    }
}

/// Package -> module -> function map built from [`DEPLOYMENTS`], so a
/// deployment's `package_id` is declared exactly once.
pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl Config {
    pub fn from_deployments<'a>(deployments: impl IntoIterator<Item = &'a HashiDeployment>) -> Self {
        let packages = deployments
            .into_iter()
            .map(|deployment| {
                let modules = BTreeMap::from([
                    (DEPOSIT_MODULE, DepositFunctions::get_supported_functions()),
                    (
                        WITHDRAW_MODULE,
                        WithdrawFunctions::get_supported_functions(),
                    ),
                ]);
                (deployment.package_id, modules)
            })
            .collect();
        Self {
            data: SuiIntegrationConfigData { packages },
        }
    }
}

impl SuiIntegrationConfig for Config {
    /// Keys are the `package_id` literals as written, and `can_handle` is
    /// called with `ObjectID::to_hex_literal`, so each `package_id` must be in
    /// that form: lowercase, `0x`-prefixed, without leading zeros.
    fn new() -> Self {
        Self::from_deployments(DEPLOYMENTS)
    }

    fn data(&self) -> &SuiIntegrationConfigData {
        &self.data
    }
}

pub static HASHI_CONFIG: OnceLock<Config> = OnceLock::new();

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
}

/// Sui network a Hashi deployment lives on. The core `Network` row always reads
/// "Sui Network", so the bridge preview names a test network itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiNetwork {
    Mainnet,
    Testnet,
}

impl SuiNetwork {
    pub fn display_name(self) -> &'static str {
        match self {
            SuiNetwork::Mainnet => "Sui",
            SuiNetwork::Testnet => "Sui Testnet",
        }
    }
}

/// The Hashi package version the preview supports for one environment.
///
/// [`DEPLOYMENTS`] is the only place package ids are declared: [`Config`] is
/// built from it, so adding a deployment (e.g. Sui mainnet, once its package is
/// published) is one new entry with its `package_id`, `type_origin_id`,
/// `sui_network` and `bitcoin_network`, and no other code change.
///
/// A Sui package upgrade publishes a new `package_id`. Only the versions Hashi
/// keeps enabled on-chain are configured, since a call to a disabled version
/// aborts (`assert_version_enabled` in Hashi's
/// `packages/hashi/sources/core/versioning.move`, at the commit `mod.rs` cites);
/// an upgrade that disables the old version replaces `package_id` here. Move
/// types keep the id of the package that first defined them, so
/// `type_origin_id` stays the original id and `BTC` is always
/// `<type_origin_id>::btc::BTC`.
pub struct HashiDeployment {
    pub package_id: &'static str,
    pub type_origin_id: &'static str,
    pub sui_network: SuiNetwork,
    pub bitcoin_network: BitcoinNetwork,
}

pub const DEPLOYMENTS: &[HashiDeployment] = &[
    HashiDeployment {
        package_id: "0x5e0b99c31f691742acea5de4ca1bf53c398d5639fed6503a51ffe8035dc68485",
        type_origin_id: "0x5e0b99c31f691742acea5de4ca1bf53c398d5639fed6503a51ffe8035dc68485",
        sui_network: SuiNetwork::Testnet,
        bitcoin_network: BitcoinNetwork::Signet,
    },
    HashiDeployment {
        package_id: "0xd30fb3f8914efdbeb7b8e85fae1d65dc584b44a1281f66737b92bb9f03a59d06",
        type_origin_id: "0xd30fb3f8914efdbeb7b8e85fae1d65dc584b44a1281f66737b92bb9f03a59d06",
        sui_network: SuiNetwork::Mainnet,
        bitcoin_network: BitcoinNetwork::Mainnet,
    },
];

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
