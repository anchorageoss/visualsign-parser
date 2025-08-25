use crate::core::{InstructionVisualizer, VisualizerContext, visualize_with_any};
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::Instruction;
use solana_sdk::transaction::Transaction as SolanaTransaction;

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

pub fn decode_transfers(
    _block_data: &SolanaTransaction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok([].into())
}
