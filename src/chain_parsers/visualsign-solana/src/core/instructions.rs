use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use crate::idl::IdlRegistry;
use solana_parser::solana::parser::parse_transaction;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::Instruction;
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
                "error",
                "legacy transaction has no account keys",
                None,
            )],
        };
    }

    // Convert compiled instructions to full instructions, emitting diagnostics
    // for out-of-bounds indices instead of silently dropping them.
    // Every rule always reports (pass or warn), providing boot-metric-style attestation.
    let mut diagnostics: Vec<AnnotatedPayloadField> = Vec::new();
    let mut oob_program_id_count: usize = 0;
    let mut oob_account_index_count: usize = 0;
    let mut oob_account_index_in_skipped_count: usize = 0;

    let oob_pid_severity = lint_config.severity_for(
        "transaction::oob_program_id",
        visualsign::lint::Severity::Warn,
    );
    let oob_acct_severity = lint_config.severity_for(
        "transaction::oob_account_index",
        visualsign::lint::Severity::Warn,
    );
    let oob_acct_skipped_severity = lint_config.severity_for(
        "transaction::oob_account_index_in_skipped_instruction",
        visualsign::lint::Severity::Warn,
    );

    // Each entry preserves the original instruction index for consistent labeling.
    let mut indexed_instructions: Vec<(usize, Instruction)> = Vec::new();

    for (ci_index, ci) in message.instructions.iter().enumerate() {
        if (ci.program_id_index as usize) >= account_keys.len() {
            oob_program_id_count += 1;
            if !matches!(oob_pid_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_program_id",
                    "transaction",
                    oob_pid_severity.as_str(),
                    &format!(
                        "instruction {} skipped: program_id_index {} out of bounds ({} accounts)",
                        ci_index,
                        ci.program_id_index,
                        account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
            // Even though this instruction is skipped, check its account indices
            // under a separate rule so the oob_account_index_in_skipped_instruction
            // rule can attest they were examined.
            let mut skipped_oob: Vec<u8> = Vec::new();
            for &i in ci.accounts.iter() {
                if (i as usize) >= account_keys.len() {
                    skipped_oob.push(i);
                }
            }
            if !skipped_oob.is_empty() {
                oob_account_index_in_skipped_count += 1;
                if !matches!(oob_acct_skipped_severity, visualsign::lint::Severity::Allow) {
                    diagnostics.push(create_diagnostic_field(
                        "transaction::oob_account_index_in_skipped_instruction",
                        "transaction",
                        oob_acct_skipped_severity.as_str(),
                        &format!(
                            "instruction {} (skipped): account indices {:?} out of bounds ({} accounts)",
                            ci_index,
                            skipped_oob,
                            account_keys.len()
                        ),
                        Some(ci_index as u32),
                    ));
                }
            }
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
            oob_account_index_count += 1;
            if !matches!(oob_acct_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_account_index",
                    "transaction",
                    oob_acct_severity.as_str(),
                    &format!(
                        "instruction {}: account indices {:?} out of bounds ({} accounts)",
                        ci_index,
                        oob_account_indices,
                        account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
        }

        indexed_instructions.push((
            ci_index,
            Instruction {
                program_id: account_keys[ci.program_id_index as usize],
                accounts,
                data: ci.data.clone(),
            },
        ));
    }

    // Emit ok diagnostics when all checks passed (boot-metric-style attestation)
    if oob_program_id_count == 0 && lint_config.should_report_ok("transaction::oob_program_id") {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_program_id",
            "transaction",
            "ok",
            &format!(
                "all {} instructions have valid program_id_index",
                message.instructions.len()
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
            "ok",
            &format!(
                "all {} instructions have valid account indices",
                message.instructions.len()
            ),
            None,
        ));
    }
    if oob_account_index_in_skipped_count == 0
        && lint_config.should_report_ok("transaction::oob_account_index_in_skipped_instruction")
    {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_account_index_in_skipped_instruction",
            "transaction",
            "ok",
            &format!("all {oob_program_id_count} skipped instructions have valid account indices"),
            None,
        ));
    }

    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();
    let mut errors: Vec<(usize, VisualSignError)> = Vec::new();

    // Extract just the instructions for the visualizer context (it needs the full slice)
    let instructions: Vec<Instruction> = indexed_instructions
        .iter()
        .map(|(_, ix)| ix.clone())
        .collect();

    for (original_index, instruction) in &indexed_instructions {
        let sender = SolanaAccount {
            account_key: account_keys[0].to_string(),
            signer: false,
            writable: false,
        };

        let context = VisualizerContext::new(&sender, *original_index, &instructions, idl_registry);

        match visualize_with_any(&visualizers_refs, &context) {
            Some(Ok(viz_result)) => fields.push(viz_result.field),
            Some(Err(e)) => errors.push((*original_index, e)),
            None => errors.push((
                *original_index,
                VisualSignError::DecodeError(format!(
                    "No visualizer available for instruction {} at index {}",
                    instruction.program_id, original_index
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

        // oob_account_index and oob_account_index_in_skipped_instruction should pass
        // since all instructions (including the skipped one) have valid account indices
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
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
        );

        let non_diagnostics: Vec<_> = fields
            .iter()
            .filter(|f| f.signable_payload_field.field_type() != "diagnostic")
            .collect();
        assert_eq!(non_diagnostics.len(), 1);
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
        // All three rules should report pass
        assert_eq!(passes.len(), 3);
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
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
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
        // The new rule fires to attest that account indices in skipped instructions
        // are also examined.
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
            "expected oob_program_id and oob_account_index_in_skipped_instruction warns"
        );
        assert!(
            warns
                .iter()
                .any(|d| d.rule == "transaction::oob_program_id")
        );
        assert!(
            warns
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
        );
        let skipped_warn = warns
            .iter()
            .find(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
            .unwrap();
        assert_eq!(skipped_warn.instruction_index, Some(0));
        assert!(
            skipped_warn.message.contains("77"),
            "message should mention the OOB index 77"
        );

        // oob_account_index (for non-skipped instructions) should report ok
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
}
