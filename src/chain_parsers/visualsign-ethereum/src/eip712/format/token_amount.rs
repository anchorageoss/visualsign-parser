//! ERC-7730 `tokenAmount` format: resolve token via params.tokenPath, format with decimals.

use crate::eip712::descriptor::path::{parse, resolve};
use crate::eip712::format::{RenderContext, RenderError, format_decimal};
use crate::eip712::payload::MessageValue;
use visualsign::SignablePayloadField;
use visualsign::field_builders::{create_amount_field, create_text_field};

pub fn render(
    label: &str,
    value: &MessageValue,
    params: &serde_json::Value,
    ctx: &RenderContext,
) -> Result<SignablePayloadField, RenderError> {
    let amount_u128 = match value {
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

    let token_path = params
        .get("tokenPath")
        .and_then(|v| v.as_str())
        .ok_or(RenderError::MissingParam("tokenPath"))?;
    let parsed = parse(token_path).map_err(|e| RenderError::Path(e.to_string()))?;
    let resolved =
        resolve(&parsed, ctx.message, ctx.domain).map_err(|e| RenderError::Path(e.to_string()))?;
    let token_addr = match resolved.first() {
        Some(MessageValue::Address(a)) => *a,
        _ => {
            return Err(RenderError::InvalidParam {
                name: "tokenPath".into(),
                detail: "did not resolve to address".into(),
            });
        }
    };

    if let Some(native_addr) = params.get("nativeCurrencyAddress").and_then(|v| v.as_str()) {
        if format!("{token_addr:?}").to_lowercase() == native_addr.to_lowercase() {
            let symbol = crate::networks::get_fee_paying_asset_symbol(ctx.chain_id).unwrap_or("");
            let formatted = format_decimal(amount_u128, 18);
            return Ok(if symbol.is_empty() {
                create_text_field(label, &formatted)?.signable_payload_field
            } else {
                create_amount_field(label, &formatted, symbol)?.signable_payload_field
            });
        }
    }

    if let Some(reg) = ctx.registry {
        if let Some((amount_str, symbol)) =
            reg.format_token_amount(ctx.chain_id, token_addr, amount_u128)
        {
            return Ok(if symbol.is_empty() {
                create_text_field(label, &amount_str)?.signable_payload_field
            } else {
                create_amount_field(label, &amount_str, &symbol)?.signable_payload_field
            });
        }
    }

    // Unknown token: raw decimal + hex address suffix as text.
    let fallback = format!("{amount_u128} (token {token_addr:?})");
    Ok(create_text_field(label, &fallback)?.signable_payload_field)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use alloy_primitives::{Address, U256};
    use std::collections::BTreeMap;

    #[test]
    fn unknown_token_falls_back_to_text() {
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let mut fields = BTreeMap::new();
        fields.insert("token".into(), MessageValue::Address(usdc));
        let msg = MessageValue::Struct(fields);
        let dom = Domain {
            name: None,
            version: None,
            chain_id: Some(1),
            verifying_contract: Some(usdc),
            salt: None,
        };
        let ctx = RenderContext {
            chain_id: 1,
            registry: None,
            domain: &dom,
            message: &msg,
        };
        let f = render(
            "Amount",
            &MessageValue::Uint {
                bits: 256,
                value: U256::from(1_000_000u64),
            },
            // `@` resolves to verifyingContract directly (no further indexing).
            &serde_json::json!({"tokenPath": "@"}),
            &ctx,
        );
        // verifyingContract is USDC; with no registry it falls back to text.
        let _ = f.unwrap();
    }

    #[test]
    fn format_decimal_drops_trailing_zeros() {
        assert_eq!(format_decimal(1_000_000, 6), "1");
        assert_eq!(format_decimal(1_500_000, 6), "1.5");
        assert_eq!(format_decimal(1_500_001, 6), "1.500001");
        assert_eq!(format_decimal(0, 6), "0");
    }
}
