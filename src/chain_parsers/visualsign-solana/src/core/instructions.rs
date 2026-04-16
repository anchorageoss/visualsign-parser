use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use crate::idl::IdlRegistry;
use solana_parser::solana::parser::parse_transaction;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::{TransactionParseError, VisualSignError};
use visualsign::field_builders::create_diagnostic_field;
use visualsign::lint::LintConfig;

// The following include! macro pulls in visualizer implementations generated at build time.
// The file "generated_visualizers.rs" is created by the build script and contains code for
// available_visualizers and related items, which are used to decode and visualize instructions.
include!(concat!(env!("OUT_DIR"), "/generated_visualizers.rs"));

/// Result of decoding instructions: display fields, per-instruction errors,
/// and lint diagnostics separately. The function always succeeds — individual
/// instruction failures are captured in `errors` rather than aborting the parse.
pub struct DecodeInstructionsResult {
    pub fields: Vec<AnnotatedPayloadField>,
    pub errors: Vec<(usize, VisualSignError)>,
    pub diagnostics: Vec<AnnotatedPayloadField>,
}

/// Visualizes all the instructions and related fields in a transaction/message.
/// Always succeeds — data quality issues become diagnostics, per-instruction
/// failures are collected in errors.
pub fn decode_instructions(
    transaction: &SolanaTransaction,
    idl_registry: &IdlRegistry,
    lint_config: &LintConfig,
) -> DecodeInstructionsResult {
    // available_visualizers is generated at build time by build.rs
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    let message = &transaction.message;
    let account_keys = &message.account_keys;

    if account_keys.is_empty() {
        return DecodeInstructionsResult {
            fields: Vec::new(),
            errors: Vec::new(),
            diagnostics: vec![create_diagnostic_field(
                "transaction::empty_account_keys",
                "transaction",
                visualsign::lint::Severity::Error,
                "legacy transaction has no account keys",
                None,
            )],
        };
    }

    // Diagnostic scan: check all indices, emit diagnostics for inaccessible ones.
    // This is purely informational — no instructions are skipped.
    let diagnostics =
        scan_instruction_diagnostics(&message.instructions, account_keys, lint_config);

    // Visualization: process every instruction (no skipping)
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();
    let mut errors: Vec<(usize, VisualSignError)> = Vec::new();

    for (i, ci) in message.instructions.iter().enumerate() {
        let sender = SolanaAccount {
            account_key: account_keys[0].to_string(),
            signer: false,
            writable: false,
        };

        let context = VisualizerContext::new(&sender, ci, account_keys, idl_registry);

        match visualize_with_any(&visualizers_refs, &context) {
            Some(Ok(viz_result)) => fields.push(viz_result.field),
            Some(Err(e)) => errors.push((i, e)),
            None => errors.push((
                i,
                VisualSignError::DecodeError(format!(
                    "No visualizer available for instruction at index {i}"
                )),
            )),
        }
    }

    DecodeInstructionsResult {
        fields,
        errors,
        diagnostics,
    }
}

