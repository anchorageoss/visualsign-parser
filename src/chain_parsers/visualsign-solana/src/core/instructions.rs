use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use crate::idl::IdlRegistry;
use solana_parser::solana::parser::parse_transaction;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::Instruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::{TransactionParseError, VisualSignError};
use visualsign::field_builders::create_diagnostic_field;

// The following include! macro pulls in visualizer implementations generated at build time.
// The file "generated_visualizers.rs" is created by the build script and contains code for
// available_visualizers and related items, which are used to decode and visualize instructions.
include!(concat!(env!("OUT_DIR"), "/generated_visualizers.rs"));

/// Visualizes all the instructions and related fields in a transaction/message
pub fn decode_instructions(
    transaction: &SolanaTransaction,
    idl_registry: &IdlRegistry,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    // available_visualizers is generated at build time by build.rs
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    let message = &transaction.message;
    let account_keys = &message.account_keys;

    if account_keys.is_empty() {
        return Err(VisualSignError::ParseError(
            TransactionParseError::DecodeError(
                "Legacy transaction has no account keys".to_string(),
            ),
        ));
    }

    // Convert compiled instructions to full instructions, emitting diagnostics
    // for out-of-bounds indices instead of silently dropping them.
    let mut instructions: Vec<Instruction> = Vec::new();
    let mut diagnostics: Vec<AnnotatedPayloadField> = Vec::new();

    for (ci_index, ci) in message.instructions.iter().enumerate() {
        if (ci.program_id_index as usize) >= account_keys.len() {
            diagnostics.push(create_diagnostic_field(
                "transaction::oob_program_id",
                "transaction",
                "warn",
                &format!(
                    "instruction {} skipped: program_id_index {} out of bounds ({} accounts)",
                    ci_index,
                    ci.program_id_index,
                    account_keys.len()
                ),
                Some(ci_index as u32),
            ));
            continue;
        }

        let mut oob_account_indices: Vec<u8> = Vec::new();
        let accounts: Vec<solana_sdk::instruction::AccountMeta> = ci
            .accounts
            .iter()
            .filter_map(|&i| {
                if (i as usize) < account_keys.len() {
                    Some(solana_sdk::instruction::AccountMeta::new_readonly(
                        account_keys[i as usize],
                        false,
                    ))
                } else {
                    oob_account_indices.push(i);
                    None
                }
            })
            .collect();

        if !oob_account_indices.is_empty() {
            diagnostics.push(create_diagnostic_field(
                "transaction::oob_account_index",
                "transaction",
                "warn",
                &format!(
                    "instruction {}: account indices {:?} out of bounds ({} accounts)",
                    ci_index,
                    oob_account_indices,
                    account_keys.len()
                ),
                Some(ci_index as u32),
            ));
        }

        instructions.push(Instruction {
            program_id: account_keys[ci.program_id_index as usize],
            accounts,
            data: ci.data.clone(),
        });
    }

    let results: Result<Vec<AnnotatedPayloadField>, VisualSignError> = instructions
        .iter()
        .enumerate()
        .map(|(instruction_index, instruction)| {
            // Create sender account from first account key (typically the fee payer)
            let sender = SolanaAccount {
                account_key: account_keys[0].to_string(),
                signer: false,
                writable: false,
            };

            let context =
                VisualizerContext::new(&sender, instruction_index, &instructions, idl_registry);

            // Try to visualize with available visualizers (including unknown_program fallback)
            visualize_with_any(&visualizers_refs, &context)
                .ok_or_else(|| {
                    VisualSignError::DecodeError(format!(
                        "No visualizer available for instruction {} at index {}",
                        instruction.program_id, instruction_index
                    ))
                })?
                .map(|viz_result| viz_result.field)
        })
        .collect();

    let mut fields = results?;

    // Self-check: ensure we have the same number of instruction fields as input instructions
    if fields.len() != instructions.len() {
        return Err(VisualSignError::InvariantViolation(format!(
            "Instruction count mismatch: expected {} instructions, got {} fields. This should never happen with unknown_program fallback.",
            instructions.len(),
            fields.len()
        )));
    }

    // Append diagnostics after instruction fields
    fields.extend(diagnostics);

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::{Message, MessageHeader};
    use solana_sdk::pubkey::Pubkey;
    use visualsign::SignablePayloadField;

    /// Build a legacy SolanaTransaction with a manually crafted message
    /// that has an OOB program_id_index.
    fn tx_with_oob_program_id() -> SolanaTransaction {
        let key0 = Pubkey::new_unique(); // fee payer
        let key1 = Pubkey::new_unique(); // valid account
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: Hash::default(),
            instructions: vec![
                // Valid instruction: program_id_index=1, within bounds
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![0],
                    data: vec![0xAA],
                },
                // OOB instruction: program_id_index=99, way out of bounds
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 99,
                    accounts: vec![0],
                    data: vec![0xBB],
                },
            ],
        };
        SolanaTransaction {
            signatures: vec![],
            message,
        }
    }

    /// Build a transaction where an instruction has OOB account indices
    /// but a valid program_id_index.
    fn tx_with_oob_account_index() -> SolanaTransaction {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: Hash::default(),
            instructions: vec![solana_sdk::instruction::CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0, 50], // index 50 is OOB
                data: vec![0xCC],
            }],
        };
        SolanaTransaction {
            signatures: vec![],
            message,
        }
    }

    #[test]
    fn test_oob_program_id_emits_diagnostic() {
        let tx = tx_with_oob_program_id();
        let registry = IdlRegistry::new();
        let fields = decode_instructions(&tx, &registry).expect("should not error");

        // Should have 1 instruction field + 1 diagnostic
        let diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() == "diagnostic")
            .collect();
        assert_eq!(diagnostics.len(), 1, "expected one diagnostic for OOB program_id");

        match &diagnostics[0].signable_payload_field {
            SignablePayloadField::Diagnostic { diagnostic, .. } => {
                assert_eq!(diagnostic.rule, "transaction::oob_program_id");
                assert_eq!(diagnostic.domain, "transaction");
                assert_eq!(diagnostic.level, "warn");
                assert_eq!(diagnostic.instruction_index, Some(1));
            }
            _ => panic!("expected Diagnostic variant"),
        }

        // The valid instruction should still be present
        let non_diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() != "diagnostic")
            .collect();
        assert_eq!(non_diagnostics.len(), 1, "expected one valid instruction field");
    }

    #[test]
    fn test_oob_account_index_emits_diagnostic() {
        let tx = tx_with_oob_account_index();
        let registry = IdlRegistry::new();
        let fields = decode_instructions(&tx, &registry).expect("should not error");

        let diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() == "diagnostic")
            .collect();
        assert_eq!(diagnostics.len(), 1, "expected one diagnostic for OOB account index");

        match &diagnostics[0].signable_payload_field {
            SignablePayloadField::Diagnostic { diagnostic, .. } => {
                assert_eq!(diagnostic.rule, "transaction::oob_account_index");
                assert_eq!(diagnostic.domain, "transaction");
                assert_eq!(diagnostic.level, "warn");
                assert_eq!(diagnostic.instruction_index, Some(0));
                assert!(diagnostic.message.contains("50"));
            }
            _ => panic!("expected Diagnostic variant"),
        }

        // The instruction should still be decoded (with the valid account only)
        let non_diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() != "diagnostic")
            .collect();
        assert_eq!(non_diagnostics.len(), 1, "instruction should still be present");
    }

    #[test]
    fn test_no_diagnostics_for_valid_transaction() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: Hash::default(),
            instructions: vec![solana_sdk::instruction::CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0],
                data: vec![0xDD],
            }],
        };
        let tx = SolanaTransaction {
            signatures: vec![],
            message,
        };
        let registry = IdlRegistry::new();
        let fields = decode_instructions(&tx, &registry).expect("should not error");

        let diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() == "diagnostic")
            .collect();
        assert!(diagnostics.is_empty(), "valid transaction should produce no diagnostics");
    }
}

