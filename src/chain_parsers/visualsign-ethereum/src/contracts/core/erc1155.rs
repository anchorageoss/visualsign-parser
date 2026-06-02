//! ERC-1155 Multi Token Standard Visualizer
//!
//! Provides visualization for the ERC-1155 transfer functions.
//!
//! Reference: <https://eips.ethereum.org/EIPS/eip-1155>

use alloy_sol_types::{SolCall, sol};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldCommon, SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout,
    SignablePayloadFieldTextV2,
};

// ERC-1155 transfer interface
sol! {
    interface IERC1155 {
        function safeTransferFrom(address from, address to, uint256 id, uint256 value, bytes data) external;
        function safeBatchTransferFrom(address from, address to, uint256[] ids, uint256[] values, bytes data) external;
    }
}

/// Visualizer for ERC-1155 multi-token contract calls
pub struct ERC1155Visualizer {}

impl ERC1155Visualizer {
    /// Attempts to decode and visualize ERC-1155 transfer calls.
    ///
    /// # Arguments
    /// * `input` - The calldata bytes (with 4-byte function selector)
    ///
    /// # Returns
    /// * `Some(field)` if a recognized ERC-1155 transfer is found
    /// * `None` if the input doesn't match any recognized function or fails to
    ///   decode (the caller falls back to raw hex, preserving the known-token
    ///   lock-out)
    pub fn visualize_tx_commands(&self, input: &[u8]) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        let selector = &input[..4];
        if selector == IERC1155::safeTransferFromCall::SELECTOR {
            // safeTransferFrom(address,address,uint256,uint256,bytes)
            if let Ok(call) = IERC1155::safeTransferFromCall::abi_decode(input) {
                return Some(Self::render_safe_transfer_from(call));
            }
        } else if selector == IERC1155::safeBatchTransferFromCall::SELECTOR {
            // safeBatchTransferFrom(address,address,uint256[],uint256[],bytes)
            if let Ok(call) = IERC1155::safeBatchTransferFromCall::abi_decode(input) {
                return Some(Self::render_safe_batch_transfer_from(call));
            }
        }

