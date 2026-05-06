use super::METADAO_FUTARCHY_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct MetadaoFutarchyConfig;

impl SolanaIntegrationConfig for MetadaoFutarchyConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut instructions = HashMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert(METADAO_FUTARCHY_PROGRAM_ID, instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
