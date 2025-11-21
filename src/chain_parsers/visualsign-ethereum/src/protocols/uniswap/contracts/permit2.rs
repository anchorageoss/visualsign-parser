//! Permit2 Contract Visualizer
//!
//! Permit2 is Uniswap's token approval system that allows signature-based approvals
//! and transfers, improving UX by batching operations.
//!
//! Reference: <https://github.com/Uniswap/permit2>

#![allow(unused_imports)]

use alloy_primitives::Address;
use alloy_sol_types::{SolCall, sol};
use chrono::{TimeZone, Utc};
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

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
    /// Handles inline 192-byte PermitSingle struct without ABI offsets
    pub(crate) fn decode_custom_permit_params(
        bytes: &[u8],
    ) -> Result<PermitSingle, Box<dyn std::error::Error>> {
        use alloy_primitives::Address;

        if bytes.len() < 192 {
            return Err("bytes too short for PermitSingle (need 192 bytes minimum)".into());
        }

        let permit_single_bytes = &bytes[0..192];

        // Extract token (address) from bytes 12-31 (left-padded in Slot 0)
        let token = Address::from_slice(&permit_single_bytes[12..32]);

        // Extract amount (uint160) from bytes 44-63 (left-padded in Slot 1)
        let amount_hex = hex::encode(&permit_single_bytes[44..64]);
        let amount = alloy_primitives::Uint::<160, 3>::from_str_radix(&amount_hex, 16)
            .map_err(|_| "Failed to parse amount")?;

        // Extract expiration (uint48) from bytes 90-95 (right-aligned in Slot 2)
        let expiration_hex = hex::encode(&permit_single_bytes[90..96]);
        let expiration = alloy_primitives::Uint::<48, 1>::from_str_radix(&expiration_hex, 16)
            .map_err(|_| "Failed to parse expiration")?;

        // Extract nonce (uint48) from bytes 96-101 (Slot 3)
        let nonce_hex = hex::encode(&permit_single_bytes[96..102]);
        let nonce = alloy_primitives::Uint::<48, 1>::from_str_radix(&nonce_hex, 16)
            .map_err(|_| "Failed to parse nonce")?;

        // Extract spender (address) from bytes 140-159 (left-padded in Slot 4)
        let spender = Address::from_slice(&permit_single_bytes[140..160]);

        // Extract sigDeadline (uint256) from bytes 160-191 (all of Slot 5)
        let sig_deadline_hex = hex::encode(&permit_single_bytes[160..192]);
        let sig_deadline = alloy_primitives::U256::from_str_radix(&sig_deadline_hex, 16)
            .map_err(|_| "Failed to parse sigDeadline")?;

        Ok(PermitSingle {
            details: PermitDetails {
                token,
                amount,
                expiration,
                nonce,
            },
            spender,
            sigDeadline: sig_deadline,
        })
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
        let expiration_str = if expiration_u64 == u64::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(expiration_u64 as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

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
            .unwrap_or_else(|| format!("{:?}", token));

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
        let expiration_str = if expiration_u64 == u64::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(expiration_u64 as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        let text = format!(
            "Permit {} to spend {} {} from {} (expires: {})",
            call.permitSingle.spender, amount_str, token_symbol, call.owner, expiration_str
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Permit2 Permit".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
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

    // TODO: Add tests for Permit2 functions once implemented
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
        let contract_registry = crate::registry::ContractRegistry::with_default_protocols();

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
