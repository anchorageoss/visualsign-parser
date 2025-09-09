use super::determine_transaction_type_string;

use sui_json_rpc_types::{SuiTransactionBlockData, SuiTransactionBlockDataAPI};
use sui_types::transaction::TransactionData;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{
        create_address_field, create_amount_field, create_raw_data_field, create_text_field,
    },
};

pub fn get_tx_network() -> Result<AnnotatedPayloadField, VisualSignError> {
    create_text_field("Network", "Sui Network")
}

pub fn get_tx_details(
    tx_data: &TransactionData,
    block_data: &SuiTransactionBlockData,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let payload_fields: Vec<AnnotatedPayloadField> = vec![create_tx_type_fields(block_data)?]
        .into_iter()
        .chain(create_tx_gas_fields(block_data)?)
        .chain(create_tx_data_fields(tx_data)?)
        .collect();

    {
        let title_text = "Transaction Details".to_string();
        let subtitle_text = format!("Gas: {} MIST", block_data.gas_data().budget);

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![
                create_tx_type_fields(block_data)?,
                create_amount_field(
                    "Gas Budget",
                    &block_data.gas_data().budget.to_string(),
                    "MIST",
                )?,
            ],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: payload_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Transaction Details".to_string(),
                },
                preview_layout,
            },
        })
    }
}

fn create_tx_type_fields(
    block_data: &SuiTransactionBlockData,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    create_text_field(
        "Transaction Type",
        determine_transaction_type_string(block_data),
    )
}

fn create_tx_gas_fields(
    block_data: &SuiTransactionBlockData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok(vec![
        create_address_field(
            "Gas Owner",
            &block_data.gas_data().owner.to_string(),
            None,
            None,
            None,
            None,
        )?,
        create_amount_field(
            "Gas Budget",
            &block_data.gas_data().budget.to_string(),
            "MIST",
        )?,
        create_amount_field(
            "Gas Price",
            &block_data.gas_data().price.to_string(),
            "MIST",
        )?,
    ])
}

fn create_tx_data_fields(
    tx_data: &TransactionData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let Ok(encoded) = bcs::to_bytes::<TransactionData>(tx_data) else {
        return Ok(vec![]);
    };

    Ok(vec![create_raw_data_field(&encoded, None)?])
}
