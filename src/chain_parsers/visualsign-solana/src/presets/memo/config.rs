use super::MEMO_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::BTreeMap;

pub struct MemoConfig;

impl SolanaIntegrationConfig for MemoConfig {
    fn new() -> Self {
        Self
    }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = BTreeMap::new();
            // The Memo program has a single, discriminator-less instruction
            // (the data buffer is the memo itself), so match every instruction
            // for the program ID.
            let mut instructions = BTreeMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert(MEMO_PROGRAM_ID, instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
