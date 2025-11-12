use alloy_sol_types::{SolCall, sol};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

sol! {
    interface IERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);

        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);
    }
}
pub struct ERC20Visualizer {}

impl ERC20Visualizer {
    pub fn visualize_tx_commands(&self, input: &[u8]) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }
        let selector = &input[..4];
        if selector == IERC20::transferCall::SELECTOR {
            // transfer(address,uint256)
            if let Ok(call) = IERC20::transferCall::abi_decode(input) {
                let mut details = Vec::new();
                // To Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.to),
                            label: "Recipient".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.to),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                // Amount
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: call.amount.to_string(),
                            label: "Amount".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: call.amount.to_string(),
                            abbreviation: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                return Some(SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Transfer {} tokens to {:?}", call.amount, call.to),
                        label: "ERC20 Transfer".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Transfer".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: format!("Transfer {} tokens to {:?}", call.amount, call.to),
                        }),
                        condensed: None,
                        expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                    },
                });
            }
        } else if selector == IERC20::transferFromCall::SELECTOR {
            // transferFrom(address,address,uint256)
            if let Ok(call) = IERC20::transferFromCall::abi_decode(input) {
                let mut details = Vec::new();

                // From Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.from),
                            label: "Sender".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.from),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                // To Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.to),
                            label: "Recipient".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.to),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                // Amount
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: call.amount.to_string(),
                            label: "Amount".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: call.amount.to_string(),
                            abbreviation: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(
                            "Transfer {} tokens from {:?} to {:?}",
                            call.amount, call.from, call.to
                        ),
                        label: "ERC20 TransferFrom".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 TransferFrom".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: format!(
                                "Transfer {} tokens from {:?} to {:?}",
                                call.amount, call.from, call.to
                            ),
                        }),
                        condensed: None,
                        expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::approveCall::SELECTOR {
            // approve(address,uint256)
            if let Ok(call) = IERC20::approveCall::abi_decode(input) {
                let mut details = Vec::new();

                // Spender Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.spender),
                            label: "Spender".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.spender),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                // Amount
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: call.amount.to_string(),
                            label: "Amount".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: call.amount.to_string(),
                            abbreviation: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(
                            "Approve {:?} to spend {} tokens",
                            call.spender, call.amount
                        ),
                        label: "ERC20 Approve".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Approve".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: format!(
                                "Approve {:?} to spend {} tokens",
                                call.spender, call.amount
                            ),
                        }),
                        condensed: None,
                        expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::balanceOfCall::SELECTOR {
            // balanceOf(address)
            if let Ok(call) = IERC20::balanceOfCall::abi_decode(input) {
                let mut details = Vec::new();

                // Account Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.account),
                            label: "Account".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.account),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Query balance of {:?}", call.account),
                        label: "ERC20 BalanceOf".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 BalanceOf".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: format!("Query balance of {:?}", call.account),
                        }),
                        condensed: None,
                        expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::allowanceCall::SELECTOR {
            // allowance(address,address)
            if let Ok(call) = IERC20::allowanceCall::abi_decode(input) {
                let mut details = Vec::new();
                // Owner Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.owner),
                            label: "Owner".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.owner),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });
                // Spender Address
                details.push(AnnotatedPayloadField {
                    signable_payload_field: SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: format!("{:?}", call.spender),
                            label: "Spender".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: format!("{:?}", call.spender),
                            name: "".to_string(),
                            memo: None,
                            asset_label: "".to_string(),
                            badge_text: None,
                        },
                    },
                    static_annotation: None,
                    dynamic_annotation: None,
                });

                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(
                            "Query allowance for {:?} by {:?}",
                            call.spender, call.owner
                        ),
                        label: "ERC20 Allowance".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Allowance".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: format!(
                                "Query allowance for {:?} by {:?}",
                                call.spender, call.owner
                            ),
                        }),
                        condensed: None,
                        expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::nameCall::SELECTOR {
            // name()
            if let Ok(_call) = IERC20::nameCall::abi_decode(input) {
                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Query token name".to_string(),
                        label: "ERC20 Name".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Name".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: "Query token name".to_string(),
                        }),
                        condensed: None,
                        expanded: None,
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::symbolCall::SELECTOR {
            // symbol()
            if let Ok(_call) = IERC20::symbolCall::abi_decode(input) {
                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Query token symbol".to_string(),
                        label: "ERC20 Symbol".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Symbol".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: "Query token symbol".to_string(),
                        }),
                        condensed: None,
                        expanded: None,
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::decimalsCall::SELECTOR {
            // decimals()
            if let Ok(_call) = IERC20::decimalsCall::abi_decode(input) {
                let preview = SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Query token decimals".to_string(),
                        label: "ERC20 Decimals".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 Decimals".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: "Query token decimals".to_string(),
                        }),
                        condensed: None,
                        expanded: None,
                    },
                };
                return Some(preview);
            }
        } else if selector == IERC20::totalSupplyCall::SELECTOR {
            // totalSupply()
            if let Ok(_call) = IERC20::totalSupplyCall::abi_decode(input) {
                return Some(SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Query total token supply".to_string(),
                        label: "ERC20 TotalSupply".to_string(),
                    },
                    preview_layout: SignablePayloadFieldPreviewLayout {
                        title: Some(SignablePayloadFieldTextV2 {
                            text: "ERC20 TotalSupply".to_string(),
                        }),
                        subtitle: Some(SignablePayloadFieldTextV2 {
                            text: "Query total token supply".to_string(),
                        }),
                        condensed: None,
                        expanded: None,
                    },
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, hex};

    #[test]
    fn test_decode_transfer() {
        let call = IERC20::transferCall {
            to: [0x11u8; 20].into(),
            amount: U256::from(12345u64),
        };
        let input = IERC20::transferCall::abi_encode(&call);

        let expected = {
            let mut details = Vec::new();
            details.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::AddressV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.to),
                        label: "Recipient".to_string(),
                    },
                    address_v2: SignablePayloadFieldAddressV2 {
                        address: format!("{:?}", call.to),
                        name: "".to_string(),
                        memo: None,
                        asset_label: "".to_string(),
                        badge_text: None,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
            details.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::AmountV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: call.amount.to_string(),
                        label: "Amount".to_string(),
                    },
                    amount_v2: SignablePayloadFieldAmountV2 {
                        amount: call.amount.to_string(),
                        abbreviation: None,
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Transfer {} tokens to {:?}", call.amount, call.to),
                    label: "ERC20 Transfer".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "ERC20 Transfer".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: format!("Transfer {} tokens to {:?}", call.amount, call.to),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout { fields: details }),
                },
            }
        };

        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_transfer_from() {
        let call = IERC20::transferFromCall {
            from: [0x22u8; 20].into(),
            to: [0x33u8; 20].into(),
            amount: U256::from(555u64),
        };
        let input = IERC20::transferFromCall::abi_encode(&call);

        let mut details = Vec::new();
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.from),
                    label: "Sender".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.from),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.to),
                    label: "Recipient".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.to),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: call.amount.to_string(),
                    label: "Amount".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: call.amount.to_string(),
                    abbreviation: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "Transfer {} tokens from {:?} to {:?}",
                    call.amount, call.from, call.to
                ),
                label: "ERC20 TransferFrom".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 TransferFrom".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: format!(
                        "Transfer {} tokens from {:?} to {:?}",
                        call.amount, call.from, call.to
                    ),
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        };

        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_approve() {
        let call = IERC20::approveCall {
            spender: [0x44u8; 20].into(),
            amount: U256::from(999u64),
        };
        let input = IERC20::approveCall::abi_encode(&call);

        let mut details = Vec::new();
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.spender),
                    label: "Spender".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.spender),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: call.amount.to_string(),
                    label: "Amount".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: call.amount.to_string(),
                    abbreviation: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "Approve {:?} to spend {} tokens",
                    call.spender, call.amount
                ),
                label: "ERC20 Approve".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 Approve".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: format!("Approve {:?} to spend {} tokens", call.spender, call.amount),
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        };

        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_balance_of() {
        let call = IERC20::balanceOfCall {
            account: [0x55u8; 20].into(),
        };
        let input = IERC20::balanceOfCall::abi_encode(&call);

        let mut details = Vec::new();
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.account),
                    label: "Account".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.account),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("Query balance of {:?}", call.account),
                label: "ERC20 BalanceOf".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 BalanceOf".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: format!("Query balance of {:?}", call.account),
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        };

        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_allowance() {
        let call = IERC20::allowanceCall {
            owner: [0x66u8; 20].into(),
            spender: [0x77u8; 20].into(),
        };
        let input = IERC20::allowanceCall::abi_encode(&call);

        let mut details = Vec::new();
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.owner),
                    label: "Owner".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.owner),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
        details.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.spender),
                    label: "Spender".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: format!("{:?}", call.spender),
                    name: "".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "Query allowance for {:?} by {:?}",
                    call.spender, call.owner
                ),
                label: "ERC20 Allowance".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 Allowance".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: format!("Query allowance for {:?} by {:?}", call.spender, call.owner),
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields: details }),
            },
        };

        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_name() {
        let input = IERC20::nameCall::abi_encode(&IERC20::nameCall {});
        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: "Query token name".to_string(),
                label: "ERC20 Name".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 Name".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: "Query token name".to_string(),
                }),
                condensed: None,
                expanded: None,
            },
        };
        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_symbol() {
        let input = IERC20::symbolCall::abi_encode(&IERC20::symbolCall {});
        let expected = SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: "Query token symbol".to_string(),
                label: "ERC20 Symbol".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "ERC20 Symbol".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: "Query token symbol".to_string(),
                }),
                condensed: None,
                expanded: None,
            },
        };
        let actual = ERC20Visualizer {}
            .visualize_tx_commands(&input)
            .expect("Expected PreviewLayout");
        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_decode_decimals() {
        let input = IERC20::decimalsCall::abi_encode(&IERC20::decimalsCall {});
        assert_eq!(
            ERC20Visualizer {}
                .visualize_tx_commands(&input)
                .expect("Expected PreviewLayout"),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Query token decimals".to_string(),
                    label: "ERC20 Decimals".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "ERC20 Decimals".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "Query token decimals".to_string(),
                    }),
                    condensed: None,
                    expanded: None,
                },
            }
        );
    }

    #[test]
    fn test_decode_total_supply() {
        let input = IERC20::totalSupplyCall::abi_encode(&IERC20::totalSupplyCall {});
        assert_eq!(
            ERC20Visualizer {}
                .visualize_tx_commands(&input)
                .expect("Expected PreviewLayout"),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Query total token supply".to_string(),
                    label: "ERC20 TotalSupply".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "ERC20 TotalSupply".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "Query total token supply".to_string(),
                    }),
                    condensed: None,
                    expanded: None,
                },
            }
        );
    }

    #[test]
    fn test_decode_invalid_selector() {
        let input = hex!("deadbeef01020304");
        let actual = ERC20Visualizer {}.visualize_tx_commands(&input);
        assert!(actual.is_none());
    }

    #[test]
    fn test_decode_too_short_input() {
        let input = &[0x01, 0x02, 0x03];
        let actual = ERC20Visualizer {}.visualize_tx_commands(input);
        assert!(actual.is_none());
    }
}
