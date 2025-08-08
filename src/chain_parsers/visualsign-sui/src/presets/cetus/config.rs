use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fmt::Display;

use crate::{
    core::{SuiIntegrationConfig, SuiIntegrationConfigData},
    utils::{decode_number, get_index},
};

use sui_json_rpc_types::{SuiArgument, SuiCallArg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolScriptV2Functions {
    SwapB2A,
}

impl TryFrom<&str> for PoolScriptV2Functions {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "swap_b2a" => Ok(PoolScriptV2Functions::SwapB2A),
            _ => Err(format!("Unsupported function name: {}", value)),
        }
    }
}

impl Display for PoolScriptV2Functions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PoolScriptV2Functions {
    pub fn as_str(&self) -> &'static str {
        match self {
            PoolScriptV2Functions::SwapB2A => "swap_b2a",
        }
    }

    pub fn get_supported_functions() -> Vec<&'static str> {
        vec![PoolScriptV2Functions::SwapB2A.as_str()]
    }
}

impl AsRef<str> for PoolScriptV2Functions {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Copy)]
pub enum SwapB2AIndexes {
    InputAmount = 5,
    MinOutputAmount = 6,
}

impl SwapB2AIndexes {
    pub fn get_input_amount(inputs: &[SuiCallArg], args: &[SuiArgument]) -> Option<u64> {
        decode_number::<u64>(
            inputs.get(get_index(args, Some(SwapB2AIndexes::InputAmount as usize))? as usize)?,
        )
    }

    pub fn get_min_output_amount(inputs: &[SuiCallArg], args: &[SuiArgument]) -> Option<u64> {
        decode_number::<u64>(
            inputs
                .get(get_index(args, Some(SwapB2AIndexes::MinOutputAmount as usize))? as usize)?,
        )
    }
}

pub struct Config {
    pub data: SuiIntegrationConfigData,
}

impl SuiIntegrationConfig for Config {
    fn new() -> Self {
        let mut modules = HashMap::new();
        modules.insert(
            "pool_script_v2",
            PoolScriptV2Functions::get_supported_functions(),
        );

        let mut packages = HashMap::new();
        packages.insert(
            "0xb2db7142fa83210a7d78d9c12ac49c043b3cbbd482224fea6e3da00aa5a5ae2d",
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

pub static CETUS_CONFIG: Lazy<Config> = Lazy::new(Config::new);
