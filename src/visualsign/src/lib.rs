use serde::{Deserialize, Serialize};
use serde_json::Value;
pub mod encodings;
pub mod field_builders;
pub mod registry;
pub mod vsptrait;

// A function to check if a string is empty (used for skip_serializing_if)
fn is_empty_string(s: &str) -> bool {
    s.is_empty()
}

// A bare bones implementation of the SignablePayload struct and its associated methods
// The fields are serialized alphabetically to ensure that default serialization works the same
// and the canonical representation is done by simply sorting the fields first
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayload {
    #[serde(rename = "Fields")]
    pub fields: Vec<SignablePayloadField>,
    #[serde(rename = "PayloadType", skip_serializing_if = "is_empty_string")]
    pub payload_type: String,
    #[serde(rename = "Subtitle", skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Version")]
    pub version: String,
}

// Common fields shared by all field types
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldCommon {
    #[serde(rename = "FallbackText")]
    pub fallback_text: String,
    #[serde(rename = "Label")]
    pub label: String,
}

// Now SignablePayloadField is an enum with variants for each field type
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "Type")]
pub enum SignablePayloadField {
    #[serde(rename = "text")]
    Text {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Text")]
        text: SignablePayloadFieldText,
    },

    #[serde(rename = "text_v2")]
    TextV2 {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "TextV2")]
        text_v2: SignablePayloadFieldTextV2,
    },

    #[serde(rename = "address")]
    Address {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Address")]
        address: SignablePayloadFieldAddress,
    },

    #[serde(rename = "address_v2")]
    AddressV2 {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "AddressV2")]
        address_v2: SignablePayloadFieldAddressV2,
    },

    #[serde(rename = "number")]
    Number {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Number")]
        number: SignablePayloadFieldNumber,
    },

    #[serde(rename = "amount")]
    Amount {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Amount")]
        amount: SignablePayloadFieldAmount,
    },

    #[serde(rename = "amount_v2")]
    AmountV2 {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "AmountV2")]
        amount_v2: SignablePayloadFieldAmountV2,
    },

    #[serde(rename = "divider")]
    Divider {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Divider")]
        divider: SignablePayloadFieldDivider,
    },

    #[serde(rename = "preview_layout")]
    PreviewLayout {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "PreviewLayout")]
        preview_layout: SignablePayloadFieldPreviewLayout,
    },

    #[serde(rename = "list_layout")]
    ListLayout {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "ListLayout")]
        list_layout: SignablePayloadFieldListLayout,
    },

    #[serde(rename = "unknown")]
    Unknown {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Unknown")]
        unknown: SignablePayloadFieldUnknown,
    },
}

// Helper methods for the enum
impl SignablePayloadField {
    pub fn fallback_text(&self) -> &String {
        match self {
            SignablePayloadField::Text { common, .. } => &common.fallback_text,
            SignablePayloadField::TextV2 { common, .. } => &common.fallback_text,
            SignablePayloadField::Address { common, .. } => &common.fallback_text,
            SignablePayloadField::AddressV2 { common, .. } => &common.fallback_text,
            SignablePayloadField::Number { common, .. } => &common.fallback_text,
            SignablePayloadField::Amount { common, .. } => &common.fallback_text,
            SignablePayloadField::AmountV2 { common, .. } => &common.fallback_text,
            SignablePayloadField::Divider { common, .. } => &common.fallback_text,
            SignablePayloadField::PreviewLayout { common, .. } => &common.fallback_text,
            SignablePayloadField::ListLayout { common, .. } => &common.fallback_text,
            SignablePayloadField::Unknown { common, .. } => &common.fallback_text,
        }
    }

