use anyhow::Result;
use solana_sdk::{pubkey::Pubkey, transaction::Transaction};
use std::collections::HashMap;

/// Validator for Solana transactions with fluent API
pub struct TransactionValidator<'a> {
    transaction: &'a Transaction,
    errors: Vec<String>,
}

impl<'a> TransactionValidator<'a> {
    /// Create a new validator for the given transaction
    pub fn new(transaction: &'a Transaction) -> Self {
        Self {
            transaction,
            errors: Vec::new(),
        }
    }

    /// Validate that the transaction uses a specific program
    pub fn validate_program(mut self, expected: &Pubkey) -> Self {
        let has_program = self
            .transaction
            .message
            .instructions
            .iter()
            .any(|ix| {
                &self.transaction.message.account_keys[ix.program_id_index as usize] == expected
            });

        if !has_program {
            self.errors.push(format!(
                "Transaction does not use program: {}",
                expected
            ));
        }
        self
    }

    /// Validate instruction count
    pub fn validate_instruction_count(mut self, expected: usize) -> Self {
        let actual = self.transaction.message.instructions.len();
        if actual != expected {
            self.errors.push(format!(
                "Expected {} instructions, found {}",
                expected, actual
            ));
        }
        self
    }

    /// Validate that an account is a signer
    pub fn validate_signer(mut self, pubkey: &Pubkey) -> Self {
        let is_signer = self
            .transaction
            .message
            .account_keys
            .iter()
            .position(|k| k == pubkey)
            .map(|index| self.transaction.message.is_signer(index))
            .unwrap_or(false);

        if !is_signer {
            self.errors
                .push(format!("Account {} is not a signer", pubkey));
        }
        self
    }

    /// Validate that the transaction is signed
    pub fn validate_signed(mut self) -> Self {
        if self.transaction.signatures.is_empty() {
            self.errors.push("Transaction has no signatures".to_string());
        }
        self
    }

    /// Validate that an account is writable
    pub fn validate_writable(mut self, pubkey: &Pubkey) -> Self {
        let is_writable = self
            .transaction
            .message
            .account_keys
            .iter()
            .position(|k| k == pubkey)
            .map(|index| self.transaction.message.is_maybe_writable(index, None))
            .unwrap_or(false);

        if !is_writable {
            self.errors
                .push(format!("Account {} is not writable", pubkey));
        }
        self
    }

    /// Validate that an account is present
    pub fn validate_account_present(mut self, pubkey: &Pubkey) -> Self {
        if !self.transaction.message.account_keys.contains(pubkey) {
            self.errors.push(format!(
                "Account {} is not present in transaction",
                pubkey
            ));
        }
        self
    }

    /// Validate transaction metadata (account counts, etc.)
    pub fn validate_metadata(mut self, expected: TransactionMetadata) -> Self {
        let header = &self.transaction.message.header;

        if let Some(num_required_signatures) = expected.num_required_signatures {
            if header.num_required_signatures != num_required_signatures {
                self.errors.push(format!(
                    "Expected {} required signatures, found {}",
                    num_required_signatures, header.num_required_signatures
                ));
            }
        }

        if let Some(num_readonly_signed_accounts) = expected.num_readonly_signed_accounts {
            if header.num_readonly_signed_accounts != num_readonly_signed_accounts {
                self.errors.push(format!(
                    "Expected {} readonly signed accounts, found {}",
                    num_readonly_signed_accounts, header.num_readonly_signed_accounts
                ));
            }
        }

        if let Some(num_readonly_unsigned_accounts) = expected.num_readonly_unsigned_accounts {
            if header.num_readonly_unsigned_accounts != num_readonly_unsigned_accounts {
                self.errors.push(format!(
                    "Expected {} readonly unsigned accounts, found {}",
                    num_readonly_unsigned_accounts, header.num_readonly_unsigned_accounts
                ));
            }
        }

        self
    }

    /// Get the transaction being validated
    pub fn transaction(&self) -> &Transaction {
        self.transaction
    }

    /// Get accumulated errors
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Check if validation passed (no errors)
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Complete validation, returning error if any validations failed
    pub fn complete(self) -> Result<()> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Transaction validation failed:\n{}",
                self.errors.join("\n")
            ))
        }
    }
}

/// Expected transaction metadata for validation
#[derive(Debug, Default)]
pub struct TransactionMetadata {
    pub num_required_signatures: Option<u8>,
    pub num_readonly_signed_accounts: Option<u8>,
    pub num_readonly_unsigned_accounts: Option<u8>,
}

/// Validate parsed instruction fields
#[allow(dead_code)]
pub fn validate_instruction_fields(
    actual: &HashMap<String, serde_json::Value>,
    expected: &HashMap<String, serde_json::Value>,
) -> Result<()> {
    let mut errors = Vec::new();

    for (key, expected_value) in expected {
        match actual.get(key) {
            Some(actual_value) => {
                if actual_value != expected_value {
                    errors.push(format!(
                        "Field '{}': expected {:?}, got {:?}",
                        key, expected_value, actual_value
                    ));
                }
            }
            None => {
                errors.push(format!("Field '{}' not found in parsed output", key));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Instruction field validation failed:\n{}",
            errors.join("\n")
        ))
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
    fn test_validator_success() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let instruction = system_instruction::transfer(&payer.pubkey(), &to.pubkey(), 1_000_000);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[&payer], Hash::default());

        let result = TransactionValidator::new(&transaction)
            .validate_signed()
            .validate_instruction_count(1)
            .validate_signer(&payer.pubkey())
            .complete();

        assert!(result.is_ok());
    }

    #[test]
    fn test_validator_failure() {
        let payer = Keypair::new();
        let to = Keypair::new();

        let instruction = system_instruction::transfer(&payer.pubkey(), &to.pubkey(), 1_000_000);

        let message = Message::new(&[instruction], Some(&payer.pubkey()));
        let mut transaction = Transaction::new_unsigned(message);
        transaction.sign(&[&payer], Hash::default());

        let result = TransactionValidator::new(&transaction)
            .validate_instruction_count(2) // Wrong count
            .validate_signer(&to.pubkey()) // Not a signer
            .complete();

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("Expected 2 instructions"));
        assert!(err_msg.contains("is not a signer"));
    }

    #[test]
    fn test_validate_instruction_fields() {
        let mut actual = HashMap::new();
        actual.insert("amount".to_string(), serde_json::json!("1000000"));
        actual.insert("token".to_string(), serde_json::json!("SOL"));

        let mut expected = HashMap::new();
        expected.insert("amount".to_string(), serde_json::json!("1000000"));
        expected.insert("token".to_string(), serde_json::json!("SOL"));

        let result = validate_instruction_fields(&actual, &expected);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_instruction_fields_mismatch() {
        let mut actual = HashMap::new();
        actual.insert("amount".to_string(), serde_json::json!("2000000"));

        let mut expected = HashMap::new();
        expected.insert("amount".to_string(), serde_json::json!("1000000"));
        expected.insert("token".to_string(), serde_json::json!("SOL"));

        let result = validate_instruction_fields(&actual, &expected);
        assert!(result.is_err());
    }
}
