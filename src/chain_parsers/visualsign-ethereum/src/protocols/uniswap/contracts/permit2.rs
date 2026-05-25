//! Permit2 Contract Visualizer
//!
//! Permit2 is Uniswap's token approval system that allows signature-based approvals
//! and transfers, improving UX by batching operations.
//!
//! Reference: <https://github.com/Uniswap/permit2>

#![allow(unused_imports)]

use alloy_primitives::{Address, U160};
use alloy_sol_types::{SolCall, sol};
use chrono::{TimeZone, Utc};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::{ContractRegistry, ContractType};

// Permit2 interface (simplified)
sol! {
    interface IPermit2 {
        function approve(address token, address spender, uint160 amount, uint48 expiration) external;
        function permit(address owner, PermitSingle calldata permitSingle, bytes calldata signature) external;
        function transferFrom(address from, address to, uint160 amount, address token) external;
    }

    struct PermitSingle {
        PermitDetails details;
        address spender;
        uint256 sigDeadline;
    }

    struct PermitDetails {
        address token;
        uint160 amount;
        uint48 expiration;
        uint48 nonce;
    }
}

/// Formats a Unix timestamp (seconds since epoch) for display.
///
/// Used for both `uint48` fields (`expiration`) and the `uint256`
/// `sigDeadline` (after a checked narrowing to `u64`), so this helper is
/// intentionally type-agnostic over the source width and operates on a
/// plain `u64`.
///
/// Behavior (out-of-range values fall back to a raw `"unix:<value>"`
/// rendering rather than panicking or numerically clamping; the underlying
/// timestamp passed to consumers is unchanged):
/// - `u64::MAX` is treated as a "never" sentinel.
/// - Values inside chrono's representable range render as
///   `"YYYY-MM-DD HH:MM UTC"`.
/// - Values above chrono's max year (year 9999), which `uint48` can reach
///   (max ~year 8,925,512), fall back to `"unix:<value>"`.
fn format_unix_timestamp_seconds_u64(value: u64) -> String {
    if value == u64::MAX {
        return "never".to_string();
    }

    let signed = match i64::try_from(value) {
        Ok(v) => v,
        Err(_) => return format!("unix:{value}"),
    };

    match Utc.timestamp_opt(signed, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => format!("unix:{value}"),
    }
}

/// Visualizer for Permit2 contract calls
///
/// Permit2 address: 0x000000000022D473030F116dDEE9F6B43aC78BA3
/// (deployed at the same address across all chains)
pub struct Permit2Visualizer;

impl Permit2Visualizer {
    /// Attempts to decode and visualize Permit2 function calls
    ///
    /// # Arguments
    /// * `input` - The calldata bytes (with 4-byte function selector)
    /// * `chain_id` - The chain ID for token lookups
    /// * `registry` - Optional contract registry for token metadata
    ///
    /// # Returns
    /// * `Some(field)` if a recognized Permit2 function is found
    /// * `None` if the input doesn't match any Permit2 function
    pub fn visualize_tx_commands(
        &self,
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        // Try to decode as approve
        if let Ok(call) = IPermit2::approveCall::abi_decode(input) {
            return Some(Self::decode_approve(call, chain_id, registry));
        }

        // Try to decode as permit (standard ABI)
        if let Ok(call) = IPermit2::permitCall::abi_decode(input) {
            return Some(Self::decode_permit(call, chain_id, registry));
        }

        // Try custom permit encoding (used by Universal Router)
        if let Ok(params) = Self::decode_custom_permit_params(input) {
            let call = IPermit2::permitCall {
                owner: Address::ZERO,
                permitSingle: params,
                signature: alloy_primitives::Bytes::default(),
            };
            return Some(Self::decode_permit(call, chain_id, registry));
        }

        // Try to decode as transferFrom
        if let Ok(call) = IPermit2::transferFromCall::abi_decode(input) {
            return Some(Self::decode_transfer_from(call, chain_id, registry));
        }

        None
    }

    /// Decodes custom permit parameter layout (used by Uniswap Universal Router)
    /// Universal Router encodes PermitSingle as inline 192 bytes (no ABI encoding with offsets)
    pub(crate) fn decode_custom_permit_params(
        bytes: &[u8],
    ) -> Result<PermitSingle, Box<dyn std::error::Error>> {
        use alloy_sol_types::SolValue;

        if bytes.len() < 192 {
            return Err("bytes too short for PermitSingle (need 192 bytes minimum)".into());
        }

        // Extract the 192-byte inline struct and decode as PermitSingle
        let permit_single_bytes = &bytes[0..192];
        PermitSingle::abi_decode(permit_single_bytes)
            .map_err(|e| format!("Failed to decode PermitSingle: {e}").into())
    }

