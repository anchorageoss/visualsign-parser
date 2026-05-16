//! ERC-7730 `calldata` format: render nested calldata as selector + length summary.
//!
//! Full recursive dispatch into the EthereumVisualSignConverter pipeline is deferred —
//! that requires threading the visualizer registry through this layer. v1 renders a
//! useful summary line (selector + byte length + optional callee name).

use crate::eip712::descriptor::path::{parse, resolve};
use crate::eip712::format::{RenderContext, RenderError};
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_text_field;

pub fn render(
    label: &str,
    value: &MessageValue,
    params: &serde_json::Value,
    ctx: &RenderContext,
) -> Result<SignablePayloadField, RenderError> {
    let bytes = match value {
        MessageValue::Bytes(b) | MessageValue::BytesFixed(b) => b.clone(),
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "bytes".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let selector = if bytes.len() >= 4 {
        Some(hex::encode(&bytes[..4]))
    } else {
        None
    };
    let callee = match params.get("calleePath").and_then(|v| v.as_str()) {
        Some(p) => {
            let parsed = parse(p).map_err(|e| RenderError::Path(e.to_string()))?;
            let resolved = resolve(&parsed, ctx.message, ctx.domain)
                .map_err(|e| RenderError::Path(e.to_string()))?;
            match resolved.first() {
                Some(MessageValue::Address(a)) => Some(*a),
                _ => None,
            }
        }
        None => None,
    };

    let mut text = String::new();
    if let Some(sel) = &selector {
        text.push_str(&format!("selector 0x{sel}, "));
    }
    text.push_str(&format!("{} bytes", bytes.len()));
    if let Some(addr) = callee {
        let name = ctx
            .registry
            .and_then(|r| r.get_token_symbol(ctx.chain_id, addr));
        match name {
            Some(n) => text.push_str(&format!(" -> {n}")),
            None => text.push_str(&format!(" -> {addr:?}")),
        }
    }
    Ok(create_text_field(label, &text)?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use std::collections::BTreeMap;

    #[test]
    fn renders_calldata_summary() {
        let dom = Domain {
            name: None,
            version: None,
            chain_id: Some(1),
            verifying_contract: None,
            salt: None,
        };
        let ctx = RenderContext {
            chain_id: 1,
            registry: None,
            domain: &dom,
            message: &MessageValue::Struct(BTreeMap::new()),
        };
        let v = MessageValue::Bytes(vec![0xa9, 0x05, 0x9c, 0xbb, 0x00, 0x00, 0x00, 0x00]);
        let f = render("Inner call", &v, &serde_json::Value::Null, &ctx).unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(text_v2.text.contains("0xa9059cbb"));
                assert!(text_v2.text.contains("8 bytes"));
            }
            _ => panic!(),
        }
    }
}
