//! ERC-7730 `raw` format: render any leaf as text.

use crate::eip712::format::RenderError;
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(label: &str, value: &MessageValue) -> Result<SignablePayloadField, RenderError> {
    let text = stringify_leaf(value);
    Ok(create_text_field(label, &text)?.signable_payload_field)
}

/// Render a leaf `MessageValue` as a human-readable string. Used by `raw` and by the
/// structured tree-walk fallback.
pub(crate) fn stringify_leaf(v: &MessageValue) -> String {
    match v {
        MessageValue::Address(a) => format!("{a:?}"), // EIP-55 checksum from alloy Debug
        MessageValue::Bool(b) => b.to_string(),
        MessageValue::Bytes(b) | MessageValue::BytesFixed(b) => format!("0x{}", hex::encode(b)),
        MessageValue::Int { value, .. } => value.to_string(),
        MessageValue::Uint { value, .. } => value.to_string(),
        MessageValue::String(s) => s.clone(),
        MessageValue::Array(_) | MessageValue::Struct(_) => "<complex>".into(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256};

    #[test]
    fn renders_address_checksummed() {
        let v = MessageValue::Address(Address::with_last_byte(0xab));
        let f = render("To", &v).unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(text_v2.text.starts_with("0x"));
                assert_eq!(
                    text_v2.text.to_lowercase(),
                    "0x00000000000000000000000000000000000000ab"
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn renders_uint_decimal() {
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(42u64),
        };
        let f = render("X", &v).unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => assert_eq!(text_v2.text, "42"),
            _ => panic!(),
        }
    }
}