    /// Decodes approve function call
    fn decode_approve(
        call: IPermit2::approveCall,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.token))
            .unwrap_or_else(|| format!("{:?}", call.token));

        // Format amount with proper decimals
        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.token, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        // Format expiration timestamp
        let expiration_u64: u64 = call.expiration.to_string().parse().unwrap_or(0);
        let expiration_str = format_unix_timestamp_seconds_u64(expiration_u64);

        let text = format!(
            "Approve {} {} {} to spend {} (expires: {})",
            call.spender, amount_str, token_symbol, token_symbol, expiration_str
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Permit2 Approve".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes permit function call
    fn decode_permit(
        call: IPermit2::permitCall,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let token = call.permitSingle.details.token;
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token))
            .unwrap_or_else(|| format!("{token:?}"));

        // Format amount with proper decimals
        let amount_u128: u128 = call
            .permitSingle
            .details
            .amount
            .to_string()
            .parse()
            .unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token, amount_u128))
            .unwrap_or_else(|| {
                (
                    call.permitSingle.details.amount.to_string(),
                    token_symbol.clone(),
                )
            });

        // Format expiration timestamp
        let expiration_u64: u64 = call
            .permitSingle
            .details
            .expiration
            .to_string()
            .parse()
            .unwrap_or(0);
        let expiration_str = format_unix_timestamp_seconds_u64(expiration_u64);

        // Format sig deadline timestamp. `sigDeadline` is `uint256`; use a
        // checked narrowing so a value above `u64::MAX` renders as
        // `unix:<original>` rather than silently collapsing to 0 (which
        // would display as the Unix epoch).
        let sig_deadline_str = match u64::try_from(call.permitSingle.sigDeadline) {
            Ok(v) => format_unix_timestamp_seconds_u64(v),
            Err(_) => format!("unix:{}", call.permitSingle.sigDeadline),
        };

        // Determine if amount is "unlimited" (max u160)
        let amount_display = if call.permitSingle.details.amount == U160::MAX {
            "Unlimited Amount".to_string()
        } else {
            amount_str.clone()
        };

        let token_lowercase = token.to_string().to_lowercase();
        let subtitle_text = format!(
            "Permit {} to spend {} of {}",
            call.permitSingle.spender, amount_display, token_lowercase
        );

        let title_text = "Permit2 Permit".to_string();

        // Build expanded fields
        let expanded_fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_lowercase.clone(),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_lowercase.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: call.permitSingle.details.amount.to_string(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: call.permitSingle.details.amount.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: call.permitSingle.spender.to_string().to_lowercase(),
                        label: "Spender".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: call.permitSingle.spender.to_string().to_lowercase(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: expiration_str.clone(),
                        label: "Expires".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: expiration_str,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: sig_deadline_str.clone(),
                        label: "Sig Deadline".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: sig_deadline_str,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: subtitle_text.clone(),
                label: title_text.clone(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: subtitle_text,
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout {
                    fields: expanded_fields,
                }),
            },
        }
    }

    /// Decodes transferFrom function call
    fn decode_transfer_from(
        call: IPermit2::transferFromCall,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.token))
            .unwrap_or_else(|| format!("{:?}", call.token));

        // Format amount with proper decimals
        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.token, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        let text = format!(
            "Transfer {} {} from {} to {}",
            amount_str, token_symbol, call.from, call.to
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Permit2 Transfer".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }
}

/// CalldataVisualizer implementation for Permit2
/// Allows delegating calldata directly to Permit2Visualizer
impl crate::visualizer::CalldataVisualizer for Permit2Visualizer {
    fn visualize_calldata(
        &self,
        calldata: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<visualsign::SignablePayloadField> {
        self.visualize_tx_commands(calldata, chain_id, registry)
    }
}

/// ContractVisualizer implementation for Permit2
pub struct Permit2ContractVisualizer {
    inner: Permit2Visualizer,
}

impl Permit2ContractVisualizer {
    pub fn new() -> Self {
        Self {
            inner: Permit2Visualizer,
        }
    }
}

impl Default for Permit2ContractVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::visualizer::ContractVisualizer for Permit2ContractVisualizer {
    fn contract_type(&self) -> &str {
        crate::protocols::uniswap::config::Permit2Contract::short_type_id()
    }