        None
    }

    /// Builds an AddressV2 detail row.
    fn address_row(label: &str, address: &alloy_primitives::Address) -> AnnotatedPayloadField {
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{address:?}"),
                    label: label.to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{address:?}"),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        }
    }

    /// Builds a TextV2 detail row.
    fn text_row(label: &str, text: String) -> AnnotatedPayloadField {
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: text.clone(),
                    label: label.to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 { text },
            },
            static_annotation: None,
            dynamic_annotation: None,
        }
    }

    /// Renders a single `safeTransferFrom` as a preview with From, To, Token ID
    /// and Amount detail rows.
    fn render_safe_transfer_from(call: IERC1155::safeTransferFromCall) -> SignablePayloadField {
        let id_str = call.id.to_string();
        let value_str = call.value.to_string();

        let details = vec![
            Self::address_row("From", &call.from),
            Self::address_row("To", &call.to),
            Self::text_row("Token ID", id_str.clone()),
            Self::text_row("Amount", value_str.clone()),
        ];

        let subtitle = format!(
            "Transfer {value_str} of token {id_str} from {:?} to {:?}",
            call.from, call.to
        );

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: subtitle.clone(),
                label: "ERC1155 Transfer".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC1155 Transfer".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: subtitle }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        }
    }

    /// Renders a `safeBatchTransferFrom` as a preview with From, To and one row
    /// per (id, value) pair.
    ///
    /// `ids` and `values` decode to independent vectors, so a malformed payload
    /// can carry mismatched lengths. We iterate over the longer of the two and
    /// surface a placeholder where one side is missing, rendering what's present
    /// rather than panicking.
    fn render_safe_batch_transfer_from(
        call: IERC1155::safeBatchTransferFromCall,
    ) -> SignablePayloadField {
        let mut details = vec![
            Self::address_row("From", &call.from),
            Self::address_row("To", &call.to),
        ];

        let pair_count = call.ids.len().max(call.values.len());
        for i in 0..pair_count {
            let id_str = call
                .ids
                .get(i)
                .map(|id| id.to_string())
                .unwrap_or_else(|| "(none)".to_string());
            let value_str = call
                .values
                .get(i)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(none)".to_string());
            details.push(Self::text_row(
                &format!("Token {i}"),
                format!("id {id_str} -> amount {value_str}"),
            ));
        }

        let subtitle = format!(
            "Batch transfer {pair_count} token(s) from {:?} to {:?}",
            call.from, call.to
        );

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: subtitle.clone(),
                label: "ERC1155 Batch Transfer".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC1155 Batch Transfer".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: subtitle }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, hex};

    #[test]
    fn test_visualize_empty_input() {
        let visualizer = ERC1155Visualizer {};
        assert_eq!(visualizer.visualize_tx_commands(&[]), None);
    }

    #[test]
    fn test_visualize_too_short() {
        let visualizer = ERC1155Visualizer {};
        assert_eq!(visualizer.visualize_tx_commands(&[0x01, 0x02]), None);
    }

    #[test]
    fn test_visualize_unknown_selector() {
        let visualizer = ERC1155Visualizer {};
        let input = hex!("deadbeef01020304");
        assert!(visualizer.visualize_tx_commands(&input).is_none());
    }

    #[test]
    fn test_decode_safe_transfer_from() {
        let call = IERC1155::safeTransferFromCall {
            from: [0x11u8; 20].into(),
            to: [0x22u8; 20].into(),
            id: U256::from(7u64),
            value: U256::from(12345u64),
            data: alloy_primitives::Bytes::default(),
        };
        let input = IERC1155::safeTransferFromCall::abi_encode(&call);

        let field = ERC1155Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");

        let json = serde_json::to_string(&field).expect("serializable");
        assert!(json.contains("ERC1155 Transfer"), "got: {json}");
        assert!(json.contains(&format!("{:?}", call.from)), "got: {json}");
        assert!(json.contains(&format!("{:?}", call.to)), "got: {json}");
        // Token id and amount.
        assert!(json.contains('7'), "expected token id in output: {json}");
        assert!(json.contains("12345"), "expected amount in output: {json}");
    }

    #[test]
    fn test_decode_safe_batch_transfer_from() {
        let call = IERC1155::safeBatchTransferFromCall {
            from: [0x33u8; 20].into(),
            to: [0x44u8; 20].into(),
            ids: vec![U256::from(1u64), U256::from(2u64)],
            values: vec![U256::from(100u64), U256::from(200u64)],
            data: alloy_primitives::Bytes::default(),
        };
        let input = IERC1155::safeBatchTransferFromCall::abi_encode(&call);

        let field = ERC1155Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");

        let json = serde_json::to_string(&field).expect("serializable");
        assert!(json.contains("ERC1155 Batch Transfer"), "got: {json}");
        assert!(json.contains(&format!("{:?}", call.from)), "got: {json}");
        assert!(json.contains(&format!("{:?}", call.to)), "got: {json}");
        // Both (id, value) pairs are rendered.
        assert!(json.contains("id 1 -> amount 100"), "got: {json}");
        assert!(json.contains("id 2 -> amount 200"), "got: {json}");
    }

    #[test]
    fn test_decode_safe_batch_transfer_from_length_mismatch_does_not_panic() {
        // ids and values decode independently; a mismatched payload must render
        // what's present without panicking.
        let call = IERC1155::safeBatchTransferFromCall {
            from: [0x55u8; 20].into(),
            to: [0x66u8; 20].into(),
            ids: vec![U256::from(9u64), U256::from(10u64)],
            values: vec![U256::from(500u64)],
            data: alloy_primitives::Bytes::default(),
        };
        let input = IERC1155::safeBatchTransferFromCall::abi_encode(&call);

        let field = ERC1155Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");

        let json = serde_json::to_string(&field).expect("serializable");
        assert!(json.contains("id 9 -> amount 500"), "got: {json}");
        // The missing value side renders the placeholder rather than panicking.
        assert!(json.contains("id 10 -> amount (none)"), "got: {json}");
    }
}
