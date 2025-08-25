use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct JupiterConfig;

impl SolanaIntegrationConfig for JupiterConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut jupiter_instructions = HashMap::new();
            jupiter_instructions.insert("*", vec!["*"]);
            programs.insert(
                "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                jupiter_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