    fn visualize(
        &self,
        context: &crate::context::VisualizerContext,
    ) -> Result<Option<Vec<visualsign::AnnotatedPayloadField>>, visualsign::vsptrait::VisualSignError>
    {
        let (contract_registry, _visualizer_builder) =
            crate::registry::ContractRegistry::with_default_protocols();

        if let Some(field) = self.inner.visualize_tx_commands(
            &context.calldata,
            context.chain_id,
            Some(&contract_registry),
        ) {
            let annotated = visualsign::AnnotatedPayloadField {
                signable_payload_field: field,
                static_annotation: None,
                dynamic_annotation: None,
            };

            Ok(Some(vec![annotated]))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use alloy_primitives::U256;
    use alloy_primitives::aliases::U48;
    use alloy_sol_types::SolCall;

    use super::*;

    #[test]
    fn test_visualize_empty_input() {
        let visualizer = Permit2Visualizer;
        assert_eq!(visualizer.visualize_tx_commands(&[], 1, None), None);
    }

    #[test]
    fn test_visualize_too_short() {
        let visualizer = Permit2Visualizer;
        assert_eq!(
            visualizer.visualize_tx_commands(&[0x01, 0x02], 1, None),
            None
        );
    }

    #[test]
    fn test_format_unix_timestamp_seconds_u64_never() {
        // u64::MAX should be the "never" sentinel.
        assert_eq!(format_unix_timestamp_seconds_u64(u64::MAX), "never");
    }

    #[test]
    fn test_format_unix_timestamp_seconds_u64_normal() {
        // 2024-01-01T00:00:00 UTC = 1704067200.
        assert_eq!(
            format_unix_timestamp_seconds_u64(1_704_067_200),
            "2024-01-01 00:00 UTC"
        );
    }

    #[test]
    fn test_format_unix_timestamp_seconds_u64_epoch() {
        assert_eq!(format_unix_timestamp_seconds_u64(0), "1970-01-01 00:00 UTC");
    }

    #[test]
    fn test_format_unix_timestamp_seconds_u64_uint48_max_does_not_panic() {
        // uint48 max = 2^48 - 1 = 281_474_976_710_655, which corresponds to
        // ~year 8,925,512, well beyond chrono's max year (9999). The old
        // implementation panicked here via unwrap(); the new helper must
        // return a non-panicking representation.
        let uint48_max: u64 = (1u64 << 48) - 1;
        let formatted = format_unix_timestamp_seconds_u64(uint48_max);
        assert_eq!(formatted, format!("unix:{uint48_max}"));
    }

    #[test]
    fn test_visualize_approve_with_uint48_max_expiration_does_not_panic() {
        // Build a Permit2 approve call with expiration = uint48 max.
        let uint48_max = U48::from((1u64 << 48) - 1);
        let call = IPermit2::approveCall {
            token: [0x11u8; 20].into(),
            spender: [0x22u8; 20].into(),
            amount: U160::from(1_000u64),
            expiration: uint48_max,
        };
        let input = IPermit2::approveCall::abi_encode(&call);

        let visualizer = Permit2Visualizer;
        // Must not panic.
        let field = visualizer
            .visualize_tx_commands(&input, 1, None)
            .expect("approve should decode");

        // Sanity check that the rendered text contains the out-of-range
        // fallback (`unix:<value>`) rather than a chrono-formatted date.
        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("unix:281474976710655"),
                    "expected unix:<value> in output, got: {}",
                    text_v2.text
                );
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn test_visualize_permit_with_uint48_max_expiration_does_not_panic() {
        // Build a Permit2 permit call with expiration and sigDeadline at
        // their respective maxes to exercise both formatting paths.
        let uint48_max = U48::from((1u64 << 48) - 1);
        let permit_single = PermitSingle {
            details: PermitDetails {
                token: [0x33u8; 20].into(),
                amount: U160::from(1_000u64),
                expiration: uint48_max,
                nonce: U48::from(0u64),
            },
            spender: [0x44u8; 20].into(),
            sigDeadline: U256::from(u64::MAX),
        };
        let call = IPermit2::permitCall {
            owner: [0x55u8; 20].into(),
            permitSingle: permit_single,
            signature: alloy_primitives::Bytes::default(),
        };
        let input = IPermit2::permitCall::abi_encode(&call);

        let visualizer = Permit2Visualizer;
        // Must not panic.
        let _field = visualizer
            .visualize_tx_commands(&input, 1, None)
            .expect("permit should decode");
    }

    #[test]
    fn test_visualize_permit_with_sig_deadline_above_u64_max_renders_fallback() {
        // `sigDeadline` is `uint256`; values above `u64::MAX` previously
        // collapsed to 0 via `to_string().parse::<u64>().unwrap_or(0)` and
        // rendered as the Unix epoch. They should now render the original
        // value via the `unix:<value>` fallback.
        let big_deadline = U256::from(u64::MAX) + U256::from(1u64);
        let permit_single = PermitSingle {
            details: PermitDetails {
                token: [0x33u8; 20].into(),
                amount: U160::from(1_000u64),
                expiration: U48::from(1_704_067_200u64),
                nonce: U48::from(0u64),
            },
            spender: [0x44u8; 20].into(),
            sigDeadline: big_deadline,
        };
        let call = IPermit2::permitCall {
            owner: [0x55u8; 20].into(),
            permitSingle: permit_single,
            signature: alloy_primitives::Bytes::default(),
        };
        let input = IPermit2::permitCall::abi_encode(&call);

        let visualizer = Permit2Visualizer;
        let field = visualizer
            .visualize_tx_commands(&input, 1, None)
            .expect("permit should decode");

        // The expanded fields include a "Sig Deadline" entry; the preview/
        // title text doesn't carry it. Walk the field tree and assert the
        // fallback rendering shows the original U256 string.
        let expected = format!("unix:{big_deadline}");
        let json = serde_json::to_string(&field).expect("serializable");
        assert!(
            json.contains(&expected),
            "expected `{expected}` in rendered field, got: {json}"
        );
        // And no 1970 epoch rendering, which would be the old broken
        // behavior.
        assert!(
            !json.contains("1970-01-01"),
            "did not expect 1970 epoch fallback in rendered field, got: {json}"
        );
    }
}
