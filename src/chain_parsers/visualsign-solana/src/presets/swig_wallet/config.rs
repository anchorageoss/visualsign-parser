use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct SwigWalletConfig;

impl SolanaIntegrationConfig for SwigWalletConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        use std::collections::HashMap;

        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs: HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>> =
                HashMap::new();
            let mut instructions = HashMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert("swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB", instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
