use crate::core::{SuiIntegrationConfig, SuiIntegrationConfigData};

use std::collections::BTreeMap;
use std::sync::OnceLock;

use sui_types::base_types::ObjectID;
use visualsign::errors::VisualSignError;

const USER_MODULE: &str = "user";
const OPERATE_FUNCTION: &str = "operate";

/// Supported functions of the vaults package's `user` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultFunctions {
    Operate,
}

impl TryFrom<&str> for VaultFunctions {
    type Error = VisualSignError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            OPERATE_FUNCTION => Ok(Self::Operate),
            _ => Err(VisualSignError::DecodeError(format!(
                "Unsupported function name: {value}"
            ))),
        }
    }
}

impl VaultFunctions {
    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![OPERATE_FUNCTION]
    }
}

/// Package -> module -> function map built from [`DEPLOYMENTS`].
pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl Config {
    pub fn from_deployments<'a>(deployments: impl IntoIterator<Item = &'a FluidDeployment>) -> Self {
        let packages = deployments
            .into_iter()
            .map(|deployment| {
                (
                    deployment.vaults_package,
                    BTreeMap::from([(USER_MODULE, VaultFunctions::get_supported_functions())]),
                )
            })
            .collect();
        Self {
            data: SuiIntegrationConfigData { packages },
        }
    }
}

impl SuiIntegrationConfig for Config {
    /// `can_handle` matches `ObjectID::to_hex_literal`, so package ids must be in that form.
    fn new() -> Self {
        Self::from_deployments(DEPLOYMENTS)
    }

    fn data(&self) -> &SuiIntegrationConfigData {
        &self.data
    }
}

pub static FLUID_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiNetwork {
    Mainnet,
}

impl SuiNetwork {
    pub fn display_name(self) -> &'static str {
        match self {
            SuiNetwork::Mainnet => "Sui",
        }
    }
}

/// A package upgrade publishes a new package id, so it is a change to the matching entry.
pub struct FluidDeployment {
    pub sui_network: SuiNetwork,
    pub vaults_package: &'static str,
}

pub const DEPLOYMENTS: &[FluidDeployment] = &[FluidDeployment {
    sui_network: SuiNetwork::Mainnet,
    vaults_package: "0x2e39b917ac7e580fb015308843144eb335625c4784f92847fbb09236e055f19f",
}];

pub fn deployment_for(package: &ObjectID) -> Option<&'static FluidDeployment> {
    DEPLOYMENTS
        .iter()
        .find(|deployment| deployment.is_vaults_package(package))
}

impl FluidDeployment {
    pub fn is_vaults_package(&self, package: &ObjectID) -> bool {
        parse_id(self.vaults_package) == Some(*package)
    }
}

fn parse_id(literal: &str) -> Option<ObjectID> {
    ObjectID::from_hex_literal(literal).ok()
}

/// Decimals live in on-chain metadata, not the transaction; other coins show in base units.
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
    KnownCoin {
        coin_type: "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        symbol: "USDC",
        decimals: 6,
    },
    KnownCoin {
        coin_type: "0xd30fb3f8914efdbeb7b8e85fae1d65dc584b44a1281f66737b92bb9f03a59d06::btc::BTC",
        symbol: "hBTC",
        decimals: 8,
    },
];
