//! Configuration for Token 2022 program integration

use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct Token2022Config;

impl SolanaIntegrationConfig for Token2022Config {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut token2022_instructions = HashMap::new();
            token2022_instructions.insert("*", vec!["*"]);
            // Token 2022 program ID
            programs.insert(
                "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
                token2022_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
