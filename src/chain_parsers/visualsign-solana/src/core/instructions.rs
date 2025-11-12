use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
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
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    // TODO: add comment that available_visualizers is generated
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    let message = &transaction.message;
    let account_keys = &message.account_keys;

    // Convert compiled instructions to full instructions
    let instructions: Vec<Instruction> = message
        .instructions
        .iter()
        .map(|ci| Instruction {
            program_id: account_keys[ci.program_id_index as usize],
            accounts: ci
                .accounts
                .iter()
                .map(|&i| {
                    solana_sdk::instruction::AccountMeta::new_readonly(
                        account_keys[i as usize],
                        false,
                    )
                })
                .collect(),
            data: ci.data.clone(),
        })
        .collect();

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

            let context = VisualizerContext::new(&sender, instruction_index, &instructions);

            // Try to visualize with available visualizers (including unknown_program fallback)
            visualize_with_any(&visualizers_refs, &context)
                .unwrap_or_else(|| {
                    panic!(
                        "No visualizer available for instruction {} at index {}",
                        instruction.program_id, instruction_index
                    )
                })
                .map(|viz_result| viz_result.field)
        })
        .collect();

    let fields = results?;

    // Self-check: ensure we have the same number of instruction fields as input instructions
    if fields.len() != instructions.len() {
        return Err(VisualSignError::InvariantViolation(format!(
            "Instruction count mismatch: expected {} instructions, got {} fields. This should never happen with unknown_program fallback.",
            instructions.len(),
            fields.len()
        )));
    }

    Ok(fields)
}

pub fn decode_transfers(
    transaction: &SolanaTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let message_clone = transaction.message.clone();
    let parsed_transaction = parse_transaction(
        hex::encode(message_clone.serialize()),
        false, /* because we're passing the message only */
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
