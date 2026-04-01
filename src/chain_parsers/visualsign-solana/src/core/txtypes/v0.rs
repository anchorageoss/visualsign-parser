use crate::core::{
    InstructionVisualizer, SolanaAccount, VisualizerContext, available_visualizers,
    visualize_with_any,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::transaction::VersionedTransaction;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::create_diagnostic_field, vsptrait::VisualSignError,
};

/// Decode V0 transaction transfers using solana-parser
/// This works with V0 transactions including those with lookup tables
pub fn decode_v0_transfers(
    versioned_tx: &VersionedTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    use solana_parser::solana::parser::parse_transaction;

    // Serialize the full versioned transaction
    let transaction_bytes = bincode::serialize(versioned_tx).map_err(|e| {
        VisualSignError::ParseError(visualsign::vsptrait::TransactionParseError::DecodeError(
            format!("Failed to serialize V0 transaction: {e}"),
        ))
    })?;

    let is_full_transaction = true; // true because we're passing full tx and not message
    // Parse using solana-parser which handles V0 transactions and lookup tables
    let parsed_transaction = parse_transaction(
        hex::encode(transaction_bytes),
        is_full_transaction,
        None,
    )
    .map_err(|e| {
        VisualSignError::ParseError(visualsign::vsptrait::TransactionParseError::DecodeError(
            format!("Failed to parse V0 transaction: {e}"),
        ))
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
                let field = AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!(
                                "Transfer {}: From {} To {} For {}",
                                i + 1,
                                transfer.from,
                                transfer.to,
                                transfer.amount
                            ),
                            label: format!("Transfer {}", i + 1),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
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
                    signable_payload_field: SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!(
                                "SPL Transfer {}: From {} To {} For {}",
                                i + 1,
                                spl_transfer.from,
                                spl_transfer.to,
                                spl_transfer.amount
                            ),
                            label: format!("V0 SPL Transfer {}", i + 1),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
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

/// Decode V0 transaction instructions using the visualizer framework
/// This works for all V0 transactions, including those with lookup tables
/// Result of decoding v0 instructions: display fields, per-instruction errors,
/// and lint diagnostics separately. The function always succeeds — individual
/// instruction failures are captured in `errors` rather than aborting the parse.
pub struct DecodeV0InstructionsResult {
    pub fields: Vec<AnnotatedPayloadField>,
    pub errors: Vec<(usize, VisualSignError)>,
    pub diagnostics: Vec<AnnotatedPayloadField>,
}

/// Always succeeds — data quality issues become diagnostics, per-instruction
/// failures are collected in errors.
pub fn decode_v0_instructions(
    v0_message: &solana_sdk::message::v0::Message,
    idl_registry: &crate::idl::IdlRegistry,
    lint_config: &visualsign::lint::LintConfig,
) -> DecodeV0InstructionsResult {
    // Get visualizers
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    // For V0 transactions, we need to resolve account keys from both static keys and lookup tables
    // For now, we'll work with just the static account keys for instruction processing
    // since lookup table accounts would require on-chain resolution
    let account_keys = &v0_message.account_keys;

    if account_keys.is_empty() {
        return DecodeV0InstructionsResult {
            fields: Vec::new(),
            errors: Vec::new(),
            diagnostics: vec![create_diagnostic_field(
                "transaction::empty_account_keys",
                "transaction",
                "error",
                "v0 transaction has no account keys",
                None,
            )],
        };
    }

    // Convert compiled instructions to full instructions, emitting diagnostics
    // for indices that reference lookup table accounts (unresolvable without on-chain data).
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

    for (ci_index, ci) in v0_message.instructions.iter().enumerate() {
        // Always check account indices, even if program_id is OOB
        let mut oob_account_indices: Vec<u8> = Vec::new();
        for &i in &ci.accounts {
            if (i as usize) >= account_keys.len() {
                oob_account_indices.push(i);
            }
        }
        if !oob_account_indices.is_empty() {
            oob_account_index_count += 1;
            if !matches!(oob_acct_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_account_index",
                    "transaction",
                    oob_acct_severity.as_str(),
                    &format!(
                        "instruction {}: account indices {:?} reference lookup table accounts ({} static keys)",
                        ci_index,
                        oob_account_indices,
                        account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
        }

        if (ci.program_id_index as usize) >= account_keys.len() {
            oob_program_id_count += 1;
            if !matches!(oob_pid_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_program_id",
                    "transaction",
                    oob_pid_severity.as_str(),
                    &format!(
                        "instruction {} skipped: program_id_index {} references a lookup table account ({} static keys)",
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
                            "instruction {} (skipped): account indices {:?} reference lookup table accounts ({} static keys)",
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

        let accounts: Vec<AccountMeta> = ci
            .accounts
            .iter()
            .filter_map(|&i| {
                if (i as usize) < account_keys.len() {
                    Some(AccountMeta::new_readonly(account_keys[i as usize], false))
                } else {
                    None // already counted above
                }
            })
            .collect();

        indexed_instructions.push((
            ci_index,
            Instruction {
                program_id: account_keys[ci.program_id_index as usize],
                accounts,
                data: ci.data.clone(),
            },
        ));
    }

    // Emit pass diagnostics when all checks passed (boot-metric-style attestation)
    if oob_program_id_count == 0 && lint_config.should_report_ok("transaction::oob_program_id") {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_program_id",
            "transaction",
            "ok",
            &format!(
                "all {} instructions have valid program_id_index",
                v0_message.instructions.len()
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
                instructions.len()
            ),
            None,
        ));
    }
    if oob_account_index_in_skipped_count == 0
        && lint_config
            .should_report_ok("transaction::oob_account_index_in_skipped_instruction")
    {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_account_index_in_skipped_instruction",
            "transaction",
            "ok",
            &format!(
                "all {oob_program_id_count} skipped instructions have valid account indices"
            ),
            None,
        ));
    }

    // Process each instruction with the visualizer framework
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();
    let mut errors: Vec<(usize, VisualSignError)> = Vec::new();

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

        match visualize_with_any(
            &visualizers_refs,
            &VisualizerContext::new(&sender, *original_index, &instructions, idl_registry),
        ) {
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

    DecodeV0InstructionsResult {
        fields,
        errors,
        diagnostics,
    }
}

/// Create a rich address lookup table field with detailed information
/// Reuses the advanced preview layout pattern to avoid top-level ListLayout restriction
pub fn create_address_lookup_table_field(
    v0_message: &solana_sdk::message::v0::Message,
) -> Result<SignablePayloadField, VisualSignError> {
    // Create fallback text with lookup table addresses
    let fallback_text = v0_message
        .address_table_lookups
        .iter()
        .map(|lookup| lookup.account_key.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    // Create the expanded fields manually for more detailed view
    let mut expanded_fields = vec![AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: v0_message.address_table_lookups.len().to_string(),
                label: "Total Tables".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: v0_message.address_table_lookups.len().to_string(),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }];

    // Add individual lookup table entries with details
    for (i, lookup) in v0_message.address_table_lookups.iter().enumerate() {
        let table_label = if v0_message.address_table_lookups.len() == 1 {
            "Table Address".to_string()
        } else {
            format!("Table {} Address", i + 1)
        };

        expanded_fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: lookup.account_key.to_string(),
                    label: table_label,
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: lookup.account_key.to_string(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        // Add writable and readonly account counts
        if !lookup.writable_indexes.is_empty() {
            expanded_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} accounts", lookup.writable_indexes.len()),
                        label: if v0_message.address_table_lookups.len() == 1 {
                            "Writable Accounts".to_string()
                        } else {
                            format!("Table {} Writable", i + 1)
                        },
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(
                            "{} writable accounts (indices: {:?})",
                            lookup.writable_indexes.len(),
                            lookup.writable_indexes
                        ),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }

        if !lookup.readonly_indexes.is_empty() {
            expanded_fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} accounts", lookup.readonly_indexes.len()),
                        label: if v0_message.address_table_lookups.len() == 1 {
                            "Readonly Accounts".to_string()
                        } else {
                            format!("Table {} Readonly", i + 1)
                        },
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(
                            "{} readonly accounts (indices: {:?})",
                            lookup.readonly_indexes.len(),
                            lookup.readonly_indexes
                        ),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }
    }

    // Create summary for condensed view
    let mut condensed_fields = vec![AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} Tables", v0_message.address_table_lookups.len()),
                label: "Total Tables".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{} Tables", v0_message.address_table_lookups.len()),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }];

    // Add table addresses to condensed view (just the addresses)
    for lookup in &v0_message.address_table_lookups {
        condensed_fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: lookup.account_key.to_string(),
                    label: "Table".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: lookup.account_key.to_string(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }

    let condensed_list = SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };

    let expanded_list = SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    // Use PreviewLayout pattern like the accounts function
    Ok(SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text,
            label: "Address Lookup Tables".to_string(),
        },
        preview_layout: SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: "Address Lookup Tables".to_string(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: format!(
                    "{} table{}",
                    v0_message.address_table_lookups.len(),
                    if v0_message.address_table_lookups.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            }),
            condensed: Some(condensed_list),
            expanded: Some(expanded_list),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use visualsign::SignablePayloadField;
    use visualsign::lint::LintConfig;

    fn v0_message_with_oob_program_id() -> solana_sdk::message::v0::Message {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        solana_sdk::message::v0::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![0],
                    data: vec![0xAA],
                },
                solana_sdk::instruction::CompiledInstruction {
                    program_id_index: 99, // OOB: only 2 static keys
                    accounts: vec![0],
                    data: vec![0xBB],
                },
            ],
            address_table_lookups: vec![],
        }
    }

    fn v0_message_with_oob_program_id_and_oob_account() -> solana_sdk::message::v0::Message {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        solana_sdk::message::v0::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![solana_sdk::instruction::CompiledInstruction {
                program_id_index: 99, // OOB
                accounts: vec![0, 88], // 88 is also OOB
                data: vec![0xCC],
            }],
            address_table_lookups: vec![],
        }
    }

    #[test]
    fn test_v0_oob_program_id_emits_diagnostic() {
        let msg = v0_message_with_oob_program_id();
        let registry = crate::idl::IdlRegistry::new();
        let config = LintConfig::default();
        let result = decode_v0_instructions(&msg, &registry, &config);
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

        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(passes.iter().any(|d| d.rule == "transaction::oob_account_index"));
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
        );
    }

    #[test]
    fn test_v0_oob_program_id_and_oob_account_index_emits_both_diagnostics() {
        let msg = v0_message_with_oob_program_id_and_oob_account();
        let registry = crate::idl::IdlRegistry::new();
        let config = LintConfig::default();
        let result = decode_v0_instructions(&msg, &registry, &config);
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
        assert_eq!(warns.len(), 2);
        assert!(warns.iter().any(|d| d.rule == "transaction::oob_program_id"));
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
        assert!(skipped_warn.message.contains("88"));

        let passes: Vec<_> = fields
            .iter()
            .filter_map(|f| match &f.signable_payload_field {
                SignablePayloadField::Diagnostic { diagnostic, .. } if diagnostic.level == "ok" => {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect();
        assert!(passes.iter().any(|d| d.rule == "transaction::oob_account_index"));
    }

    #[test]
    fn test_v0_valid_transaction_emits_three_pass_diagnostics() {
        let key0 = Pubkey::new_unique();
        let key1 = Pubkey::new_unique();
        let msg = solana_sdk::message::v0::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key0, key1],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![solana_sdk::instruction::CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0],
                data: vec![0xDD],
            }],
            address_table_lookups: vec![],
        };
        let registry = crate::idl::IdlRegistry::new();
        let config = LintConfig::default();
        let result = decode_v0_instructions(&msg, &registry, &config);
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
        assert_eq!(passes.len(), 3);
        assert!(passes.iter().any(|d| d.rule == "transaction::oob_program_id"));
        assert!(passes.iter().any(|d| d.rule == "transaction::oob_account_index"));
        assert!(
            passes
                .iter()
                .any(|d| d.rule == "transaction::oob_account_index_in_skipped_instruction")
        );
    }
}
