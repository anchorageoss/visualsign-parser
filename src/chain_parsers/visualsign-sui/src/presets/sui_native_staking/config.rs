use once_cell::sync::Lazy;
use std::collections::HashMap;

use crate::core::{SuiIntegrationConfig, SuiIntegrationConfigData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiSystemFunctions {
    AddStake,
    WithdrawStake,
}

impl TryFrom<&str> for SuiSystemFunctions {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "request_add_stake" => Ok(SuiSystemFunctions::AddStake),
            "request_withdraw_stake" => Ok(SuiSystemFunctions::WithdrawStake),
            _ => Err(format!("Unsupported function name: {}", value)),
        }
    }
}

impl SuiSystemFunctions {
    pub fn as_str(&self) -> &'static str {
        match self {
            SuiSystemFunctions::AddStake => "request_add_stake",
            SuiSystemFunctions::WithdrawStake => "request_withdraw_stake",
        }
    }

    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![
            SuiSystemFunctions::AddStake.as_str(),
            SuiSystemFunctions::WithdrawStake.as_str(),
        ]
    }
}

pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl SuiIntegrationConfig for Config {
    fn new() -> Self {
        let mut modules = HashMap::new();
        modules.insert("sui_system", SuiSystemFunctions::get_supported_functions());

        let mut packages = HashMap::new();
        packages.insert("0x3", modules);

        Self {
            data: SuiIntegrationConfigData { packages },
        }
    }

    fn data(&self) -> &SuiIntegrationConfigData {
        &self.data
    }
}

pub static NATIVE_STAKING_CONFIG: Lazy<Config> = Lazy::new(Config::new);
