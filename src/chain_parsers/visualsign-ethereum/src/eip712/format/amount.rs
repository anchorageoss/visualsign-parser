//! ERC-7730 `amount` format: native chain currency, 18 decimals by default.

use crate::eip712::format::{RenderContext, RenderError, format_decimal};
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::{create_amount_field, create_text_field};

pub fn render(
    label: &str,
    value: &MessageValue,
    _params: &serde_json::Value,
    ctx: &RenderContext,
) -> Result<SignablePayloadField, RenderError> {
    let amt = match value {
        MessageValue::Uint { value, .. } => {
            u128::try_from(*value).map_err(|_| RenderError::TypeMismatch {
                expected: "uint <= 128 bits".into(),
                actual: "uint > 128 bits".into(),
            })?
        }
        other => {
            return Err(RenderError::TypeMismatch {
                expected: "uint".into(),
                actual: format!("{other:?}"),
            });
        }
    };
    let symbol = crate::networks::get_fee_paying_asset_symbol(ctx.chain_id).unwrap_or("");
    let formatted = format_decimal(amt, 18);
    if symbol.is_empty() {
        // create_amount_field requires a non-empty abbreviation; fall back to text.
        Ok(create_text_field(label, &formatted)?.signable_payload_field)
    } else {
        Ok(create_amount_field(label, &formatted, symbol)?.signable_payload_field)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use alloy_primitives::U256;
    use std::collections::BTreeMap;

    #[test]
    fn renders_native_amount() {
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
        let v = MessageValue::Uint {
            bits: 256,
            value: U256::from(1_000_000_000_000_000_000u128),
        };
        let f = render("Value", &v, &serde_json::Value::Null, &ctx).unwrap();
        match f {
            SignablePayloadField::AmountV2 { amount_v2, .. } => {
                assert_eq!(amount_v2.amount, "1");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("ETH"));
            }
            _ => panic!("expected AmountV2"),
        }
    }
}
