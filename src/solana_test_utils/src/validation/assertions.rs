use solana_sdk::{pubkey::Pubkey, transaction::Transaction};

/// Trait for making assertions about Solana transactions
pub trait SolanaAssertions {
    /// Assert that the transaction uses the specified program
    fn assert_program(&self, expected: &Pubkey) -> &Self;

    /// Assert the number of instructions in the transaction
    fn assert_instruction_count(&self, count: usize) -> &Self;

    /// Assert that a specific account is a signer
    fn assert_signer(&self, pubkey: &Pubkey) -> &Self;

    /// Assert that the transaction is signed
    fn assert_signed(&self) -> &Self;

    /// Assert that a specific account is writable
    fn assert_writable(&self, pubkey: &Pubkey) -> &Self;

    /// Assert that a specific account is present in the transaction
    fn assert_account_present(&self, pubkey: &Pubkey) -> &Self;
}

impl SolanaAssertions for Transaction {
    fn assert_program(&self, expected: &Pubkey) -> &Self {
        let has_program = self
            .message
            .instructions
            .iter()
            .any(|ix| &self.message.account_keys[ix.program_id_index as usize] == expected);

        assert!(
            has_program,
            "Transaction does not use program: {}",
            expected
        );
        self
    }

    fn assert_instruction_count(&self, count: usize) -> &Self {
        assert_eq!(
            self.message.instructions.len(),
            count,
            "Expected {} instructions, found {}",
            count,
            self.message.instructions.len()
        );
        self
    }

    fn assert_signer(&self, pubkey: &Pubkey) -> &Self {
        let is_signer = self
            .message
            .account_keys
            .iter()
            .position(|k| k == pubkey)
            .map(|index| self.message.is_signer(index))
            .unwrap_or(false);

        assert!(is_signer, "Account {} is not a signer", pubkey);
        self
    }

    fn assert_signed(&self) -> &Self {
        assert!(
            !self.signatures.is_empty(),
            "Transaction has no signatures"
        );
        self
    }

    fn assert_writable(&self, pubkey: &Pubkey) -> &Self {
        let is_writable = self
            .message
            .account_keys
            .iter()
            .position(|k| k == pubkey)
            .map(|index| self.message.is_maybe_writable(index, None))
            .unwrap_or(false);

        assert!(is_writable, "Account {} is not writable", pubkey);
        self
    }

    fn assert_account_present(&self, pubkey: &Pubkey) -> &Self {
        assert!(
            self.message.account_keys.contains(pubkey),
            "Account {} is not present in transaction",
            pubkey
        );
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(deprecated)]
    use solana_sdk::{
        hash::Hash, message::Message, signature::Keypair, signer::Signer,
        system_instruction,
    };

    #[test]
    fn test_assertions() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let instruction = system_instruction::transfer(&payer.pubkey(), &to.pubkey(), 1_000_000);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[&payer], Hash::default());

        transaction
            .assert_signed()
            .assert_instruction_count(1)
            .assert_signer(&payer.pubkey())
            .assert_account_present(&payer.pubkey())
            .assert_account_present(&to.pubkey());
    }

    #[test]
    #[should_panic(expected = "is not a signer")]
    fn test_assert_signer_fails() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let instruction = system_instruction::transfer(&payer.pubkey(), &to.pubkey(), 1_000_000);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[&payer], Hash::default());

        // This should panic because 'to' is not a signer
        transaction.assert_signer(&to.pubkey());
    }
}
