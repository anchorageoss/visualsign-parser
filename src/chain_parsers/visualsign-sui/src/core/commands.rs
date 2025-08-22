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

    // TODO: Add a comment that `available_visualizers` is generated
    let visualizers: Vec<Box<dyn CommandVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn CommandVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

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

    #[test]
    fn test_visualizer_kind_for_suilend_repay() {
        // https://suivision.xyz/txblock/FTckS194eV3LBGCfcqiW8LxD7E3Nif5MNWqZa21jE5fn
        let test_data = "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";

        let block_data = block_data_from_b64(test_data);
        let (tx_commands, tx_inputs) = match block_data.transaction() {
            SuiTransactionBlockKind::ProgrammableTransaction(tx) => (&tx.commands, &tx.inputs),
            _ => panic!("expected programmable transaction"),
        };

        let visualizer = crate::presets::suilend::SuilendVisualizer;
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
            results
                .iter()
                .any(|r| matches!(r.kind, VisualizerKind::Lending(name) if name == "Suilend")),
            "should contain a suilend lending visualization"
        );
    }
}