pub fn decode_transfers(
    transaction: &SolanaTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let message_clone = transaction.message.clone();
    let parsed_transaction = parse_transaction(
        hex::encode(message_clone.serialize()),
        false, /* because we're passing the message only */
        None,  // No custom IDLs for transfer parsing
    )
    .map_err(|e| {
        VisualSignError::ParseError(TransactionParseError::DecodeError(format!(
            "Failed to parse transaction: {e}"
        )))
    })?;

    let mut fields = Vec::new();

    // Extract native SOL transfers
    if let Some(payload) = parsed_transaction
        .solana_parsed_transaction
        .payload
        .as_ref()
    {
        if let Some(transaction_metadata) = payload.transaction_metadata.as_ref() {
            // Add native SOL transfers
            for (i, transfer) in transaction_metadata.transfers.iter().enumerate() {
                // Create the field using the old format for compatibility
                let field = AnnotatedPayloadField {
                    signable_payload_field: visualsign::SignablePayloadField::TextV2 {
                        common: visualsign::SignablePayloadFieldCommon {
                            fallback_text: format!(
                                "Transfer {}: From {} To {} For {}",
                                i + 1,
                                transfer.from,
                                transfer.to,
                                transfer.amount
                            ),
                            label: format!("Transfer {}", i + 1),
                        },
                        text_v2: visualsign::SignablePayloadFieldTextV2 {
                            text: format!(
                                "From: {}\nTo: {}\nAmount: {}",
                                transfer.from, transfer.to, transfer.amount
                            ),
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                };

                fields.push(field);
            }

            // Add SPL token transfers
            for (i, spl_transfer) in transaction_metadata.spl_transfers.iter().enumerate() {
                let field = AnnotatedPayloadField {
                    signable_payload_field: visualsign::SignablePayloadField::TextV2 {
                        common: visualsign::SignablePayloadFieldCommon {
                            fallback_text: format!(
                                "SPL Transfer {}: From {} To {} For {}",
                                i + 1,
                                spl_transfer.from,
                                spl_transfer.to,
                                spl_transfer.amount
                            ),
                            label: format!("SPL Transfer {}", i + 1),
                        },
                        text_v2: visualsign::SignablePayloadFieldTextV2 {
                            text: format!(
                                "From: {}\nTo: {}\nOwner: {}\nAmount: {}\nMint: {:?}\nDecimals: {:?}\nFee: {:?}",
                                spl_transfer.from,
                                spl_transfer.to,
                                spl_transfer.owner,
                                spl_transfer.amount,
                                spl_transfer.token_mint,
                                spl_transfer.decimals,
                                spl_transfer.fee
                            ),
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                };

                fields.push(field);
            }
        }
    }

    Ok(fields)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::{Message, MessageHeader};
    use solana_sdk::pubkey::Pubkey;

    /// Verifies that empty account keys returns a DecodeError (not a panic).
    /// This error path was introduced in PR #245 replacing the previous unwrap.
    #[test]
    fn test_empty_account_keys_returns_parse_error() {
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 0,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![],
            recent_blockhash: Hash::default(),
            instructions: vec![],
        };
        let tx = SolanaTransaction {
            signatures: vec![],
            message,
        };
        let registry = IdlRegistry::new();
        let result = decode_instructions(&tx, &registry);

        assert!(
            matches!(
                &result,
                Err(VisualSignError::ParseError(TransactionParseError::DecodeError(msg)))
                    if msg.contains("no account keys")
            ),
            "expected DecodeError for empty account keys, got: {result:?}"
        );
    }

    /// Verifies that OOB program_id_index silently skips the instruction
    /// instead of panicking. Only the valid instruction produces a field.
    #[test]
    fn test_oob_program_id_index_skips_instruction() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: Hash::default(),
            instructions: vec![
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![0],
                    data: vec![0xAA],
                },
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 99,
                    accounts: vec![0],
                    data: vec![0xBB],
                },
            ],
        };
        let tx = SolanaTransaction {
            signatures: vec![],
            message,
        };
        let registry = IdlRegistry::new();
        let result = decode_instructions(&tx, &registry);

        let fields = result.expect("should not error when OOB instructions are skipped");
        assert_eq!(fields.len(), 1, "expected 1 field for 1 valid instruction");
    }

    /// Verifies that all-OOB instructions produce empty fields (not a panic).
    #[test]
    fn test_all_instructions_oob_returns_empty_fields() {
        let key0 = Pubkey::new_unique();
        let message = Message {
            header: MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0],
            recent_blockhash: Hash::default(),
            instructions: vec![
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 99,
                    accounts: vec![],
                    data: vec![],
                },
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 88,
                    accounts: vec![],
                    data: vec![],
                },
            ],
        };
        let tx = SolanaTransaction {
            signatures: vec![],
            message,
        };
        let registry = IdlRegistry::new();
        let result = decode_instructions(&tx, &registry);

        let fields = result.expect("should succeed with all OOB instructions");
        assert!(
            fields.is_empty(),
            "expected no fields when all instructions are OOB"
        );
    }
}
