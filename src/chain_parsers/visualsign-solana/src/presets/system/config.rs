//! Configuration for System program integration

use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct SystemConfig;

impl SolanaIntegrationConfig for SystemConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut system_instructions = HashMap::new();
            system_instructions.insert("*", vec!["*"]);
            programs.insert("11111111111111111111111111111111", system_instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
