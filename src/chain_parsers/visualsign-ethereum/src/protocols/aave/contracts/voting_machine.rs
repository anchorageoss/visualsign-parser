use alloy_sol_types::{SolCall as _, sol};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

// Aave Governance VotingMachine interface definitions
//
// Official Documentation:
// - Technical Reference: https://docs.aave.com/governance/master/aave-governance-v3
// - Contract Source: https://github.com/bgd-labs/aave-governance-v3
sol! {
    interface IVotingMachine {
        function submitVote(uint256 proposalId, bool support) external;
        function submitVoteAsRepresentative(
            uint256 proposalId,
            bool support,
            address[] calldata votingTokens
        ) external;
    }
}

pub struct VotingMachineVisualizer;

impl VotingMachineVisualizer {
    pub fn visualize_vote(
        &self,
        input: &[u8],
        _chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        // Try submitVote
        if let Ok(call) = IVotingMachine::submitVoteCall::abi_decode(input) {
            return Self::decode_submit_vote(&call);
        }

        // Try submitVoteAsRepresentative
        if let Ok(call) = IVotingMachine::submitVoteAsRepresentativeCall::abi_decode(input) {
            return Self::decode_submit_vote_as_representative(&call);
        }

        None
    }

    fn decode_submit_vote(call: &IVotingMachine::submitVoteCall) -> Option<SignablePayloadField> {
        let vote_direction = if call.support { "For" } else { "Against" };
        let proposal_id = call.proposalId.to_string();
        let summary = format!("Vote {} on proposal #{}", vote_direction, proposal_id);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: proposal_id.clone(),
                        label: "Proposal ID".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: proposal_id },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: vote_direction.to_string(),
                        label: "Vote".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: vote_direction.to_string(),
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
                    text: "Aave Governance Vote".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    fn decode_submit_vote_as_representative(
        call: &IVotingMachine::submitVoteAsRepresentativeCall,
    ) -> Option<SignablePayloadField> {
        let vote_direction = if call.support { "For" } else { "Against" };
        let proposal_id = call.proposalId.to_string();
        let num_tokens = call.votingTokens.len();

        let summary = format!(
            "Vote {} on proposal #{} (as representative with {} token{})",
            vote_direction,
            proposal_id,
            num_tokens,
            if num_tokens == 1 { "" } else { "s" }
        );

        let mut fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: proposal_id.clone(),
                        label: "Proposal ID".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: proposal_id },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: vote_direction.to_string(),
                        label: "Vote".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: vote_direction.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: num_tokens.to_string(),
                        label: "Voting Tokens".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: num_tokens.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        for (i, token) in call.votingTokens.iter().enumerate() {
            let token_str = format!("{:?}", token);
            fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_str.clone(),
                        label: format!("Token {}", i + 1),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: token_str },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Governance".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave Governance Vote".to_string(),
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
    use alloy_primitives::{U256, address};

    #[test]
    fn test_decode_submit_vote_for() {
        let call = IVotingMachine::submitVoteCall {
            proposalId: U256::from(123),
            support: true,
        };

        let input = IVotingMachine::submitVoteCall::abi_encode(&call);
        let result = VotingMachineVisualizer.visualize_vote(&input, 1, None);

        assert!(result.is_some(), "Should decode submitVote successfully");

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Governance");
            assert!(common.fallback_text.contains("For"));
            assert!(common.fallback_text.contains("123"));
            assert!(preview_layout.subtitle.is_some());
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_submit_vote_against() {
        let call = IVotingMachine::submitVoteCall {
            proposalId: U256::from(456),
            support: false,
        };

        let input = IVotingMachine::submitVoteCall::abi_encode(&call);
        let result = VotingMachineVisualizer.visualize_vote(&input, 137, None);

        assert!(result.is_some(), "Should decode submitVote successfully");

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("Against"));
            assert!(common.fallback_text.contains("456"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_submit_vote_as_representative() {
        let call = IVotingMachine::submitVoteAsRepresentativeCall {
            proposalId: U256::from(789),
            support: true,
            votingTokens: vec![
                address!("7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9"),
                address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            ],
        };

        let input = IVotingMachine::submitVoteAsRepresentativeCall::abi_encode(&call);
        let result = VotingMachineVisualizer.visualize_vote(&input, 43114, None);

        assert!(
            result.is_some(),
            "Should decode submitVoteAsRepresentative successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Governance");
            assert!(common.fallback_text.contains("For"));
            assert!(common.fallback_text.contains("789"));
            assert!(common.fallback_text.contains("2 tokens"));
            assert!(preview_layout.subtitle.is_some());
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_submit_vote_as_representative_single_token() {
        let call = IVotingMachine::submitVoteAsRepresentativeCall {
            proposalId: U256::from(100),
            support: false,
            votingTokens: vec![address!("7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9")],
        };

        let input = IVotingMachine::submitVoteAsRepresentativeCall::abi_encode(&call);
        let result = VotingMachineVisualizer.visualize_vote(&input, 1, None);

        assert!(result.is_some(), "Should decode successfully");

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("1 token"));
            assert!(!common.fallback_text.contains("tokens"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_invalid_input() {
        let result = VotingMachineVisualizer.visualize_vote(&[], 1, None);
        assert!(result.is_none(), "Should return None for empty input");

        let invalid = vec![0xff, 0xff, 0xff, 0xff];
        let result = VotingMachineVisualizer.visualize_vote(&invalid, 1, None);
        assert!(result.is_none(), "Should return None for invalid selector");
    }
}
