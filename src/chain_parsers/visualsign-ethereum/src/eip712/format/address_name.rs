//! ERC-7730 `addressName` format: resolve address -> name via registry.

use crate::eip712::format::{RenderContext, RenderError};
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::create_address_field;

pub fn render(
    label: &str,
    value: &MessageValue,
    _params: &serde_json::Value,
    ctx: &RenderContext,
) -> Result<SignablePayloadField, RenderError> {
    let addr = match value {
        MessageValue::Address(a) => *a,
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "address".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let name = ctx
        .registry
        .and_then(|r| r.get_token_symbol(ctx.chain_id, addr));
    Ok(create_address_field(
        label,
        &format!("{addr:?}"),
        name.as_deref(),
        None,
        None,
        None,
    )?
    .signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use alloy_primitives::Address;
    use std::collections::BTreeMap;

    #[test]
    fn renders_unknown_address() {
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
        let f = render(
            "Spender",
            &MessageValue::Address(Address::with_last_byte(0x33)),
            &serde_json::Value::Null,
            &ctx,
        )
        .unwrap();
        match f {
            SignablePayloadField::AddressV2 { address_v2, .. } => {
                assert_eq!(
                    address_v2.address.to_lowercase(),
                    "0x0000000000000000000000000000000000000033"
                );
            }
            _ => panic!(),
        }
    }
}
