use base64::Engine;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
};

/// Helper function to create a text field
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

/// Helper function to create an amount field
pub fn create_amount_field(label: &str, amount: &str, abbreviation: &str) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} {}", amount, abbreviation),
                label: label.to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: amount.to_string(),
                abbreviation: Some(abbreviation.to_string()),
            },
        },
    }
}

/// Helper function to create an address field
pub fn create_address_field(
    label: &str,
    address: &str,
    name: Option<&str>,
    memo: Option<&str>,
    asset_label: Option<&str>,
    badge_text: Option<&str>,
) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: address.to_string(),
                label: label.to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: address.to_string(),
                name: name.unwrap_or("").to_string(),
                memo: memo.map(|s| s.to_string()),
                asset_label: asset_label.unwrap_or("").to_string(),
                badge_text: badge_text.map(|s| s.to_string()),
            },
        },
    }
}

/// Helper function to create a simple text field (non-annotated)
pub fn create_simple_text_field(label: &str, text: &str) -> SignablePayloadField {
    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: text.to_string(),
            label: label.to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: text.to_string(),
        },
    }
}

/// Create a standard Raw Data field for expanded views
pub fn create_raw_data_field(data: &[u8]) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: "The raw instruction data in base64 format".to_string(),
                label: "Raw Data".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: base64::engine::general_purpose::STANDARD.encode(data),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }
}
