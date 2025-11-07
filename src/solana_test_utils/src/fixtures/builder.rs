use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signature, Signer},
    transaction::Transaction,
};

/// Builder for creating and executing Solana transactions
pub struct SolanaTransactionBuilder {
    instructions: Vec<Instruction>,
    signers: Vec<Keypair>,
    payer: Option<Keypair>,
    recent_blockhash: Option<Hash>,
}

impl SolanaTransactionBuilder {
    /// Create a new transaction builder
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            signers: Vec::new(),
            payer: None,
            recent_blockhash: None,
        }
    }

    /// Add an instruction to the transaction
    pub fn add_instruction(mut self, instruction: Instruction) -> Self {
        self.instructions.push(instruction);
        self
    }

    /// Add a program instruction with the given details
    pub fn add_program_instruction(
        self,
        program_id: Pubkey,
        data: Vec<u8>,
        accounts: Vec<AccountMeta>,
    ) -> Self {
        self.add_instruction(Instruction {
            program_id,
            accounts,
            data,
        })
    }

    /// Add a signer keypair
    pub fn add_signer(mut self, signer: Keypair) -> Self {
        self.signers.push(signer);
        self
    }

    /// Set the fee payer (defaults to first signer if not set)
    pub fn set_payer(mut self, payer: Keypair) -> Self {
        self.payer = Some(payer);
        self
    }

    /// Set the recent blockhash manually
    pub fn set_recent_blockhash(mut self, blockhash: Hash) -> Self {
        self.recent_blockhash = Some(blockhash);
        self
    }

    /// Build the transaction (without sending)
    pub fn build(&self, recent_blockhash: Hash) -> Result<Transaction> {
        if self.instructions.is_empty() {
            return Err(anyhow::anyhow!("No instructions provided"));
        }

        // Determine the payer
        let payer = if let Some(ref p) = self.payer {
            p
        } else if let Some(first_signer) = self.signers.first() {
            first_signer
        } else {
            return Err(anyhow::anyhow!("No payer or signers provided"));
        };

        let message = Message::new(&self.instructions, Some(&payer.pubkey()));

        let mut signers: Vec<&Keypair> = vec![payer];
        for signer in &self.signers {
            // Avoid duplicates
            if signer.pubkey() != payer.pubkey() {
                signers.push(signer);
            }
        }

        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&signers, recent_blockhash);

        Ok(transaction)
    }

    /// Execute the transaction on the given RPC client
    pub async fn execute(self, client: &RpcClient) -> Result<Signature> {
        let recent_blockhash = if let Some(hash) = self.recent_blockhash {
            hash
        } else {
            client
                .get_latest_blockhash()
                .context("Failed to get recent blockhash")?
        };

        let transaction = self.build(recent_blockhash)?;

        let signature = client
            .send_and_confirm_transaction(&transaction)
            .context("Failed to send and confirm transaction")?;

        Ok(signature)
    }

    /// Get reference to instructions
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
}

impl Default for SolanaTransactionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(deprecated)]
    use solana_sdk::system_instruction;

    #[test]
    fn test_builder_basic() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let builder = SolanaTransactionBuilder::new()
            .add_instruction(system_instruction::transfer(
                &payer.pubkey(),
                &to.pubkey(),
                1_000_000,
            ))
            .set_payer(payer);

        assert_eq!(builder.instructions().len(), 1);
    }

    #[test]
    fn test_builder_build() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let transaction = SolanaTransactionBuilder::new()
            .add_instruction(system_instruction::transfer(
                &payer.pubkey(),
                &to.pubkey(),
                1_000_000,
            ))
            .set_payer(payer)
            .build(Hash::default())
            .unwrap();

        assert_eq!(transaction.message.instructions.len(), 1);
    }
}
