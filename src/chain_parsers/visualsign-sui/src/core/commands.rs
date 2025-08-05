use crate::core::{CommandVisualizer, VisualizerContext, visualize_with_any};

use sui_json_rpc_types::{
    SuiCallArg, SuiCommand, SuiTransactionBlockData, SuiTransactionBlockDataAPI,
    SuiTransactionBlockKind,
};
use visualsign::SignablePayloadField;

/// Returns a list of all available visualizers.
/// Extend this function to add new visualizers.
fn available_visualizers() -> Vec<Box<dyn CommandVisualizer>> {
    vec![
        Box::new(crate::presets::coin_transfer::CoinTransferVisualizer),
        Box::new(crate::presets::sui_native_staking::SuiNativeStakingVisualizer),
        Box::new(crate::presets::cetus::CetusVisualizer),
    ]
}

/// Extracts commands and inputs from a programmable transaction, or returns None.
fn extract_commands_and_inputs(
    block_data: &SuiTransactionBlockData,
) -> Option<(&Vec<SuiCommand>, &Vec<SuiCallArg>)> {
    match block_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(tx) => Some((&tx.commands, &tx.inputs)),
        _ => None,
    }
}

/// Visualizes all commands in a transaction block, returning their signable fields.
pub fn add_tx_commands(block_data: &SuiTransactionBlockData) -> Vec<SignablePayloadField> {
    let (tx_commands, tx_inputs) = match extract_commands_and_inputs(block_data) {
        Some((cmds, inputs)) => (cmds, inputs),
        None => return vec![],
    };

    let visualizers = available_visualizers();
    let visualizers_refs = visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    tx_commands
        .iter()
        .enumerate()
        .filter_map(|(command_index, _)| {
            visualize_with_any(
                &visualizers_refs,
                &VisualizerContext::new(block_data.sender(), command_index, tx_commands, tx_inputs),
            )
        })
        .collect()
}
