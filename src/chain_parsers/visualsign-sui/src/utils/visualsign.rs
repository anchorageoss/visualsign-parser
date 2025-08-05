use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldCommon,
};

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
