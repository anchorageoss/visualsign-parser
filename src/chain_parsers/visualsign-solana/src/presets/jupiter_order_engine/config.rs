use super::JUPITER_ORDER_ENGINE_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::BTreeMap;

pub struct JupiterOrderEngineConfig;

impl SolanaIntegrationConfig for JupiterOrderEngineConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = BTreeMap::new();
            let mut instructions = BTreeMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert(JUPITER_ORDER_ENGINE_PROGRAM_ID, instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
