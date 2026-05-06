use super::ONRE_APP_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct OnreAppConfig;

impl SolanaIntegrationConfig for OnreAppConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs: HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>> =
                HashMap::new();
            let mut instructions = HashMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert(ONRE_APP_PROGRAM_ID, instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
