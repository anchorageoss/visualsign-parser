use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Test account representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAccount {
    pub pubkey: String,
    pub signer: bool,
    pub writable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl TestAccount {
    pub fn new(pubkey: impl Into<String>, signer: bool, writable: bool) -> Self {
        Self {
            pubkey: pubkey.into(),
            signer,
            writable,
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn pubkey(&self) -> anyhow::Result<Pubkey> {
        Pubkey::from_str(&self.pubkey)
            .map_err(|e| anyhow::anyhow!("Invalid pubkey '{}': {}", self.pubkey, e))
    }

    pub fn to_account_meta(&self) -> anyhow::Result<solana_sdk::instruction::AccountMeta> {
        let pubkey = self.pubkey()?;
        Ok(solana_sdk::instruction::AccountMeta {
            pubkey,
            is_signer: self.signer,
            is_writable: self.writable,
        })
    }
}

impl From<&solana_sdk::instruction::AccountMeta> for TestAccount {
    fn from(meta: &solana_sdk::instruction::AccountMeta) -> Self {
        Self {
            pubkey: meta.pubkey.to_string(),
            signer: meta.is_signer,
            writable: meta.is_writable,
            description: None,
        }
    }
}
