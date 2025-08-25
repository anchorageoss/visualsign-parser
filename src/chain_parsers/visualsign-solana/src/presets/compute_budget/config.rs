use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};

pub struct ComputeBudgetConfig;

impl SolanaIntegrationConfig for ComputeBudgetConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = std::collections::HashMap::new();
            let mut compute_budget_instructions = std::collections::HashMap::new();
            compute_budget_instructions.insert("*", vec!["*"]);
            programs.insert(
                "ComputeBudget111111111111111111111111111111",
                compute_budget_instructions,
            );
            SolanaIntegrationConfigData { programs }
        })
    }
}
