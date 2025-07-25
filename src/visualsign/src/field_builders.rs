use crate::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAmountV2,
    SignablePayloadFieldCommon, SignablePayloadFieldNumber, SignablePayloadFieldTextV2,
};

pub fn create_text_field(label: &str, text: &str) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.to_string(),
                label: label.to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: text.to_string(),
            },
        },
    }
}

pub fn create_number_field(label: &str, number: &str, unit: &str) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::Number {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} {}", number, unit),
                label: label.to_string(),
            },
            number: SignablePayloadFieldNumber {
                number: number.to_string(),
            },
        },
    }
}

pub fn create_amount_field(
    label: &str,
    amount: &str,
    abbreviation: &str,
    value: f64,
    asset_type: &str,
) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} {}", value, asset_type),
                label: label.to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: amount.to_string(),
                abbreviation: Some(abbreviation.to_string()),
            },
        },
    }
}
