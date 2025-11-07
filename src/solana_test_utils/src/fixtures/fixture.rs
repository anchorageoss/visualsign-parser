use crate::common::TestAccount;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Test fixture for Solana transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaTestFixture {
    /// Human-readable description of what this test covers
    pub description: String,

    /// Optional source (e.g., Solscan URL, transaction signature)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Transaction signature (if from real transaction)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// Cluster this transaction was from (mainnet-beta, devnet, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,

    /// Additional notes about the transaction context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_transaction_note: Option<String>,

    /// Index of the instruction being tested (if transaction has multiple)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction_index: Option<usize>,

    /// Base58-encoded instruction data
    pub instruction_data: String,

    /// Program ID for this instruction
    pub program_id: String,

    /// Accounts involved in this instruction
    pub accounts: Vec<TestAccount>,

    /// Expected parsed fields (for validation)
    pub expected_fields: HashMap<String, serde_json::Value>,
}

impl SolanaTestFixture {
    pub fn new(
        description: impl Into<String>,
        program_id: impl Into<String>,
        instruction_data: impl Into<String>,
    ) -> Self {
        Self {
            description: description.into(),
            source: None,
            signature: None,
            cluster: None,
            full_transaction_note: None,
            instruction_index: None,
            instruction_data: instruction_data.into(),
            program_id: program_id.into(),
            accounts: Vec::new(),
            expected_fields: HashMap::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    pub fn with_cluster(mut self, cluster: impl Into<String>) -> Self {
        self.cluster = Some(cluster.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.full_transaction_note = Some(note.into());
        self
    }

    pub fn with_instruction_index(mut self, index: usize) -> Self {
        self.instruction_index = Some(index);
        self
    }

    pub fn with_accounts(mut self, accounts: Vec<TestAccount>) -> Self {
        self.accounts = accounts;
        self
    }

    pub fn add_account(mut self, account: TestAccount) -> Self {
        self.accounts.push(account);
        self
    }

    pub fn with_expected_field(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.expected_fields.insert(key.into(), value.into());
        self
    }

    pub fn with_expected_fields(mut self, fields: HashMap<String, serde_json::Value>) -> Self {
        self.expected_fields = fields;
        self
    }

    /// Decode the instruction data from base58
    pub fn decode_instruction_data(&self) -> anyhow::Result<Vec<u8>> {
        bs58::decode(&self.instruction_data)
            .into_vec()
            .map_err(|e| anyhow::anyhow!("Failed to decode instruction data: {}", e))
    }
}
