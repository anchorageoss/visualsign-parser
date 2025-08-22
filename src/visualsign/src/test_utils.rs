use crate::{SignablePayload, SignablePayloadField};

pub fn assert_has_field(payload: &SignablePayload, label: &str) {
    let (found, _) = payload
        .fields
        .iter()
        .map(|field| check_signable_payload_field(field, label))
        .find(|x| x.0)
        .unwrap_or((false, "".to_string()));
    assert!(found, "Should have a {label} field");
}

pub fn assert_has_field_with_value(payload: &SignablePayload, label: &str, expected_value: &str) {
    let (found, value) = payload
        .fields
        .iter()
        .map(|field| check_signable_payload_field(field, label))
        .find(|x| x.0)
        .unwrap_or((false, "".to_string()));
    assert!(
        found,
        "Should have a {label} field with value {expected_value}"
    );
    assert_eq!(
        value, expected_value,
        "Should have a {label} field with value {expected_value}. Actual value: {value}"
    );
}

fn check_signable_payload_field(field: &SignablePayloadField, label: &str) -> (bool, String) {
    match field {
        SignablePayloadField::Text { common, text } => {
            (common.label == label, text.text.to_string())
        }
        SignablePayloadField::TextV2 { common, text_v2 } => {
            (common.label == label, text_v2.text.to_string())
        }
        SignablePayloadField::Address { common, address } => {
            (common.label == label, address.address.to_string())
        }
        SignablePayloadField::AddressV2 { common, address_v2 } => {
            (common.label == label, address_v2.address.to_string())
        }
        SignablePayloadField::Number { common, number } => {
            (common.label == label, number.number.to_string())
        }
        SignablePayloadField::Amount { common, amount } => {
            (common.label == label, amount.amount.to_string())
        }
        SignablePayloadField::AmountV2 { common, amount_v2 } => {
            (common.label == label, amount_v2.amount.to_string())
        }
        SignablePayloadField::PreviewLayout {
            preview_layout,
            common,
        } => {
            if common.label == label {
                return (true, common.fallback_text.to_string());
            }

            let condensed_check: (bool, String) = if let Some(condensed) =
                preview_layout.condensed.as_ref()
            {
                condensed
                    .fields
                    .iter()
                    .map(|field| check_signable_payload_field(&field.signable_payload_field, label))
                    .find(|x| x.0)
                    .unwrap_or((false, "".to_string()))
            } else {
                (false, "".to_string())
            };

            let expanded_check: (bool, String) = if let Some(expanded) =
                preview_layout.expanded.as_ref()
            {
                expanded
                    .fields
                    .iter()
                    .map(|field| check_signable_payload_field(&field.signable_payload_field, label))
                    .find(|x| x.0)
                    .unwrap_or((false, "".to_string()))
            } else {
                (false, "".to_string())
            };

            if let (true, value) = condensed_check {
                return (true, value);
            }

            expanded_check
        }
        SignablePayloadField::ListLayout {
            list_layout,
            common,
        } => {
            if common.label == label {
                return (true, common.fallback_text.to_string());
            }

            list_layout
                .fields
                .iter()
                .map(|field| check_signable_payload_field(&field.signable_payload_field, label))
                .find(|x| x.0)
                .unwrap_or((false, "".to_string()))
        }
        _ => (false, "".to_string()),
    }
}
