use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct SplTokenConfig;

impl SolanaIntegrationConfig for SplTokenConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = std::collections::HashMap::new();
            let mut spl_token_instructions = std::collections::HashMap::new();
            spl_token_instructions.insert("*", vec!["*"]);
            programs.insert(
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                spl_token_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
