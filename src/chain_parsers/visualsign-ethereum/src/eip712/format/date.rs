//! ERC-7730 `date` format: timestamp or blockheight.

use crate::eip712::format::RenderError;
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(
    label: &str,
    value: &MessageValue,
    params: &serde_json::Value,
) -> Result<SignablePayloadField, RenderError> {
    let n = match value {
        MessageValue::Uint { value, .. } => {
            u128::try_from(*value).map_err(|_| RenderError::TypeMismatch {
                expected: "uint fitting u128 seconds".into(),
                actual: format!("uint {value}"),
            })?
        }
        MessageValue::Int { value, .. } => {
            let i = i128::try_from(*value).map_err(|_| RenderError::TypeMismatch {
                expected: "int fitting i128 seconds".into(),
                actual: format!("int {value}"),
            })?;
            u128::try_from(i.max(0)).unwrap_or(0)
        }
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "uint/int".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let encoding = params
        .get("encoding")
        .and_then(|v| v.as_str())
        .unwrap_or("timestamp");
    let text = match encoding {
        "timestamp" => {
            let secs = i64::try_from(n).map_err(|_| RenderError::TypeMismatch {
                expected: "timestamp fitting i64 seconds".into(),
                actual: format!("uint {n}"),
            })?;
            match chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0) {
                Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                None => format!("timestamp {n}"),
            }
        }
        "blockheight" => format!("block {n}"),
        other => {
            return Err(RenderError::InvalidParam {
                name: "encoding".into(),
                detail: format!("unknown: {other}"),
            });
        }
    };
    Ok(create_text_field(label, &text)?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn renders_timestamp() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(1_700_000_000u64),
        };
        let f = render(
            "Valid until",
            &v,
            &serde_json::json!({"encoding": "timestamp"}),
        )
        .unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(text_v2.text.starts_with("2023-"));
                assert!(text_v2.text.ends_with('Z'));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn renders_blockheight() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(19_000_000u64),
        };
        let f = render("Block", &v, &serde_json::json!({"encoding": "blockheight"})).unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "block 19000000")
            }
            _ => panic!(),
        }
    }
}