    pub fn label(&self) -> &String {
        match self {
            SignablePayloadField::Text { common, .. } => &common.label,
            SignablePayloadField::TextV2 { common, .. } => &common.label,
            SignablePayloadField::Address { common, .. } => &common.label,
            SignablePayloadField::AddressV2 { common, .. } => &common.label,
            SignablePayloadField::Number { common, .. } => &common.label,
            SignablePayloadField::Amount { common, .. } => &common.label,
            SignablePayloadField::AmountV2 { common, .. } => &common.label,
            SignablePayloadField::Divider { common, .. } => &common.label,
            SignablePayloadField::PreviewLayout { common, .. } => &common.label,
            SignablePayloadField::ListLayout { common, .. } => &common.label,
            SignablePayloadField::Unknown { common, .. } => &common.label,
        }
    }

    pub fn field_type(&self) -> &str {
        match self {
            SignablePayloadField::Text { .. } => "text",
            SignablePayloadField::TextV2 { .. } => "text_v2",
            SignablePayloadField::Address { .. } => "address",
            SignablePayloadField::AddressV2 { .. } => "address_v2",
            SignablePayloadField::Number { .. } => "number",
            SignablePayloadField::Amount { .. } => "amount",
            SignablePayloadField::AmountV2 { .. } => "amount_v2",
            SignablePayloadField::Divider { .. } => "divider",
            SignablePayloadField::PreviewLayout { .. } => "preview_layout",
            SignablePayloadField::ListLayout { .. } => "list_layout",
            SignablePayloadField::Unknown { .. } => "unknown",
        }
    }
}

