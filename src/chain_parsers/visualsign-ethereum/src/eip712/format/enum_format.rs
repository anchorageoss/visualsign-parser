//! ERC-7730 `enum` format: int/string -> named label.

use crate::eip712::format::RenderError;
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(
    label: &str,
    value: &MessageValue,
    params: &serde_json::Value,
) -> Result<SignablePayloadField, RenderError> {
    let key = match value {
        MessageValue::Uint { value, .. } => value.to_string(),
        MessageValue::String(s) => s.clone(),
        MessageValue::Int { value, .. } => value.to_string(),
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "uint/int/string".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let mapped = params
        .get("enums")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get(&key).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{key} (unknown enum)"));
    Ok(create_text_field(label, &mapped)?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn maps_enum_value() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(2u64),
        };
        let f = render(
            "Status",
            &v,
            &serde_json::json!({"enums": {"0": "Pending", "1": "Active", "2": "Closed"}}),
        )
        .unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => assert_eq!(text_v2.text, "Closed"),
            _ => panic!(),
        }
    }

    #[test]
    fn unknown_value_falls_back() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(99u64),
        };
        let f = render(
            "Status",
            &v,
            &serde_json::json!({"enums": {"0": "Pending"}}),
        )
        .unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "99 (unknown enum)")
            }
            _ => panic!(),
        }
    }
}
