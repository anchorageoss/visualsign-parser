use super::ABRA_PROGRAMS;
use crate::core::receipt_token;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct AbraConfig;

impl SolanaIntegrationConfig for AbraConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| receipt_token::config_data(&ABRA_PROGRAMS))
    }
}
