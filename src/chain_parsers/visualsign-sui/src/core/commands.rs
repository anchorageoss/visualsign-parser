use crate::core::{CommandVisualizer, VisualizerContext, visualize_with_any};

use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};

use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;

include!(concat!(env!("OUT_DIR"), "/generated_visualizers.rs"));

/// Visualizes all commands in a transaction block, returning their signable fields.
pub fn decode_commands(
    block_data: &SuiTransactionBlockData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let (tx_commands, tx_inputs) = match block_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(tx) => (&tx.commands, &tx.inputs),
        _ => return Ok(vec![]),
    };

    // Note: `available_visualizers()` is generated at build time by build.rs.
    // It scans `src/presets` and `src/integrations` for visualizers and wires
    // them here. To adjust which visualizers are included, modify build.rs.
    let visualizers: Vec<Box<dyn CommandVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn CommandVisualizer> = visualizers
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect::<Vec<_>>();

    tx_commands
        .iter()
        .enumerate()
        .filter_map(|(command_index, _)| {
            visualize_with_any(
                &visualizers_refs,
                &VisualizerContext::new(block_data.sender(), command_index, tx_commands, tx_inputs),
            )
        })
        .map(|res| res.map(|viz_result| viz_result.field))
        .collect::<Result<Vec<Vec<AnnotatedPayloadField>>, _>>()
        .map(|nested| nested.into_iter().flatten().collect())
}

pub fn decode_transfers(
    block_data: &SuiTransactionBlockData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let (tx_commands, tx_inputs) = match block_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(tx) => (&tx.commands, &tx.inputs),
        _ => return Ok(vec![]),
    };

    let visualizer = crate::presets::coin_transfer::CoinTransferVisualizer;

    tx_commands
        .iter()
        .enumerate()
        .filter_map(|(command_index, _)| {
            visualize_with_any(
                &[&visualizer],
                &VisualizerContext::new(block_data.sender(), command_index, tx_commands, tx_inputs),
            )
        })
        .map(|res| res.map(|viz_result| viz_result.field))
        .collect::<Result<Vec<Vec<AnnotatedPayloadField>>, _>>()
        .map(|nested| nested.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SuiTransactionWrapper;
    use crate::core::helper::SuiModuleResolver;
    use crate::core::{VisualizerContext, VisualizerKind, visualize_with_any};

    use move_bytecode_utils::module_cache::SyncModuleCache;
    use visualsign::vsptrait::Transaction;

    fn block_data_from_b64(data: &str) -> SuiTransactionBlockData {
        let wrapper = <SuiTransactionWrapper as Transaction>::from_string(data).expect("parse tx");
        let tx = wrapper.inner().clone();

        SuiTransactionBlockData::try_from_with_module_cache(
            tx,
            &SyncModuleCache::new(SuiModuleResolver),
        )
        .expect("block data")
    }

    #[test]
    fn test_visualizer_kind_for_transfers() {
        // https://suivision.xyz/txblock/CE46w3GYgWnZU8HF4P149m6ANGebD22xuNqA64v7JykJ
        let test_data = "AQAAAAAABQEAm9cmP35lHGKppWJLgoYU7aexd43oTT2ci4QzxDXFNv92CAsjAAAAACANp0teIzSyzZ4Pj5dL3YaYBdeVmiWScWL/9RCV4mUINwEAARQFJheK7qwbpqmQudEhsSyQ6AjVawfLpN4XRBhe12FH6TIiAAAAACDXzuT2xanZ36QNQSYtDhZn31zfzIlhRk5H6pTsqGdRDAEAXpykdGz3KJdaAVjyAMZQxufRYJfqzNXfOu8jVCAjEjIzfYIhAAAAACA5hk9rACYb1i5fqrUBJIgXhdUFOqOaouNWmQINCW4/WQAIAPLhNQAAAAAAIEutPmqkZpN81fwdos/haXZAQJoZsX8SvKilyMRxrv/pAwMBAAACAQEAAQIAAgEAAAEBAwABAQIBAAEEAA4x8k3bZAV+p192pmk9h7U2nGDwuTmW8EY6c95JyFHCAaCnde0j6aiVXUd/1gCf3q5Uuj1mPVIuuEpJn1teueghdggLIwAAAAAgNhuP2zGpc0qF3gRzxQC5B0lpAZR7xyssXC3gKbH8uxwOMfJN22QFfqdfdqZpPYe1Npxg8Lk5lvBGOnPeSchRwugDAAAAAAAAoIVIAAAAAAAAAWEAFrlPuI8JOSzIoIBc0xwfWia7T5uPf1PS+aSSphoTTq0lRpNuTOg8eOggpBxpLsQDrbAx3jDoWg1R8hZKR62LBex1R808U6AgiY8V7LxOVsChXFf8nSAEGaeSLQc7mJbx";

        let block_data = block_data_from_b64(test_data);
        let (tx_commands, tx_inputs) = match block_data.transaction() {
            SuiTransactionBlockKind::ProgrammableTransaction(tx) => (&tx.commands, &tx.inputs),
            _ => panic!("expected programmable transaction"),
        };

        let visualizer = crate::presets::coin_transfer::CoinTransferVisualizer;

        let results: Vec<_> = tx_commands
            .iter()
            .enumerate()
            .filter_map(|(command_index, _)| {
                visualize_with_any(
                    &[&visualizer],
                    &VisualizerContext::new(
                        block_data.sender(),
                        command_index,
                        tx_commands,
                        tx_inputs,
                    ),
                )
            })
            .map(|res| res.unwrap())
            .collect();

        assert!(
            !results.is_empty(),
            "should visualize at least one transfer"
        );
        assert!(results.iter().all(
            |r| matches!(r.kind, VisualizerKind::Payments(name) if name == "Native Transfer")
        ));
    }
}
