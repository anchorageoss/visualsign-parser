//! Structured tree walk for EIP-712 payloads with no matching descriptor.

use crate::eip712::payload::{Eip712Payload, MessageValue, TypeMember};
use std::collections::BTreeMap;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;
use visualsign::vsptrait::VisualSignError;

/// Walk the payload's primaryType tree and emit one field per leaf, labelled by dotted path.
pub fn render(payload: &Eip712Payload) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    let mut out = Vec::new();
    walk(
        &payload.primary_type,
        &payload.message,
        &payload.types,
        "",
        &mut out,
    )?;
    Ok(out)
}

fn walk(
    type_name: &str,
    value: &MessageValue,
    types: &BTreeMap<String, Vec<TypeMember>>,
    prefix: &str,
    out: &mut Vec<SignablePayloadField>,
) -> Result<(), VisualSignError> {
    if let Some(inner) = type_name.strip_suffix("[]") {
        if let MessageValue::Array(items) = value {
            for (i, item) in items.iter().enumerate() {
                walk(inner, item, types, &format!("{prefix}[{i}]"), out)?;
            }
        }
        return Ok(());
    }
    if let Some(members) = types.get(type_name) {
        if let MessageValue::Struct(fields) = value {
            for m in members {
                let child_label = if prefix.is_empty() {
                    m.name.clone()
                } else {
                    format!("{prefix}.{}", m.name)
                };
                if let Some(v) = fields.get(&m.name) {
                    walk(&m.r#type, v, types, &child_label, out)?;
                }
            }
        }
        return Ok(());
    }
    let text = crate::eip712::format::raw::stringify_leaf(value);
    out.push(create_text_field(prefix, &text)?.signable_payload_field);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn walks_permit_message() {
        let payload = Eip712Payload::from_json(
            r#"{
            "domain": {"chainId": "0x1", "verifyingContract": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"},
            "primaryType": "Permit",
            "types": {
              "EIP712Domain": [
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
              ],
              "Permit": [
                {"name": "owner", "type": "address"},
                {"name": "value", "type": "uint256"}
              ]
            },
            "message": {"owner": "0x1111111111111111111111111111111111111111", "value": "42"}
          }"#,
        )
        .unwrap();
        let fields = render(&payload).unwrap();
        assert_eq!(fields.len(), 2);
        let labels: Vec<_> = fields
            .iter()
            .map(|f| match f {
                SignablePayloadField::TextV2 { common, .. } => common.label.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(labels, vec!["owner".to_string(), "value".to_string()]);
    }
}
