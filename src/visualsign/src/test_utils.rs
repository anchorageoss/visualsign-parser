use crate::{SignablePayload, SignablePayloadField};

pub fn assert_has_field(payload: &SignablePayload, label: &str) {
    let (found, _) = check_signable_payload(payload, label);
    assert!(found, "Should have a {label} field");
}

pub fn assert_has_field_with_context(payload: &SignablePayload, label: &str, context: &str) {
    let (found, _) = check_signable_payload(payload, label);
    assert!(found, "Should have a {label} field in {context}");
}

pub fn assert_has_field_with_value(payload: &SignablePayload, label: &str, expected_value: &str) {
    let (found, values) = check_signable_payload(payload, label);
    assert!(
        found,
        "Should have a {label} field with value {expected_value}"
    );
    assert!(
        values.contains(&expected_value.to_string()),
        "Should have a {label} field with value {expected_value}. Actual values: {:?}",
        values
    );
}

pub fn assert_has_field_with_value_with_context(
    payload: &SignablePayload,
    label: &str,
    expected_value: &str,
    context: &str,
) {
    let (found, values) = check_signable_payload(payload, label);
    assert!(
        found,
        "Should have a {label} field with value {expected_value} in {context}"
    );
    assert!(
        values.iter().all(|x| x.eq(expected_value)),
        "Should have a {label} field with value {expected_value}. Actual values: {:?} (use `assert_has_fields_with_values_with_context` if there could be different expected values) in {context}",
        values
    );
}

pub fn assert_has_fields_with_values_with_context(
    payload: &SignablePayload,
    label: &str,
    expected: &[String],
    context: &str,
) {
    let (found, values) = check_signable_payload(payload, label);
    assert!(found, "Should have at least one {label} field in {context}");

    assert_eq!(
        values.len(),
        expected.len(),
        "Should have {} {label} field(s) in {context}. Actual values: {:?}",
        expected.len(),
        values
    );

    assert_eq!(
        values, expected,
        "Mismatch in {label} field values in {context}. Expected: {:?}, Actual: {:?}",
        expected, values
    );
}

pub fn check_signable_payload(payload: &SignablePayload, label: &str) -> (bool, Vec<String>) {
    let values: Vec<String> = payload
        .fields
        .iter()
        .flat_map(|field| check_signable_payload_field(field, label).1)
        .collect();

    (!values.is_empty(), values)
}

pub fn check_signable_payload_field(
    field: &SignablePayloadField,
    label: &str,
) -> (bool, Vec<String>) {
    let values: Vec<String> = match field {
        SignablePayloadField::Text { common, text } => (common.label == label)
            .then(|| text.text.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::TextV2 { common, text_v2 } => (common.label == label)
            .then(|| text_v2.text.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::Address { common, address } => (common.label == label)
            .then(|| address.address.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::AddressV2 { common, address_v2 } => (common.label == label)
            .then(|| address_v2.address.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::Number { common, number } => (common.label == label)
            .then(|| number.number.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::Amount { common, amount } => (common.label == label)
            .then(|| amount.amount.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::AmountV2 { common, amount_v2 } => (common.label == label)
            .then(|| amount_v2.amount.to_string())
            .into_iter()
            .collect(),
        SignablePayloadField::PreviewLayout {
            preview_layout,
            common,
        } => {
            let fallback = (common.label == label).then(|| common.fallback_text.to_string());

            let nested = preview_layout
                .condensed
                .iter()
                .flat_map(|c| c.fields.iter())
                .chain(preview_layout.expanded.iter().flat_map(|e| e.fields.iter()))
                .flat_map(|f| check_signable_payload_field(&f.signable_payload_field, label).1);

            fallback.into_iter().chain(nested).collect()
        }
        SignablePayloadField::ListLayout {
            list_layout,
            common,
        } => {
            let fallback = (common.label == label).then(|| common.fallback_text.to_string());

            let nested = list_layout
                .fields
                .iter()
                .flat_map(|f| check_signable_payload_field(&f.signable_payload_field, label).1);

            fallback.into_iter().chain(nested).collect()
        }
        _ => Vec::new(),
    };

    (!values.is_empty(), values)
}
