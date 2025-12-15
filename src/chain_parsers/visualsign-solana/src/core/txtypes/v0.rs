use crate::core::{
    InstructionVisualizer, SolanaAccount, VisualizerContext, available_visualizers,
    visualize_with_any,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::transaction::VersionedTransaction;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    vsptrait::VisualSignError,
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
pub fn decode_v0_instructions(
    v0_message: &solana_sdk::message::v0::Message,
    _idl_registry: &crate::idl::IdlRegistry,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    // Get visualizers
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    // For V0 transactions, we need to resolve account keys from both static keys and lookup tables
    // For now, we'll work with just the static account keys for instruction processing
    // since lookup table accounts would require on-chain resolution
    let account_keys = &v0_message.account_keys;

    // Convert compiled instructions to full instructions using static account keys only
    // Instructions that reference lookup table accounts will be processed with limited info
    let instructions: Vec<Instruction> = v0_message
        .instructions
        .iter()
        .filter_map(|ci| {
            // Only process instructions where program_id is in static account keys
            if (ci.program_id_index as usize) < account_keys.len() {
                let program_id = account_keys[ci.program_id_index as usize];

                let accounts: Vec<AccountMeta> = ci
                    .accounts
                    .iter()
                    .filter_map(|&i| {
                        // Only include accounts that are in static account keys
                        if (i as usize) < account_keys.len() {
                            Some(AccountMeta::new_readonly(account_keys[i as usize], false))
                        } else {
                            // Account is in lookup table - we can't resolve it without on-chain data
                            None
                        }
                    })
                    .collect();

                Some(Instruction {
                    program_id,
                    accounts,
                    data: ci.data.clone(),
                })
            } else {
                // Program ID is in lookup table - skip this instruction
                None
            }
        })
        .collect();

    // Process each instruction with the visualizer framework
    if account_keys.is_empty() {
        return Err(VisualSignError::ParseError(
            visualsign::vsptrait::TransactionParseError::DecodeError(
                "V0 transaction has no account keys".to_string(),
            ),
        ));
    }

    instructions
        .iter()
        .enumerate()
        .filter_map(|(instruction_index, _)| {
            // Create sender account from first account key (typically the fee payer)
            let sender = SolanaAccount {
                account_key: account_keys[0].to_string(),
                signer: false,
                writable: false,
            };

            visualize_with_any(
                &visualizers_refs,
                &VisualizerContext::new(&sender, instruction_index, &instructions),
            )
        })
        .map(|res| res.map(|viz_result| viz_result.field))
        .collect()
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