/// Scan compiled instructions for inaccessible indices and emit diagnostics.
/// Does not modify or filter instructions — purely informational.
pub fn scan_instruction_diagnostics(
    instructions: &[solana_sdk::instruction::CompiledInstruction],
    account_keys: &[solana_sdk::pubkey::Pubkey],
    lint_config: &LintConfig,
) -> Vec<AnnotatedPayloadField> {
    let mut diagnostics: Vec<AnnotatedPayloadField> = Vec::new();
    let mut oob_program_id_count: usize = 0;
    let mut oob_account_index_count: usize = 0;

    let oob_pid_severity = lint_config.severity_for(
        "transaction::oob_program_id",
        visualsign::lint::Severity::Warn,
    );
    let oob_acct_severity = lint_config.severity_for(
        "transaction::oob_account_index",
        visualsign::lint::Severity::Warn,
    );

    for (ci_index, ci) in instructions.iter().enumerate() {
        if (ci.program_id_index as usize) >= account_keys.len() {
            oob_program_id_count += 1;
            if !matches!(oob_pid_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_program_id",
                    "transaction",
                    oob_pid_severity.clone(),
                    &format!(
                        "instruction {}: program_id_index {} out of bounds ({} account keys)",
                        ci_index,
                        ci.program_id_index,
                        account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
        }

        // Check all account indices (unified — no separate "skipped" rule)
        let oob_accounts: Vec<u8> = ci
            .accounts
            .iter()
            .filter(|&&idx| (idx as usize) >= account_keys.len())
            .copied()
            .collect();
        if !oob_accounts.is_empty() {
            oob_account_index_count += 1;
            if !matches!(oob_acct_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_account_index",
                    "transaction",
                    oob_acct_severity.clone(),
                    &format!(
                        "instruction {}: account indices {:?} out of bounds ({} account keys)",
                        ci_index,
                        oob_accounts,
                        account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
        }
    }

    // Boot-metric ok diagnostics
    if oob_program_id_count == 0 && lint_config.should_report_ok("transaction::oob_program_id") {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_program_id",
            "transaction",
            visualsign::lint::Severity::Ok,
            &format!(
                "all {} instructions have valid program_id_index",
                instructions.len()
            ),
            None,
        ));
    }
    if oob_account_index_count == 0
        && lint_config.should_report_ok("transaction::oob_account_index")
    {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_account_index",
            "transaction",
            visualsign::lint::Severity::Ok,
            &format!(
                "all {} instructions have valid account indices",
                instructions.len()
            ),
            None,
        ));
    }

    diagnostics
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
mod tests {
    use super::*;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::{Message, MessageHeader};
    use solana_sdk::pubkey::Pubkey;
    use visualsign::SignablePayloadField;

    fn tx_with_oob_program_id() -> SolanaTransaction {
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
        SolanaTransaction {
            signatures: vec![],
            message,
        }
    }

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
                accounts: vec![0, 50],
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
        let config = LintConfig::default();
        let result = decode_instructions(&tx, &registry, &config);
        let fields = [result.fields, result.diagnostics].concat();

        let warns: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. }
                    if diagnostic.level == "warn" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].rule, "transaction::oob_program_id");
        assert_eq!(warns[0].instruction_index, Some(1));

        // oob_account_index should pass since the instruction's accounts are valid
        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index")
        );
    }

    #[test]
    fn test_oob_account_index_emits_diagnostic() {
        let tx = tx_with_oob_account_index();
        let registry = IdlRegistry::new();
        let config = LintConfig::default();
        let result = decode_instructions(&tx, &registry, &config);
        let fields = [result.fields, result.diagnostics].concat();

        let warns: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. }
                    if diagnostic.level == "warn" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].rule, "transaction::oob_account_index");
        assert_eq!(warns[0].instruction_index, Some(0));
        assert!(warns[0].message.contains("50"));

        // oob_program_id should pass since the instruction has a valid program_id_index
        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_program_id")
        );

        let non_diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() != "diagnostic")
            .collect();
        assert_eq!(non_diagnostics.len(), 1);
    }

    #[test]
    fn test_valid_transaction_emits_pass_diagnostics() {
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
        let config = LintConfig::default();
        let result = decode_instructions(&tx, &registry, &config);
        let fields = [result.fields, result.diagnostics].concat();

        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        // Both rules should report ok
        assert_eq!(passes.len(), 2);
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_program_id")
        );
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index")
        );

        let warns: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. }
                    if diagnostic.level == "warn" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(
            warns.is_empty(),
            "valid transaction should have no warnings"
        );
    }

    #[test]
    fn test_oob_program_id_and_oob_account_index_emits_both_diagnostics() {
        // Instruction has both an OOB program_id_index and OOB account indices.
        // Both are reported as separate diagnostics (unified rules, no skipping).
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
                program_id_index: 99,  // OOB: only 2 keys
                accounts: vec![0, 77], // index 77 is also OOB
                data: vec![0xEE],
            }],
        };
        let tx = SolanaTransaction {
            signatures: vec![],
            message,
        };
        let registry = IdlRegistry::new();
        let config = LintConfig::default();
        let result = decode_instructions(&tx, &registry, &config);
        let fields = [result.fields, result.diagnostics].concat();

        let warns: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. }
                    if diagnostic.level == "warn" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            warns.len(),
            2,
            "expected oob_program_id and oob_account_index warns"
        );
        assert!(
            warns
                .iter()
                .any(|d| d.rule == "transaction::oob_program_id")
        );
        assert!(
            warns
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index")
        );
        let acct_warn = warns
            .iter()
            .find(|d| d.rule == "transaction::oob_account_index")
            .unwrap();
        assert_eq!(acct_warn.instruction_index, Some(0));
        assert!(
            acct_warn.message.contains("77"),
            "message should mention the OOB index 77"
        );

        // No ok-diagnostics expected -- both rules fired with warnings
        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(passes.is_empty(), "no ok-diagnostics when both rules fire");
    }
}
