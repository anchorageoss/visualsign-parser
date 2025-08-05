use super::determine_transaction_type_string;
use crate::utils::{create_address_field, truncate_address};

use sui_json_rpc_types::{SuiTransactionBlockData, SuiTransactionBlockDataAPI};
use sui_types::transaction::TransactionData;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout,
    field_builders::{create_amount_field, create_raw_data_field, create_text_field},
};

pub fn add_tx_network() -> AnnotatedPayloadField {
    create_text_field("Network", "Sui Network")
}

pub fn add_tx_details(
    tx_data: &TransactionData,
    block_data: &SuiTransactionBlockData,
) -> SignablePayloadField {
    let mut payload_fields: Vec<AnnotatedPayloadField> = vec![];

    payload_fields.extend(create_tx_type_fields(block_data));
    payload_fields.extend(create_tx_gas_fields(block_data));
    payload_fields.extend(create_tx_data_fields(tx_data));

    SignablePayloadField::ListLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: "Transaction Details".to_string(),
            label: "Transaction Details".to_string(),
        },
        list_layout: SignablePayloadFieldListLayout {
            fields: payload_fields,
        },
    }
}

fn create_tx_type_fields(block_data: &SuiTransactionBlockData) -> Vec<AnnotatedPayloadField> {
    vec![create_text_field(
        "Transaction Type",
        &determine_transaction_type_string(block_data),
    )]
}

fn create_tx_gas_fields(block_data: &SuiTransactionBlockData) -> Vec<AnnotatedPayloadField> {
    vec![
        create_address_field(
            "Gas Owner",
            &truncate_address(&block_data.gas_data().owner.to_string()),
            None,
            None,
            None,
            None,
        ),
        create_amount_field(
            "Gas Budget",
            &block_data.gas_data().budget.to_string(),
            "MIST",
        ),
        create_amount_field(
            "Gas Price",
            &block_data.gas_data().price.to_string(),
            "MIST",
        ),
    ]
}

fn create_tx_data_fields(tx_data: &TransactionData) -> Vec<AnnotatedPayloadField> {
    if let Ok(encoded) = bcs::to_bytes::<TransactionData>(tx_data) {
        vec![create_raw_data_field(&encoded, None)]
    } else {
        vec![]
    }
}
