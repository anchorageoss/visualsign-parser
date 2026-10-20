use crate::core::{SuiIntegrationConfig, SuiIntegrationConfigData};

use std::collections::BTreeMap;
use std::sync::OnceLock;

use sui_types::base_types::ObjectID;
use visualsign::errors::VisualSignError;

const USER_MODULE: &str = "user";
const OPERATE_FUNCTION: &str = "operate";
const CLOSE_POSITION_FUNCTION: &str = "close_position";
const DEPOSIT_FUNCTION: &str = "deposit";
const WITHDRAW_FUNCTION: &str = "withdraw";
const REDEEM_FUNCTION: &str = "redeem";

/// Supported functions of the vaults package's `user` module. Opening a
/// position (`init_position`) is shown on the `operate` card that funds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultFunctions {
    Operate,
    ClosePosition,
}

impl TryFrom<&str> for VaultFunctions {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            OPERATE_FUNCTION => Ok(Self::Operate),
            CLOSE_POSITION_FUNCTION => Ok(Self::ClosePosition),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported function name: {value}"
            ))),
        }
    }
}

impl VaultFunctions {
    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![OPERATE_FUNCTION, CLOSE_POSITION_FUNCTION]
    }
}

/// Supported functions of the lending package's `user` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LendingFunctions {
    Deposit,
    Withdraw,
    Redeem,
}

impl TryFrom<&str> for LendingFunctions {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            DEPOSIT_FUNCTION => Ok(Self::Deposit),
            WITHDRAW_FUNCTION => Ok(Self::Withdraw),
            REDEEM_FUNCTION => Ok(Self::Redeem),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported function name: {value}"
            ))),
        }
    }
}

impl LendingFunctions {
    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![DEPOSIT_FUNCTION, WITHDRAW_FUNCTION, REDEEM_FUNCTION]
    }
}

/// Package -> module -> function map built from [`DEPLOYMENTS`], so each
/// package id is declared exactly once.
pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl Config {
    pub fn from_deployments<'a>(deployments: impl IntoIterator<Item = &'a FluidDeployment>) -> Self {
        let packages = deployments
            .into_iter()
            .flat_map(|deployment| {
                [
                    (
                        deployment.vaults_package,
                        BTreeMap::from([(USER_MODULE, VaultFunctions::get_supported_functions())]),
                    ),
                    (
                        deployment.lending_package,
                        BTreeMap::from([(
                            USER_MODULE,
                            LendingFunctions::get_supported_functions(),
                        )]),
                    ),
                ]
            })
            .collect();
        Self {
            data: SuiIntegrationConfigData { packages },
        }
    }
}

impl SuiIntegrationConfig for Config {
    /// Keys are the package literals as written, and `can_handle` is called
    /// with `ObjectID::to_hex_literal`, so each package id must be in that
    /// form: lowercase, `0x`-prefixed, without leading zeros.
    fn new() -> Self {
        Self::from_deployments(DEPLOYMENTS)
    }

    fn data(&self) -> &SuiIntegrationConfigData {
        &self.data
    }
}

pub static FLUID_CONFIG: OnceLock<Config> = OnceLock::new();

/// Sui network a Fluid deployment lives on. The core `Network` row always
/// reads "Sui Network", so the preview names a test network itself.
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

/// The Fluid packages the preview supports on one network.
///
/// [`DEPLOYMENTS`] is the only place package ids are declared: [`Config`] is
/// built from it. A Sui package upgrade publishes a new package id, so an
/// upgrade is a change to the matching entry here.
///
/// - `vaults_package` defines `user::{operate, close_position, init_position}`,
///   the `structs` amount constructors and `position::share`.
/// - `lending_package` defines `user::{deposit, withdraw, redeem}` and the
///   `lending_state::FToken` share type.
/// - `oracle_package` defines the `view` price quote that `operate` reads.
pub struct FluidDeployment {
    pub sui_network: SuiNetwork,
    pub vaults_package: &'static str,
    pub lending_package: &'static str,
    pub oracle_package: &'static str,
}

pub const DEPLOYMENTS: &[FluidDeployment] = &[
    FluidDeployment {
        sui_network: SuiNetwork::Mainnet,
        vaults_package: "0xf0f5a9c105384d5b79e212a9c9d3adfa5d030ef4c0cf9e22fa48674f9ad3652a",
        lending_package: "0x207c696eaa1b80e6e001cb0739dff0a554b5561265008d33ca9b071fd1d202",
        oracle_package: "0xebe2409c7ea9636f0e446797515cbee2ab809d898b06d99179d9eda59e1e42d5",
    },
    FluidDeployment {
        sui_network: SuiNetwork::Testnet,
        vaults_package: "0x5bb75759ba5becce85713753f650157a826c010431337d74bf1cc7bfbbc8be8b",
        lending_package: "0x7b37c46c1593f327aa611395a3ea2f2bf99090ffac6257bd16f6a101a8ffc537",
        oracle_package: "0x31d36f4e3b9f72bafc604c20c2eaf1f45f826f8ea5757c346e87e0ee3823e107",
    },
];

/// The deployment whose vaults or lending package is `package`.
pub fn deployment_for(package: &ObjectID) -> Option<&'static FluidDeployment> {
    DEPLOYMENTS.iter().find(|deployment| {
        deployment.is_vaults_package(package) || deployment.is_lending_package(package)
    })
}

impl FluidDeployment {
    pub fn is_vaults_package(&self, package: &ObjectID) -> bool {
        parse_id(self.vaults_package) == Some(*package)
    }

    pub fn is_lending_package(&self, package: &ObjectID) -> bool {
        parse_id(self.lending_package) == Some(*package)
    }

    pub fn is_oracle_package(&self, package: &ObjectID) -> bool {
        parse_id(self.oracle_package) == Some(*package)
    }

    /// `<lending_package>::lending_state::FToken`, without type parameters.
    pub fn f_token_type_prefix(&self) -> Option<String> {
        parse_id(self.lending_package)
            .map(|id| format!("{}::lending_state::FToken<", id.to_hex_literal()))
    }
}

fn parse_id(literal: &str) -> Option<ObjectID> {
    ObjectID::from_hex_literal(literal).ok()
}

/// Coins whose decimals the preview knows. A coin's decimals live in its
/// on-chain metadata, not in the transaction, so any other coin is shown in
/// base units with its full type.
pub struct KnownCoin {
    pub coin_type: &'static str,
    pub symbol: &'static str,
    pub decimals: u32,
}

pub const KNOWN_COINS: &[KnownCoin] = &[
    KnownCoin {
        coin_type: "0x2::sui::SUI",
        symbol: "SUI",
        decimals: 9,
    },
    // Native USDC on Sui mainnet.
    KnownCoin {
        coin_type: "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        symbol: "USDC",
        decimals: 6,
    },
    // USDC on Sui testnet.
    KnownCoin {
        coin_type: "0xa1ec7fc00a6f40db9693ad1415d0c193ad3906494428cf252621037bd7117e29::usdc::USDC",
        symbol: "USDC",
        decimals: 6,
    },
    // Hashi's hBTC on Sui mainnet.
    KnownCoin {
        coin_type: "0xd30fb3f8914efdbeb7b8e85fae1d65dc584b44a1281f66737b92bb9f03a59d06::btc::BTC",
        symbol: "hBTC",
        decimals: 8,
    },
    // Hashi's hBTC on Sui testnet.
    KnownCoin {
        coin_type: "0x5e0b99c31f691742acea5de4ca1bf53c398d5639fed6503a51ffe8035dc68485::btc::BTC",
        symbol: "hBTC",
        decimals: 8,
    },
];
