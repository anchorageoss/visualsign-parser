//! Aggregated test fixtures and helpers for protocol visualizers.
//!
//! The aggregated runner reduces complexity when testing visualizers by loading a single
//! JSON file (`aggregated_test_data.json`) per integration and verifying labeled fields.
//!
//! When a visualizer is created, the goal is to verify that specific fields appear with the
//! correct values in the final output. The aggregated fixture (`aggregated_test_data.json`)
//! is organized as a nested map of modules → categories → operations, and includes enough
//! metadata for the runner to select the exact command and visualized result to assert.
//!
//! JSON structure (high-level):
//! - `explorer_tx_prefix` (string): prefix used to build human‑readable context (e.g., explorer links)
//! - `<moduleName>` (object): a group of categories for a Sui package/module family; for each module:
//!   - `<categoryName>` (object):
//!     - `label` (string): the expected title/label of the rendered field group
//!     - `operations` (object): map of `<operationId>` → operation
//!       - operation fields:
//!         - `data` (string): base64‑encoded transaction block
//!         - `command_index` (number): index of the `ProgrammableTransaction` command to visualize
//!         - `visualize_result_index` (number): index into the visualizer's returned vector to assert
//!         - `asserts` (object): map of field label → expected value(s)
//!           - value can be a string (exact match) or an array of strings (exact order and length)
//!
//! Minimal example:
//! ```json
//! {
//!   "explorer_tx_prefix": "https://suivision.xyz/txblock/",
//!   "pool_script": {
//!     "swap_a2b": {
//!       "label": "CetusAMM Swap Command",
//!       "operations": {
//!         "<tx_digest>": {
//!           "data": "<base64_transaction>",
//!           "command_index": 2,
//!           "visualize_result_index": 0,
//!           "asserts": {
//!             "User Address": "0x...",
//!             "Amount In": "1000"
//!           }
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! The data is mainly obtained from the `SuiVision` explorer. The raw format can be found in the Raw JSON tab.
//!
//! Assertions are represented as a map from field names to expected values or lists of expected values.
//!
//! As shown in the `cetus` and other presets, create a JSON file that matches this format and run the `run_aggregated_fixture` test.

use crate::core::{CommandVisualizer, SuiModuleResolver, VisualizerContext};
use crate::{SuiTransactionWrapper, transaction_string_to_visual_sign};

use std::collections::HashMap;

use move_bytecode_utils::module_cache::SyncModuleCache;

use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};

use visualsign::SignablePayload;
use visualsign::test_utils::check_signable_payload_field;
use visualsign::vsptrait::{Transaction, VisualSignOptions};

pub fn payload_from_b64(data: &str) -> SignablePayload {
    transaction_string_to_visual_sign(
        data,
        VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: false,
        },
    )
    .expect("Failed to visualize tx commands")
}

#[allow(dead_code)]
pub fn payload_from_b64_with_context(data: &str, context: &str) -> SignablePayload {
    match transaction_string_to_visual_sign(
        data,
        VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            partial_parsing: false,
        },
    ) {
        Ok(payload) => payload,
        Err(e) => panic!("Failed to visualize tx commands. Error: {e}, context: {context}"),
    }
}

/// Shared structure for aggregated test data loaded from JSON fixtures.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, serde::Deserialize)]
pub struct Operation {
    pub data: String,
    pub command_index: usize,
    pub visualize_result_index: usize,
    pub asserts: HashMap<String, OneOrMany>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Category {
    pub label: String,
    pub operations: HashMap<String, Operation>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AggregatedTestData {
    pub explorer_tx_prefix: String,
    #[serde(flatten)]
    pub modules: HashMap<String, HashMap<String, Category>>,
}

/// Runs a standard aggregated test over protocol JSON fixtures.
/// - `json_str`: contents of `aggregated_test_data.json` via `include_str`!
/// - `protocol`: short name, used only in assertion context strings
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn run_aggregated_fixture(json_str: &str, protocol: Box<dyn CommandVisualizer>) {
    let data: AggregatedTestData =
        serde_json::from_str(json_str).expect("invalid aggregated_test_data.json");

    // TODO: use module during visualization (in details)
    for module in data.modules.values() {
        for (name, category) in module {
            let label = &category.label;
            for (op_id, op) in &category.operations {
                let test_info_context = format!(
                    "Test name: {name}. Tx id: {}{op_id}",
                    data.explorer_tx_prefix
                );

                let block_data: SuiTransactionBlockData =
                    SuiTransactionBlockData::try_from_with_module_cache(
                        SuiTransactionWrapper::from_string(&op.data)
                            .expect("Failed to parse transaction. {test_context}")
                            .inner()
                            .clone(),
                        &SyncModuleCache::new(SuiModuleResolver),
                    )
                    .expect("Failed to convert transaction to block data. {test_context}");

                let (tx_commands, tx_inputs) = match block_data.transaction() {
                    SuiTransactionBlockKind::ProgrammableTransaction(tx) => {
                        (&tx.commands, &tx.inputs)
                    }
                    _ => {
                        panic!("Transaction is not a programmable transaction. {test_info_context}")
                    }
                };

                assert!(
                    op.command_index < tx_commands.len(),
                    "Command index is out of bounds. {test_info_context}"
                );

                let context = VisualizerContext::new(
                    block_data.sender(),
                    op.command_index,
                    tx_commands,
                    tx_inputs,
                );

                assert!(
                    protocol.can_handle(&context),
                    "Protocol {:?} cannot handle command with index: {}. {test_info_context}",
                    protocol.kind(),
                    op.command_index
                );

                let visualized_result = match protocol.visualize_tx_commands(&context) {
                    Ok(result) => result,
                    Err(e) => {
                        panic!("Failed to visualize command. {test_info_context}. Error: {e}")
                    }
                };

                assert!(
                    op.visualize_result_index < visualized_result.len(),
                    "Visualize result index is out of bounds. {test_info_context}"
                );
                let result_to_assert = visualized_result.get(op.visualize_result_index).unwrap();

                let (label_found, _) =
                    check_signable_payload_field(&result_to_assert.signable_payload_field, label);
                assert!(
                    label_found,
                    "Should have a '{label}' field in {test_info_context}"
                );

                for (field, expected) in &op.asserts {
                    match expected {
                        OneOrMany::One(expected_value) => {
                            let (found, actual_values) = check_signable_payload_field(
                                &result_to_assert.signable_payload_field,
                                field,
                            );
                            assert!(
                                found,
                                "Should have a '{field}' field with value {expected_value} in {test_info_context}"
                            );
                            assert!(
                                actual_values.iter().all(|x| x.eq(expected_value)),
                                "Should have a '{field}' field with value {expected_value}. Actual values: {actual_values:?} in {test_info_context}"
                            );
                        }
                        OneOrMany::Many(expected_values) => {
                            let (found, actual_values) = check_signable_payload_field(
                                &result_to_assert.signable_payload_field,
                                field,
                            );

                            assert!(
                                found,
                                "Should have at least one '{field}' field in {test_info_context}"
                            );

                            assert_eq!(
                                actual_values.len(),
                                expected_values.len(),
                                "Should have {} '{field}' field(s) in {test_info_context}. Actual values: {actual_values:?}",
                                expected_values.len()
                            );

                            assert_eq!(
                                actual_values.as_slice(),
                                expected_values.as_slice(),
                                "Mismatch in '{field}' field values in {test_info_context}. Expected: {expected_values:?}, Actual: {actual_values:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
