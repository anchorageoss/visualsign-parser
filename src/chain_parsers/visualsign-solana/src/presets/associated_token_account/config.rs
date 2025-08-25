use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct AssociatedTokenAccountConfig;

impl SolanaIntegrationConfig for AssociatedTokenAccountConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = std::collections::HashMap::new();
            let mut ata_instructions = std::collections::HashMap::new();
            ata_instructions.insert("*", vec!["*"]);
            programs.insert(
                "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
                ata_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
