use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use crate::idl::IdlRegistry;
use solana_parser::solana::parser::parse_transaction;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::Instruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::{TransactionParseError, VisualSignError};

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

    // Convert compiled instructions to full instructions, skipping those with out-of-bounds
    // account indices (which can occur with malformed/fuzz inputs).
    let instructions: Vec<Instruction> = message
        .instructions
        .iter()
        .filter_map(|ci| {
            let program_id_idx = ci.program_id_index as usize;
            if program_id_idx >= account_keys.len() {
                return None;
            }
            let program_id = account_keys[program_id_idx];

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
                        None
                    }
                })
                .collect();

            Some(Instruction {
                program_id,
                accounts,
                data: ci.data.clone(),
            })
        })
        .collect();

    // Use the zero pubkey as a placeholder sender when there are no account keys.
    let sender_key = account_keys
        .first()
        .map(|k| k.to_string())
        .unwrap_or_else(|| solana_sdk::pubkey::Pubkey::default().to_string());

    let results: Result<Vec<AnnotatedPayloadField>, VisualSignError> = instructions
        .iter()
        .enumerate()
        .map(|(instruction_index, instruction)| {
            let sender = SolanaAccount {
                account_key: sender_key.clone(),
                signer: false,
                writable: false,
            };

            let context =
                VisualizerContext::new(&sender, instruction_index, &instructions, idl_registry);

            // Try to visualize with available visualizers (including unknown_program fallback).
            // Return an error instead of panicking if all visualizers decline the instruction.
            visualize_with_any(&visualizers_refs, &context)
                .ok_or_else(|| {
                    VisualSignError::ParseError(TransactionParseError::DecodeError(format!(
                        "Failed to visualize instruction {} at index {}",
                        instruction.program_id, instruction_index
                    )))
                })?
                .map(|viz_result| viz_result.field)
        })
        .collect();

    Ok(results?)
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
