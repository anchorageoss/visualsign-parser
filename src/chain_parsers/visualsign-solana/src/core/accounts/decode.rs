use solana_sdk::message::Message;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    errors::VisualSignError,
};

/// Represents an account in a Solana transaction with its properties
#[derive(Debug, Clone, PartialEq)]
pub struct SolanaAccountInfo {
    pub address: String,
    pub is_signer: bool,
    pub is_writable: bool,
    pub original_index: usize,
}

/// Decode accounts from a Solana transaction message and return them sorted by importance
/// (signers first, then signer+writable, then everything else)
pub fn decode_accounts(message: &Message) -> Result<Vec<SolanaAccountInfo>, VisualSignError> {
    let mut accounts: Vec<SolanaAccountInfo> = message
        .account_keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let is_signer = i < message.header.num_required_signatures as usize;
            let is_writable = if i < message.header.num_required_signatures as usize {
                // For signers: readonly ones come at the end of the signer range
                let readonly_signer_start = message.header.num_required_signatures as usize
                    - message.header.num_readonly_signed_accounts as usize;
                i < readonly_signer_start
            } else {
                // For non-signers: readonly ones come at the end of the non-signer range
                let non_signer_index = i - message.header.num_required_signatures as usize;
                let total_non_signers =
                    message.account_keys.len() - message.header.num_required_signatures as usize;
                let writable_non_signers =
                    total_non_signers - message.header.num_readonly_unsigned_accounts as usize;
                non_signer_index < writable_non_signers
            };

            SolanaAccountInfo {
                address: key.to_string(),
                is_signer,
                is_writable,
                original_index: i,
            }
        })
        .collect();

    // Sort according to Solana specification:
    // 1. Accounts that are writable and signers
    // 2. Accounts that are read-only and signers
    // 3. Accounts that are writable and not signers
    // 4. Accounts that are read-only and not signers
    accounts.sort_by(|a, b| {
        let a_category = if a.is_signer && a.is_writable {
            0 // writable signers
        } else if a.is_signer && !a.is_writable {
            1 // readonly signers
        } else if !a.is_signer && a.is_writable {
            2 // writable non-signers
        } else {
            3 // readonly non-signers
        };

        let b_category = if b.is_signer && b.is_writable {
            0 // writable signers
        } else if b.is_signer && !b.is_writable {
            1 // readonly signers
        } else if !b.is_signer && b.is_writable {
            2 // writable non-signers
        } else {
            3 // readonly non-signers
        };

        match a_category.cmp(&b_category) {
            std::cmp::Ordering::Equal => a.original_index.cmp(&b.original_index),
            other => other,
        }
    });

    Ok(accounts)
}

/// Decode accounts from a V0 message
pub fn decode_v0_accounts(
    v0_message: &solana_sdk::message::v0::Message,
) -> Result<Vec<SolanaAccountInfo>, VisualSignError> {
    let mut accounts: Vec<SolanaAccountInfo> = v0_message
        .account_keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            // V0 message header is same as legacy
            let is_signer = i < v0_message.header.num_required_signatures as usize;
            let is_writable = if i < v0_message.header.num_required_signatures as usize {
                // For signers: readonly ones come at the end of the signer range
                let readonly_signer_start = v0_message.header.num_required_signatures as usize
                    - v0_message.header.num_readonly_signed_accounts as usize;
                i < readonly_signer_start
            } else {
                // For non-signers: readonly ones come at the end of the non-signer range
                let non_signer_index = i - v0_message.header.num_required_signatures as usize;
                let total_non_signers = v0_message.account_keys.len()
                    - v0_message.header.num_required_signatures as usize;
                let writable_non_signers =
                    total_non_signers - v0_message.header.num_readonly_unsigned_accounts as usize;
                non_signer_index < writable_non_signers
            };

            SolanaAccountInfo {
                address: key.to_string(),
                is_signer,
                is_writable,
                original_index: i,
            }
        })
        .collect();

    // Sort according to Solana specification:
    // 1. Accounts that are writable and signers
    // 2. Accounts that are read-only and signers
    // 3. Accounts that are writable and not signers
    // 4. Accounts that are read-only and not signers
    accounts.sort_by(|a, b| {
        let a_category = if a.is_signer && a.is_writable {
            0 // writable signers
        } else if a.is_signer && !a.is_writable {
            1 // readonly signers
        } else if !a.is_signer && a.is_writable {
            2 // writable non-signers
        } else {
            3 // readonly non-signers
        };

        let b_category = if b.is_signer && b.is_writable {
            0 // writable signers
        } else if b.is_signer && !b.is_writable {
            1 // readonly signers
        } else if !b.is_signer && b.is_writable {
            2 // writable non-signers
        } else {
            3 // readonly non-signers
        };

        match a_category.cmp(&b_category) {
            std::cmp::Ordering::Equal => a.original_index.cmp(&b.original_index),
            other => other,
        }
    });

    Ok(accounts)
}

