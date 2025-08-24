use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use solana_parser::solana::parser::parse_transaction;
use solana_parser::solana::structs::SolanaAccount;
//use solana_parser::solana::parser::SolanaTransaction;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use solana_sdk::instruction::Instruction; // <-- Add this import

use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;

include!(concat!(env!("OUT_DIR"), "/generated_visualizers.rs"));

/// Visualizes all the instructions and related fields in a transaction/message
pub fn decode_instructions(
    transaction: &SolanaTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    // TODO: add comment that available_visualizers is generated
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    // this clone is probably unneccessary - todo revisit and switch to borrow
    let message_clone = transaction.message.clone();
    let parsed_transaction = parse_transaction(
        hex::encode(message_clone.serialize()),
        false, /* because we're passing the message only */
    )
    .expect("Failed to parse transaction");

    message_clone
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(command_index, instruction)| {
            visualize_with_any(
                &visualizers_refs,
                &VisualizerContext::new(
                    &SolanaAccount { account_key: message_clone.account_keys[0].to_string(), signer: false, writable: false }, // Construct SolanaAccount directly
                    command_index,
                    &message_clone
                        .instructions
                        .iter()
                        .map(|ci| {
                            Instruction {
                                program_id: message_clone.account_keys[ci.program_id_index as usize],
                                accounts: ci.accounts.iter().map(|&i| message_clone.account_keys[i as usize]).collect(),
                                data: ci.data.clone(),
                            }
                        })
                        .collect::<Vec<_>>(), // Manually convert CompiledInstruction to Instruction
                ),
            )
        })
        .map(|res| res.map(|viz_result| viz_result.field))
        .collect()
}

pub fn decode_transfers(
    block_data: &SolanaTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok([].into())
}
