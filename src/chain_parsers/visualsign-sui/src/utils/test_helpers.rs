//! Aggregated test fixtures and helpers for protocol visualizers.
//!
//! The aggregated runner reduces complexity when testing visualizers.
//!
//! When a visualizer is created, the goal is to verify that specific fields appear with the correct values in the final output.
//! The JSON file (e.g., `aggregated_test_data.json`) has the following structure:
//! - `explorer_tx_prefix`: a prefix added to the test context
//! - `modules`: a map from module names to category maps
//! - `categories`: a map from category names to operations
//! - `operations`: a map from operation names to operation data and assertions
//!
//! The data is mainly obtained from the `SuiVision` explorer. The raw format can be found in the Raw JSON tab.
//!
//! Assertions are represented as a map from field names to expected values or lists of expected values.
//!
//! As shown in the `cetus` and other presets, create a JSON file that matches this format and run the `run_aggregated_fixture` test.

use std::collections::HashMap;

use crate::transaction_string_to_visual_sign;

use visualsign::SignablePayload;
use visualsign::test_utils::{
    assert_has_field_with_context, assert_has_field_with_value_with_context,
    assert_has_fields_with_values_with_context,
};
use visualsign::vsptrait::VisualSignOptions;

pub fn payload_from_b64(data: &str) -> SignablePayload {
    transaction_string_to_visual_sign(
        data,
        VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        },
    )
    .expect("Failed to visualize tx commands")
}

pub fn payload_from_b64_with_context(data: &str, context: &str) -> SignablePayload {
    match transaction_string_to_visual_sign(
        data,
        VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
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
pub fn run_aggregated_fixture(json_str: &str, _protocol: &str) {
    let data: AggregatedTestData =
        serde_json::from_str(json_str).expect("invalid aggregated_test_data.json");

    // TODO: use module during visualization (in details)
    for module in data.modules.values() {
        for (name, category) in module {
            let label = &category.label;
            for (op_id, op) in &category.operations {
                let test_context = format!(
                    "Test name: {name}. Tx id: {}{op_id}",
                    data.explorer_tx_prefix
                );

                let payload = payload_from_b64_with_context(&op.data, &test_context);

                assert_has_field_with_context(&payload, label, &test_context);
                for (field, expected) in &op.asserts {
                    match expected {
                        OneOrMany::One(value) => assert_has_field_with_value_with_context(
                            &payload,
                            field,
                            value.as_str(),
                            &test_context,
                        ),
                        OneOrMany::Many(values) => assert_has_fields_with_values_with_context(
                            &payload,
                            field,
                            values.as_slice(),
                            &test_context,
                        ),
                    }
                }
            }
        }
    }
}
