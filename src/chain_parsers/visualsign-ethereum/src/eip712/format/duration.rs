//! ERC-7730 `duration` format: seconds -> human time.

use crate::eip712::format::RenderError;
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(label: &str, value: &MessageValue) -> Result<SignablePayloadField, RenderError> {
    let secs = match value {
        MessageValue::Uint { value, .. } => {
            u64::try_from(*value).map_err(|_| RenderError::TypeMismatch {
                expected: "uint fitting u64 seconds".into(),
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
    Ok(create_text_field(label, &format_duration(secs))?.signable_payload_field)
}

pub fn format_duration(mut secs: u64) -> String {
    let units = [("d", 86_400u64), ("h", 3_600), ("m", 60), ("s", 1)];
    let mut parts = Vec::new();
    for (unit, size) in units {
        let n = secs / size;
        if n > 0 {
            parts.push(format!("{n}{unit}"));
        }
        secs %= size;
        if parts.len() == 2 {
            break;
        }
    }
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn formats_duration_components() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(3_600), "1h");
        assert_eq!(format_duration(3_661), "1h 1m");
        assert_eq!(format_duration(86_400 * 7), "7d");
    }

    #[test]
    fn renders_duration() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(86_400u64),
        };
        let f = render("Lockup", &v).unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => assert_eq!(text_v2.text, "1d"),
            _ => panic!(),
        }
    }
}
