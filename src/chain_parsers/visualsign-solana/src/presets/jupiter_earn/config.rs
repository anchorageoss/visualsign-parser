use super::JUPITER_EARN;
use crate::core::receipt_token;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct JupiterEarnConfig;

impl SolanaIntegrationConfig for JupiterEarnConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| receipt_token::config_data(std::slice::from_ref(&JUPITER_EARN)))
    }
}
