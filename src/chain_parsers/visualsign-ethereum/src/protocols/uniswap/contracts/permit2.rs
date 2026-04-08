//! Permit2 Contract Visualizer
//!
//! Permit2 is Uniswap's token approval system that allows signature-based approvals
//! and transfers, improving UX by batching operations.
//!
//! Reference: <https://github.com/Uniswap/permit2>

#![allow(unused_imports)] // SolCall used implicitly via abi_decode trait methods

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

    /// Max uint48 value — Permit2 uses this to mean "never expires"
    const U48_MAX: u64 = (1u64 << 48) - 1;

    /// Safely format a timestamp, handling edge cases
    fn format_timestamp(value_u64: u64) -> String {
        if value_u64 == 0 {
            return "epoch (1970-01-01)".to_string();
        }
        // Check for overflow when casting to i64
        if let Ok(ts) = i64::try_from(value_u64) {
            if let Some(dt) = Utc.timestamp_opt(ts, 0).single() {
                return dt.format("%Y-%m-%d %H:%M UTC").to_string();
            }
        }
        format!("timestamp {value_u64}")
    }

    /// Format a uint48 expiration, treating max-uint48 as "never"
    fn format_expiration_u48(value: u64) -> String {
        if value == Self::U48_MAX {
            "never".to_string()
        } else {
            Self::format_timestamp(value)
        }
    }

    /// Format a uint256 deadline, treating very large values gracefully.
    /// Accepts a string because uint256 may not fit in u64.
    fn format_deadline_u256(raw: &str) -> String {
        match raw.parse::<u64>() {
            Ok(v) if v == u64::MAX => "never".to_string(),
            Ok(v) => Self::format_timestamp(v),
            Err(_) => "never".to_string(), // Value > u64::MAX is effectively infinite
        }
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

        // Format amount with proper decimals — avoid silent U160→u128 overflow
        let amount_str = match call.amount.to_string().parse::<u128>() {
            Ok(amount_u128) => registry
                .and_then(|r| r.format_token_amount(chain_id, call.token, amount_u128))
                .map(|(s, _)| s)
                .unwrap_or_else(|| call.amount.to_string()),
            Err(_) => call.amount.to_string(),
        };

        // Format expiration timestamp (uint48 — max means "never")
        let expiration_str = Self::format_expiration_u48(call.expiration.to::<u64>());

        let text = format!(
            "Approve {} to spend {} {} (expires: {})",
            call.spender, amount_str, token_symbol, expiration_str
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

        // Format amount with proper decimals — avoid silent U160→u128 overflow
        let amount_str = match call.permitSingle.details.amount.to_string().parse::<u128>() {
            Ok(amount_u128) => registry
                .and_then(|r| r.format_token_amount(chain_id, token, amount_u128))
                .map(|(s, _)| s)
                .unwrap_or_else(|| call.permitSingle.details.amount.to_string()),
            Err(_) => call.permitSingle.details.amount.to_string(),
        };

        // Format expiration timestamp (uint48 — max means "never")
        let expiration_str =
            Self::format_expiration_u48(call.permitSingle.details.expiration.to::<u64>());

        // Format sig deadline timestamp (uint256 — values > u64::MAX treated as "never")
        let sig_deadline_str =
            Self::format_deadline_u256(&call.permitSingle.sigDeadline.to_string());

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
                        fallback_text: format!("{amount_str} {token_symbol}"),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{amount_str} {token_symbol}"),
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

        // Format amount with proper decimals — avoid silent U160→u128 overflow
        let amount_str = match call.amount.to_string().parse::<u128>() {
            Ok(amount_u128) => registry
                .and_then(|r| r.format_token_amount(chain_id, call.token, amount_u128))
                .map(|(s, _)| s)
                .unwrap_or_else(|| call.amount.to_string()),
            Err(_) => call.amount.to_string(),
        };

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
mod tests {
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
}
