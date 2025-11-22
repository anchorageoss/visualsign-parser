use alloy_sol_types::{SolCall as _, sol};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

// Aave Governance Token interface definitions
//
// Official Documentation:
// - Technical Reference: https://docs.aave.com/governance
// - Contract Source: https://github.com/bgd-labs/aave-governance-v3
sol! {
    interface IAaveToken {
        function delegate(address delegatee) external;
        function delegateByType(address delegatee, uint8 delegationType) external;
    }
}

pub struct AaveTokenVisualizer;

impl AaveTokenVisualizer {
    pub fn visualize_governance(
        &self,
        input: &[u8],
        _chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        // Try delegate
        if let Ok(call) = IAaveToken::delegateCall::abi_decode(input) {
            return Self::decode_delegate(&call);
        }

        // Try delegateByType
        if let Ok(call) = IAaveToken::delegateByTypeCall::abi_decode(input) {
            return Self::decode_delegate_by_type(&call);
        }

        None
    }

    fn decode_delegate(call: &IAaveToken::delegateCall) -> Option<SignablePayloadField> {
        let delegatee_str = format!("{:?}", call.delegatee);
        let summary = format!("Delegate all governance power to {}", delegatee_str);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: delegatee_str.clone(),
                        label: "Delegatee".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: delegatee_str,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Voting + Proposition".to_string(),
                        label: "Powers Delegated".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Voting + Proposition".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Governance".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave Governance Delegation".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    fn decode_delegate_by_type(
        call: &IAaveToken::delegateByTypeCall,
    ) -> Option<SignablePayloadField> {
        let delegatee_str = format!("{:?}", call.delegatee);
        let power_type = match call.delegationType {
            0 => "Voting Power",
            1 => "Proposition Power",
            _ => "Unknown Power",
        };

        let summary = format!("Delegate {} to {}", power_type, delegatee_str);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: delegatee_str.clone(),
                        label: "Delegatee".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: delegatee_str,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: power_type.to_string(),
                        label: "Power Type".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: power_type.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Governance".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave Governance Delegation".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn test_decode_delegate() {
        let call = IAaveToken::delegateCall {
            delegatee: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
        };

        let input = IAaveToken::delegateCall::abi_encode(&call);
        let result = AaveTokenVisualizer.visualize_governance(&input, 1, None);

        assert!(result.is_some(), "Should decode delegate successfully");

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Governance");
            assert!(
                common
                    .fallback_text
                    .to_lowercase()
                    .contains("0742d35cc6634c0532925a3b844bc9e7595f0beb")
            );
            assert!(preview_layout.subtitle.is_some());
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_delegate_by_type_voting() {
        let call = IAaveToken::delegateByTypeCall {
            delegatee: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            delegationType: 0, // Voting power
        };

        let input = IAaveToken::delegateByTypeCall::abi_encode(&call);
        let result = AaveTokenVisualizer.visualize_governance(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode delegateByType successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Governance");
            assert!(common.fallback_text.contains("Voting Power"));
            assert!(preview_layout.subtitle.is_some());
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_delegate_by_type_proposition() {
        let call = IAaveToken::delegateByTypeCall {
            delegatee: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            delegationType: 1, // Proposition power
        };

        let input = IAaveToken::delegateByTypeCall::abi_encode(&call);
        let result = AaveTokenVisualizer.visualize_governance(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode delegateByType successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("Proposition Power"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_invalid_input() {
        let result = AaveTokenVisualizer.visualize_governance(&[], 1, None);
        assert!(result.is_none(), "Should return None for empty input");

        let invalid = vec![0xff, 0xff, 0xff, 0xff];
        let result = AaveTokenVisualizer.visualize_governance(&invalid, 1, None);
        assert!(result.is_none(), "Should return None for invalid selector");
    }
}
