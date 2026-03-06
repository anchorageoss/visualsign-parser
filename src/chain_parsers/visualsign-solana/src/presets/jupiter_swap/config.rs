use super::JUPITER_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct JupiterSwapConfig;

impl SolanaIntegrationConfig for JupiterSwapConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut jupiter_instructions = HashMap::new();
            jupiter_instructions.insert("*", vec!["*"]);
            programs.insert(JUPITER_PROGRAM_ID, jupiter_instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
