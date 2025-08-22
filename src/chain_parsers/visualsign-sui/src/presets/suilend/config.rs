use std::collections::HashMap;

use crate::core::{SuiIntegrationConfig, SuiIntegrationConfigData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiLendMarketFunction {
    Repay,
    ClaimRewardsAndDeposit,
}

impl TryFrom<&str> for SuiLendMarketFunction {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "repay" => Ok(SuiLendMarketFunction::Repay),
            "claim_rewards_and_deposit" => Ok(SuiLendMarketFunction::ClaimRewardsAndDeposit),
            _ => Err(format!("Unsupported function name: {}", value)),
        }
    }
}

impl SuiLendMarketFunction {
    pub fn as_str(&self) -> &'static str {
        match self {
            SuiLendMarketFunction::Repay => "repay",
            SuiLendMarketFunction::ClaimRewardsAndDeposit => "claim_rewards_and_deposit",
        }
    }

    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![
            SuiLendMarketFunction::Repay.as_str(),
            SuiLendMarketFunction::ClaimRewardsAndDeposit.as_str(),
        ]
    }
}

pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl SuiIntegrationConfig for Config {
    fn new() -> Self {
        let mut modules = HashMap::new();
        modules.insert(
            "lending_market",
            SuiLendMarketFunction::get_supported_functions(),
        );

        let mut packages = HashMap::new();
        packages.insert(
            "0x43d25be6a55db4e7cc08dd914b8326e7d56fb64c67f0fb961a349e2872f4cc08",
            modules,
        );

        Self {
            data: SuiIntegrationConfigData { packages },
        }
    }

    fn data(&self) -> &SuiIntegrationConfigData {
        &self.data
    }
}

pub static SUILEND_CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();