/// Convert accounts to AnnotatedPayloadField format for the SignablePayload
pub fn accounts_to_payload_fields(accounts: &[SolanaAccountInfo]) -> Vec<AnnotatedPayloadField> {
    accounts
        .iter()
        .map(|account| {
            let mut details = vec![format!("Address: {}", account.address)];
            if account.is_signer {
                details.push("Signer".to_string());
            }
            if account.is_writable {
                details.push("Writable".to_string());
            }

            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: details.join(", "),
                        label: "Account".to_string(),
                    },
                    text_v2: visualsign::SignablePayloadFieldTextV2 {
                        text: details.join(", "),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{hash::Hash, message::MessageHeader, pubkey::Pubkey};

    fn create_test_message(
        num_required_signatures: u8,
        num_readonly_signed_accounts: u8,
        num_readonly_unsigned_accounts: u8,
        account_keys: Vec<Pubkey>,
    ) -> Message {
        Message {
            header: MessageHeader {
                num_required_signatures,
                num_readonly_signed_accounts,
                num_readonly_unsigned_accounts,
            },
            account_keys,
            recent_blockhash: Hash::new_unique(),
            instructions: vec![],
        }
    }

    #[test]
    fn test_decode_accounts_basic() {
        let account1 = Pubkey::new_unique();
        let account2 = Pubkey::new_unique();
        let account3 = Pubkey::new_unique();

        // Create a message with 2 signers, 1 readonly signed, 1 readonly unsigned
        let message = create_test_message(
            2, // num_required_signatures
            1, // num_readonly_signed_accounts
            1, // num_readonly_unsigned_accounts
            vec![account1, account2, account3],
        );

        let accounts = decode_accounts(&message).unwrap();

        assert_eq!(accounts.len(), 3);

        // First account: signer + writable (index 0, signer, not readonly)
        assert_eq!(accounts[0].address, account1.to_string());
        assert!(accounts[0].is_signer);
        assert!(accounts[0].is_writable);
        assert_eq!(accounts[0].original_index, 0);

        // Second account: signer + readonly (index 1, signer, readonly signed)
        assert_eq!(accounts[1].address, account2.to_string());
        assert!(accounts[1].is_signer);
        assert!(!accounts[1].is_writable);
        assert_eq!(accounts[1].original_index, 1);

        // Third account: non-signer + readonly (index 2, not signer, readonly unsigned)
        assert_eq!(accounts[2].address, account3.to_string());
        assert!(!accounts[2].is_signer);
        assert!(!accounts[2].is_writable);
        assert_eq!(accounts[2].original_index, 2);
    }

    #[test]
    fn test_account_sorting() {
        let account1 = Pubkey::new_unique(); // index 0: signer + writable
        let account2 = Pubkey::new_unique(); // index 1: signer + readonly  
        let account3 = Pubkey::new_unique(); // index 2: non-signer + writable
        let account4 = Pubkey::new_unique(); // index 3: non-signer + readonly

        // Create message: 2 signers (indices 0,1), 1 readonly signed (index 1), 1 readonly unsigned (index 3)
        // This means: index 0 = signer writable, index 1 = signer readonly, index 2 = non-signer writable, index 3 = non-signer readonly
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 2,
                num_readonly_signed_accounts: 1,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![account1, account2, account3, account4],
            recent_blockhash: Hash::new_unique(),
            instructions: vec![],
        };

        let accounts = decode_accounts(&message).unwrap();

        // Should be sorted according to Solana spec:
        // 1. Writable signers, 2. Readonly signers, 3. Writable non-signers, 4. Readonly non-signers
        // Expected order matches original since accounts are already in correct Solana spec order

        // First account should be signer+writable (original index 0)
        assert!(accounts[0].is_signer);
        assert!(accounts[0].is_writable);
        assert_eq!(accounts[0].original_index, 0);

        // Second account should be signer+readonly (original index 1)
        assert!(accounts[1].is_signer);
        assert!(!accounts[1].is_writable);
        assert_eq!(accounts[1].original_index, 1);

        // Third account should be non-signer+writable (original index 2)
        assert!(!accounts[2].is_signer);
        assert!(accounts[2].is_writable);
        assert_eq!(accounts[2].original_index, 2);

        // Fourth account should be non-signer+readonly (original index 3)
        assert!(!accounts[3].is_signer);
        assert!(!accounts[3].is_writable);
        assert_eq!(accounts[3].original_index, 3);
    }

    #[test]
    fn test_signer_writable_priority() {
        let account1 = Pubkey::new_unique(); // signer, writable (index 0)
        let account2 = Pubkey::new_unique(); // signer, readonly (index 1)

        // With num_readonly_signed_accounts: 1, the last signer (index 1) is readonly
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 2,
                num_readonly_signed_accounts: 1, // last signer is readonly
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![account1, account2],
            recent_blockhash: Hash::new_unique(),
            instructions: vec![],
        };

        let accounts = decode_accounts(&message).unwrap();

        // Signer+writable should come before signer+readonly
        assert!(accounts[0].is_signer);
        assert!(accounts[0].is_writable);
        assert_eq!(accounts[0].original_index, 0); // first account (writable signer)

        assert!(accounts[1].is_signer);
        assert!(!accounts[1].is_writable);
        assert_eq!(accounts[1].original_index, 1); // second account (readonly signer)
    }

    #[test]
    fn test_decode_all_account_categories() {
        // Test decode_accounts with all 4 categories of accounts in proper Solana spec order
        let writable_signer = Pubkey::new_unique(); // index 0
        let readonly_signer = Pubkey::new_unique(); // index 1
        let writable_non_signer = Pubkey::new_unique(); // index 2
        let readonly_non_signer = Pubkey::new_unique(); // index 3

        // Create message following Solana spec:
        // - First 2 accounts are signers (indices 0-1)
        // - Last signer (index 1) is readonly (num_readonly_signed_accounts: 1)
        // - Last non-signer (index 3) is readonly (num_readonly_unsigned_accounts: 1)
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 2,
                num_readonly_signed_accounts: 1,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![
                writable_signer,
                readonly_signer,
                writable_non_signer,
                readonly_non_signer,
            ],
            recent_blockhash: Hash::new_unique(),
            instructions: vec![],
        };

        let accounts = decode_accounts(&message).unwrap();

        assert_eq!(accounts.len(), 4);

        // Verify accounts are decoded and ordered correctly per Solana spec:
        // 1. Writable signers
        assert_eq!(accounts[0].address, writable_signer.to_string());
        assert!(accounts[0].is_signer);
        assert!(accounts[0].is_writable);
        assert_eq!(accounts[0].original_index, 0);

        // 2. Readonly signers
        assert_eq!(accounts[1].address, readonly_signer.to_string());
        assert!(accounts[1].is_signer);
        assert!(!accounts[1].is_writable);
        assert_eq!(accounts[1].original_index, 1);

        // 3. Writable non-signers
        assert_eq!(accounts[2].address, writable_non_signer.to_string());
        assert!(!accounts[2].is_signer);
        assert!(accounts[2].is_writable);
        assert_eq!(accounts[2].original_index, 2);

        // 4. Readonly non-signers
        assert_eq!(accounts[3].address, readonly_non_signer.to_string());
        assert!(!accounts[3].is_signer);
        assert!(!accounts[3].is_writable);
        assert_eq!(accounts[3].original_index, 3);
    }

    #[test]
    fn test_accounts_to_payload_fields() {
        let accounts = vec![
            SolanaAccountInfo {
                address: "11111111111111111111111111111112".to_string(),
                is_signer: true,
                is_writable: true,
                original_index: 0,
            },
            SolanaAccountInfo {
                address: "11111111111111111111111111111113".to_string(),
                is_signer: false,
                is_writable: false,
                original_index: 1,
            },
        ];

        let payload_fields = accounts_to_payload_fields(&accounts);

        assert_eq!(payload_fields.len(), 2);

        // First field should be signer+writable
        match &payload_fields[0].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Account");
                assert!(
                    common
                        .fallback_text
                        .contains("Address: 11111111111111111111111111111112")
                );
                assert!(common.fallback_text.contains("Signer"));
                assert!(common.fallback_text.contains("Writable"));
                assert_eq!(text_v2.text, common.fallback_text);
            }
            _ => panic!("Expected TextV2 field"),
        }

        // Second field should be non-signer+readonly
        match &payload_fields[1].signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Account");
                assert!(
                    common
                        .fallback_text
                        .contains("Address: 11111111111111111111111111111113")
                );
                assert!(!common.fallback_text.contains("Signer"));
                assert!(!common.fallback_text.contains("Writable"));
                assert_eq!(text_v2.text, common.fallback_text);
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_v0_accounts() {
        use solana_sdk::message::{MessageHeader, v0::Message as V0Message};

        let account1 = Pubkey::new_unique();
        let account2 = Pubkey::new_unique();

        let v0_message = V0Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![account1, account2],
            recent_blockhash: Hash::new_unique(),
            instructions: vec![],
            address_table_lookups: vec![],
        };

        let accounts = decode_v0_accounts(&v0_message).unwrap();

        assert_eq!(accounts.len(), 2);

        // First should be signer+writable
        assert!(accounts[0].is_signer);
        assert!(accounts[0].is_writable);
        assert_eq!(accounts[0].original_index, 0);

        // Second should be non-signer+readonly
        assert!(!accounts[1].is_signer);
        assert!(!accounts[1].is_writable);
        assert_eq!(accounts[1].original_index, 1);
    }
}