// Update all struct definitions to use String instead of NormalString
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldPreviewLayout {
    #[serde(rename = "Title", skip_serializing_if = "Option::is_none")]
    pub title: Option<SignablePayloadFieldTextV2>,
    #[serde(rename = "Subtitle", skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<SignablePayloadFieldTextV2>,
    #[serde(rename = "Condensed", skip_serializing_if = "Option::is_none")]
    pub condensed: Option<SignablePayloadFieldListLayout>,
    #[serde(rename = "Expanded", skip_serializing_if = "Option::is_none")]
    pub expanded: Option<SignablePayloadFieldListLayout>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldListLayout {
    #[serde(rename = "Fields")]
    pub fields: Vec<AnnotatedPayloadField>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldText {
    #[serde(rename = "Text")]
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldTextV2 {
    #[serde(rename = "Text")]
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldAddress {
    #[serde(rename = "Address")]
    pub address: String,
    #[serde(rename = "Name")]
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldAddressV2 {
    #[serde(rename = "Address")]
    pub address: String,
    #[serde(rename = "Name", skip_serializing_if = "is_empty_string")]
    pub name: String,
    #[serde(rename = "Memo", skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(rename = "AssetLabel", skip_serializing_if = "is_empty_string")]
    pub asset_label: String,
    #[serde(rename = "BadgeText", skip_serializing_if = "Option::is_none")]
    pub badge_text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldNumber {
    #[serde(rename = "Number")]
    pub number: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldAmount {
    #[serde(rename = "Amount")]
    pub amount: String,
    #[serde(rename = "Abbreviation", skip_serializing_if = "Option::is_none")]
    pub abbreviation: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldAmountV2 {
    #[serde(rename = "Amount")]
    pub amount: String,
    #[serde(rename = "Abbreviation", skip_serializing_if = "Option::is_none")]
    pub abbreviation: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldDivider {
    #[serde(rename = "Style")]
    pub style: DividerStyle,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldUnknown {
    #[serde(rename = "Data")]
    pub data: String,
    #[serde(rename = "Explanation")]
    pub explanation: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldStaticAnnotation {
    #[serde(rename = "Text")]
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldDynamicAnnotation {
    #[serde(rename = "Type")]
    pub field_type: String,
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Params")]
    pub params: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedPayload {
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "Title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "Subtitle", skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(rename = "Fields", skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<AnnotatedPayloadField>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedPayloadField {
    #[serde(flatten)]
    pub signable_payload_field: SignablePayloadField,
    #[serde(rename = "StaticAnnotation", skip_serializing_if = "Option::is_none")]
    pub static_annotation: Option<SignablePayloadFieldStaticAnnotation>,
    #[serde(rename = "DynamicAnnotation", skip_serializing_if = "Option::is_none")]
    pub dynamic_annotation: Option<SignablePayloadFieldDynamicAnnotation>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UserIntent {
    #[serde(rename = "Type")]
    pub intent_type: String,
    #[serde(rename = "Payload")]
    pub payload: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DividerStyle(String);

impl DividerStyle {
    pub const THIN: DividerStyle = DividerStyle(String::new());
}

impl SignablePayload {
    pub fn new(
        version: i64,
        title: String,
        subtitle: Option<String>,
        fields: Vec<SignablePayloadField>,
        payload_type: String,
    ) -> Self {
        SignablePayload {
            version: version.to_string(),
            title,
            subtitle,
            payload_type,
            fields,
        }
    }

    pub fn to_json(&self) -> Result<String, Box<dyn std::error::Error>> {
        // First convert to a standard JSON value
        let value = serde_json::to_value(self)?;

        // Convert to a completely new object with alphabetically sorted keys
        let sorted_value = sort_json_alphabetically(value);

        // Serialize without pretty-printing and without escape HTML
        let mut buf = Vec::new();
        let formatter = serde_json::ser::CompactFormatter;
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
        sorted_value.serialize(&mut ser)?;

        // Convert bytes to string
        Ok(String::from_utf8(buf)?)
    }

    // Add this method for debugging
    pub fn to_pretty_json(&self) -> Result<String, Box<dyn std::error::Error>> {
        let value = serde_json::to_value(self)?;
        let sorted_value = sort_json_alphabetically(value);
        Ok(serde_json::to_string_pretty(&sorted_value)?)
    }
}

// Helper function to recursively sort JSON by keys alphabetically
fn sort_json_alphabetically(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            // Create a BTreeMap (which is sorted by keys)
            let mut sorted_map = std::collections::BTreeMap::new();

            // Insert all entries, recursively sorting nested objects
            for (key, val) in map {
                sorted_map.insert(key, sort_json_alphabetically(val));
            }

            // Convert back to serde_json::Value
            serde_json::Value::Object(serde_json::Map::from_iter(sorted_map))
        }
        serde_json::Value::Array(arr) => {
            // Recursively sort array elements (if they are objects)
            serde_json::Value::Array(arr.into_iter().map(sort_json_alphabetically).collect())
        }
        // Other value types (string, number, boolean, null) don't need sorting
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn test_signable_payload_to_json() {
        let fields = vec![
            SignablePayloadField::Text {
                common: SignablePayloadFieldCommon {
                    fallback_text: "FallbackText1".to_string(),
                    label: "Label1".to_string(),
                },
                text: SignablePayloadFieldText {
                    text: "Text1".to_string(),
                },
            },
            SignablePayloadField::Text {
                common: SignablePayloadFieldCommon {
                    fallback_text: "FallbackText2".to_string(),
                    label: "Label2".to_string(),
                },
                text: SignablePayloadFieldText {
                    text: "Text2".to_string(),
                },
            },
        ];

        let payload = SignablePayload::new(
            1,
            "Test Title".to_string(),
            Some("Test Subtitle".to_string()),
            fields,
            "Test Payload Type".to_string(),
        );

        let json = payload.to_json().unwrap();
        println!("{}", json);
    }

    #[test]
    fn test_eth_user_intent_equivalence() {
        // this is a relatively lazy attempt to keep this consistent with the Go implementation at
        let from_address = "0xYourFromAddress";

        let fields = vec![
            SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Ethereum Regnet".to_string(),
                    label: "Network".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Ethereum Regnet".to_string(),
                },
            },
            SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: from_address.to_string(),
                    label: "From".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: from_address.to_string(),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "0xb06E442b696513d54B05b5De58494E902E6e08Cb".to_string(),
                    label: "Contract Address".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: "0xb06E442b696513d54B05b5De58494E902E6e08Cb".to_string(),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "0x00".to_string(),
                    label: "Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "0x00".to_string(),
                },
            },
            SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "0 ETH_R".to_string(),
                    label: "Value".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: "0".to_string(),
                    abbreviation: Some("ETH_R".to_string()),
                },
            },
            SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "0.000000000000000004 ETH_R".to_string(),
                    label: "Max Fee".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: "0.000000000000000004".to_string(),
                    abbreviation: Some("ETH_R".to_string()),
                },
            },
        ];

        let payload =
            SignablePayload::new(15, "Withdraw".to_string(), None, fields, "".to_string());

        let json = payload.to_json().unwrap();
        println!("{}", json);

        let expected_json = json!({
            "Version": "15",
            "Title": "Withdraw",
            "Fields": [
                {
                    "FallbackText": "Ethereum Regnet",
                    "Type": "text_v2",
                    "Label": "Network",
                    "TextV2": {
                        "Text": "Ethereum Regnet"
                    }
                },
                {
                    "FallbackText": "0xYourFromAddress",
                    "Type": "address_v2",
                    "Label": "From",
                    "AddressV2": {
                        "Address": "0xYourFromAddress"
                    }
                },
                {
                    "FallbackText": "0xb06E442b696513d54B05b5De58494E902E6e08Cb",
                    "Type": "address_v2",
                    "Label": "Contract Address",
                    "AddressV2": {
                        "Address": "0xb06E442b696513d54B05b5De58494E902E6e08Cb"
                    }
                },
                {
                    "FallbackText": "0x00",
                    "Type": "text_v2",
                    "Label": "Data",
                    "TextV2": {
                        "Text": "0x00"
                    }
                },
                {
                    "FallbackText": "0 ETH_R",
                    "Type": "amount_v2",
                    "Label": "Value",
                    "AmountV2": {
                        "Amount": "0",
                        "Abbreviation": "ETH_R"
                    }
                },
                {
                    "FallbackText": "0.000000000000000004 ETH_R",
                    "Type": "amount_v2",
                    "Label": "Max Fee",
                    "AmountV2": {
                        "Amount": "0.000000000000000004",
                        "Abbreviation": "ETH_R"
                    }
                }
            ]
        });

        let generated_json: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(generated_json, expected_json);
    }

    #[test]
    fn test_alphabetical_sorting() {
        let payload = SignablePayload::new(
            1,
            "Z_Title".to_string(),          // Starts with Z
            Some("A_Subtitle".to_string()), // Starts with A
            vec![],                         // Empty fields
            "M_PayloadType".to_string(),    // Starts with M
        );

        let json = payload.to_json().unwrap();
        assert_sorted_alphabetically(json);

        // Lets try using serde_json to serialize the payload and ensure that ordering is still same
        // this is more to ensure that engineer isn't changing the order of fields
        let serde_default_json = serde_json::to_string(&payload).unwrap();
        assert_sorted_alphabetically(serde_default_json);
    }

    fn assert_sorted_alphabetically(json: String) {
        println!("Sorted JSON: {}", json);
        // ensure that ordering si preserved when using to_json()
        let pos_fields = json.find("Fields").unwrap_or(0);
        let pos_payload = json.find("PayloadType").unwrap_or(0);
        let pos_subtitle = json.find("Subtitle").unwrap_or(0);
        let pos_title = json.find("Title").unwrap_or(0);
        let pos_version = json.find("Version").unwrap_or(0);

        assert!(
            pos_fields < pos_payload,
            "Fields should come before PayloadType"
        );
        assert!(
            pos_payload < pos_subtitle,
            "PayloadType should come before Subtitle"
        );
        assert!(
            pos_subtitle < pos_title,
            "Subtitle should come before Title"
        );
        assert!(pos_title < pos_version, "Title should come before Version");
    }
}
