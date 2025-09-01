//! Configuration for Stakepool program integration

use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct StakepoolConfig;

impl SolanaIntegrationConfig for StakepoolConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut stakepool_instructions = HashMap::new();
            stakepool_instructions.insert("*", vec!["*"]);
            // this is a weaker version, we can probably do a prefix match on SPoo1
            programs.insert(
                "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
                stakepool_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
