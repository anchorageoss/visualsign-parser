//! ERC-7730 `unit` format: value with named unit + optional decimal scaling.

use crate::eip712::format::{RenderError, format_decimal};
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(
    label: &str,
    value: &MessageValue,
    params: &serde_json::Value,
) -> Result<SignablePayloadField, RenderError> {
    let amt = match value {
        MessageValue::Uint { value, .. } => {
            u128::try_from(*value).map_err(|_| RenderError::TypeMismatch {
                expected: "uint fitting u128".into(),
                actual: format!("uint {value}"),
            })?
        }
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "uint".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let decimals = params.get("decimals").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let base = params.get("base").and_then(|v| v.as_str()).unwrap_or("");
    let scaled = format_decimal(amt, decimals);
    let text = if base.is_empty() {
        scaled
    } else {
        format!("{scaled} {base}")
    };
    Ok(create_text_field(label, &text)?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn renders_unit_with_decimals() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(1_500_000u64),
        };
        let f = render(
            "Gwei",
            &v,
            &serde_json::json!({"base": "gwei", "decimals": 9}),
        )
        .unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "0.0015 gwei")
            }
            _ => panic!(),
        }
    }
}
