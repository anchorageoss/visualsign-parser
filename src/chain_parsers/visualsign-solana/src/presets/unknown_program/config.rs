//! Configuration for unknown/unsupported program fallback
//! This is a catch-all visualizer that handles any program not supported by other visualizers

use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct UnknownProgramConfig;

impl SolanaIntegrationConfig for UnknownProgramConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            // This is a catch-all - it doesn't match specific programs
            // Instead, can_handle is overridden to always return true
            let programs = HashMap::new();
            SolanaIntegrationConfigData { programs }
        })
    }

    // Override can_handle to always return true - this is a catch-all fallback
    fn can_handle(
        &self,
        _program_id: &str,
        _instruction: &solana_sdk::instruction::Instruction,
    ) -> bool {
        true
    }
}
