//! ERC-7730 `nft` format: collection name + tokenId.

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
    let token_id = match value {
        MessageValue::Uint { value, .. } => value.to_string(),
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "uint".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let collection_path = params
        .get("collectionPath")
        .and_then(|v| v.as_str())
        .ok_or(RenderError::MissingParam("collectionPath"))?;
    let parsed = parse(collection_path).map_err(|e| RenderError::Path(e.to_string()))?;
    let resolved =
        resolve(&parsed, ctx.message, ctx.domain).map_err(|e| RenderError::Path(e.to_string()))?;
    let collection_addr = match resolved.first() {
        Some(MessageValue::Address(a)) => *a,
        _ => {
            return Err(RenderError::InvalidParam {
                name: "collectionPath".into(),
                detail: "did not resolve to address".into(),
            });
        }
    };
    let collection_name = ctx
        .registry
        .and_then(|r| r.get_token_symbol(ctx.chain_id, collection_addr))
        .unwrap_or_else(|| format!("{collection_addr:?}"));
    Ok(create_text_field(label, &format!("{collection_name} #{token_id}"))?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use alloy_primitives::{Address, U256};
    use std::collections::BTreeMap;

    #[test]
    fn renders_nft_with_unknown_collection() {
        let collection = Address::with_last_byte(0xbc);
        let mut fields = BTreeMap::new();
        fields.insert("collection".into(), MessageValue::Address(collection));
        let msg = MessageValue::Struct(fields);
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
            message: &msg,
        };
        let f = render(
            "NFT",
            &MessageValue::Uint {
                bits: 256,
                value: U256::from(1234u64),
            },
            &serde_json::json!({"collectionPath": "#.collection"}),
            &ctx,
        )
        .unwrap();
        match f {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(text_v2.text.ends_with("#1234"));
            }
            _ => panic!(),
        }
    }
}
