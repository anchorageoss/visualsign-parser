use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolCall as _, sol};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use super::l2_pool::{IL2Pool, L2PoolVisualizer};
use crate::context::VisualizerContext;
use crate::protocols::aave::config::AaveV3PoolContract;
use crate::registry::{ContractRegistry, ContractType};

// Aave v3 Pool interface definitions
//
// Official Documentation:
// - Technical Reference: https://docs.aave.com/developers/core-contracts/pool
// - Contract Source: https://github.com/aave-dao/aave-v3-origin
//
// The Pool contract is the main entry point for Aave v3 interactions.
// It supports lending, borrowing, repayment, and liquidations.
sol! {
    interface IPool {
        function supply(
            address asset,
            uint256 amount,
            address onBehalfOf,
            uint16 referralCode
        ) external;

        function withdraw(
            address asset,
            uint256 amount,
            address to
        ) external returns (uint256);

        function borrow(
            address asset,
            uint256 amount,
            uint256 interestRateMode,
            uint16 referralCode,
            address onBehalfOf
        ) external;

        function repay(
            address asset,
            uint256 amount,
            uint256 interestRateMode,
            address onBehalfOf
        ) external returns (uint256);

        function liquidationCall(
            address collateralAsset,
            address debtAsset,
            address user,
            uint256 debtToCover,
            bool receiveAToken
        ) external;

        function supplyWithPermit(
            address asset,
            uint256 amount,
            address onBehalfOf,
            uint16 referralCode,
            uint256 deadline,
            uint8 permitV,
            bytes32 permitR,
            bytes32 permitS
        ) external;

        function repayWithPermit(
            address asset,
            uint256 amount,
            uint256 interestRateMode,
            address onBehalfOf,
            uint256 deadline,
            uint8 permitV,
            bytes32 permitR,
            bytes32 permitS
        ) external returns (uint256);

        function repayWithATokens(
            address asset,
            uint256 amount,
            uint256 interestRateMode
        ) external returns (uint256);

        function setUserUseReserveAsCollateral(
            address asset,
            bool useAsCollateral
        ) external;

        function flashLoan(
            address receiverAddress,
            address[] calldata assets,
            uint256[] calldata amounts,
            uint256[] calldata interestRateModes,
            address onBehalfOf,
            bytes calldata params,
            uint16 referralCode
        ) external;

        function flashLoanSimple(
            address receiverAddress,
            address asset,
            uint256 amount,
            bytes calldata params,
            uint16 referralCode
        ) external;
    }
}

/// Visualizer for Aave v3 Pool contract
pub struct PoolVisualizer {
    l2_pool_visualizer: L2PoolVisualizer,
}

impl PoolVisualizer {
    pub fn new() -> Self {
        Self {
            l2_pool_visualizer: L2PoolVisualizer::new(),
        }
    }

    /// Visualizes Aave v3 Pool operations
    ///
    /// # Arguments
    /// * `input` - The calldata bytes
    /// * `chain_id` - The chain ID for registry lookups
    /// * `registry` - Optional registry for resolving token symbols
    pub fn visualize_pool_operation(
        &self,
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        let selector = &input[0..4];

        // Route to specific decoders based on function selector
        match selector {
            // Standard Pool functions (Ethereum mainnet, Base, etc.)

            // supply(address,uint256,address,uint16)
            _ if selector == IPool::supplyCall::SELECTOR => {
                Self::decode_supply(input, chain_id, registry)
            }
            // withdraw(address,uint256,address)
            _ if selector == IPool::withdrawCall::SELECTOR => {
                Self::decode_withdraw(input, chain_id, registry)
            }
            // borrow(address,uint256,uint256,uint16,address)
            _ if selector == IPool::borrowCall::SELECTOR => {
                Self::decode_borrow(input, chain_id, registry)
            }
            // repay(address,uint256,uint256,address)
            _ if selector == IPool::repayCall::SELECTOR => {
                Self::decode_repay(input, chain_id, registry)
            }
            // liquidationCall(address,address,address,uint256,bool)
            _ if selector == IPool::liquidationCallCall::SELECTOR => {
                Self::decode_liquidation_call(input, chain_id, registry)
            }
            // supplyWithPermit(address,uint256,address,uint16,uint256,uint8,bytes32,bytes32)
            _ if selector == IPool::supplyWithPermitCall::SELECTOR => {
                Self::decode_supply_with_permit(input, chain_id, registry)
            }
            // repayWithPermit(address,uint256,uint256,address,uint256,uint8,bytes32,bytes32)
            _ if selector == IPool::repayWithPermitCall::SELECTOR => {
                Self::decode_repay_with_permit(input, chain_id, registry)
            }
            // repayWithATokens(address,uint256,uint256)
            _ if selector == IPool::repayWithATokensCall::SELECTOR => {
                Self::decode_repay_with_atokens(input, chain_id, registry)
            }
            // setUserUseReserveAsCollateral(address,bool)
            _ if selector == IPool::setUserUseReserveAsCollateralCall::SELECTOR => {
                Self::decode_set_user_use_reserve_as_collateral(input, chain_id, registry)
            }
            // flashLoan(address,address[],uint256[],uint256[],address,bytes,uint16)
            _ if selector == IPool::flashLoanCall::SELECTOR => {
                Self::decode_flash_loan(input, chain_id, registry)
            }
            // flashLoanSimple(address,address,uint256,bytes,uint16)
            _ if selector == IPool::flashLoanSimpleCall::SELECTOR => {
                Self::decode_flash_loan_simple(input, chain_id, registry)
            }

            // L2Pool functions - delegate to L2PoolVisualizer

            // supply(bytes32)
            _ if selector == IL2Pool::supplyCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // withdraw(bytes32)
            _ if selector == IL2Pool::withdrawCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // borrow(bytes32)
            _ if selector == IL2Pool::borrowCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // repay(bytes32)
            _ if selector == IL2Pool::repayCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // repayWithATokens(bytes32)
            _ if selector == IL2Pool::repayWithATokensCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // liquidationCall(bytes32, bytes32)
            _ if selector == IL2Pool::liquidationCallCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // setUserUseReserveAsCollateral(bytes32)
            _ if selector == IL2Pool::setUserUseReserveAsCollateralCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // supplyWithPermit(bytes32,bytes32,bytes32)
            _ if selector == IL2Pool::supplyWithPermitCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),
            // repayWithPermit(bytes32,bytes32,bytes32)
            _ if selector == IL2Pool::repayWithPermitCall::SELECTOR => self
                .l2_pool_visualizer
                .visualize_l2pool_operation(input, chain_id, registry),

            _ => None,
        }
    }

    /// Decodes supply operation
    fn decode_supply(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::supplyCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        let behalf_text = if call.onBehalfOf != Address::ZERO {
            format!(" on behalf of {:?}", call.onBehalfOf)
        } else {
            String::new()
        };

        let summary = format!("Supply {} {}{}", amount_str, token_symbol, behalf_text);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, call.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.onBehalfOf),
                        label: "On Behalf Of".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.onBehalfOf),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Supply".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Supply".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes withdraw operation
    fn decode_withdraw(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::withdrawCall::abi_decode(input).ok()?;

        // Resolve token symbol
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        // Check if withdrawing max amount
        let is_max = call.amount == U256::MAX;
        let amount_display = if is_max {
            "Maximum".to_string()
        } else {
            let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
                .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));
            amount_str
        };

        let summary = format!(
            "Withdraw {} {} to {:?}",
            amount_display, token_symbol, call.to
        );

        // Create detailed parameter fields
        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: if is_max {
                            "Maximum (type(uint256).max)".to_string()
                        } else {
                            format!("{} {}", amount_display, token_symbol)
                        },
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Maximum ({})", call.amount)
                        } else {
                            format!("{} {} (raw: {})", amount_display, token_symbol, call.amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.to),
                        label: "To".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.to),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Withdraw".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Withdraw".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes borrow operation
    fn decode_borrow(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::borrowCall::abi_decode(input).ok()?;

        // Resolve token symbol
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        // Format amount
        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        // Interest rate mode (2 = Variable, 1 = Deprecated Stable)
        let rate_mode_str = call.interestRateMode.to_string();
        let rate_mode = match rate_mode_str.as_str() {
            "1" => "Stable (Deprecated)",
            "2" => "Variable",
            _ => &rate_mode_str,
        };

        let behalf_text = if call.onBehalfOf != Address::ZERO {
            format!(" on behalf of {:?}", call.onBehalfOf)
        } else {
            String::new()
        };

        let summary = format!(
            "Borrow {} {} at {} rate{}",
            amount_str, token_symbol, rate_mode, behalf_text
        );

        // Create detailed parameter fields
        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, call.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, call.interestRateMode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.onBehalfOf),
                        label: "On Behalf Of".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.onBehalfOf),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Borrow".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Borrow".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes repay operation
    fn decode_repay(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::repayCall::abi_decode(input).ok()?;

        // Resolve token symbol
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        // Check if repaying max amount
        let is_max = call.amount == U256::MAX;
        let amount_display = if is_max {
            "Full debt".to_string()
        } else {
            let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
                .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));
            format!("{} {}", amount_str, token_symbol)
        };

        // Interest rate mode
        let rate_mode_str = call.interestRateMode.to_string();
        let rate_mode = match rate_mode_str.as_str() {
            "1" => "Stable (Deprecated)",
            "2" => "Variable",
            _ => &rate_mode_str,
        };

        let behalf_text = if call.onBehalfOf != Address::ZERO {
            format!(" on behalf of {:?}", call.onBehalfOf)
        } else {
            String::new()
        };

        let summary = format!(
            "Repay {} at {} rate{}",
            amount_display, rate_mode, behalf_text
        );

        // Create detailed parameter fields
        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_display.clone(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full debt ({})", call.amount)
                        } else {
                            format!("{} (raw: {})", amount_display, call.amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, call.interestRateMode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.onBehalfOf),
                        label: "On Behalf Of".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.onBehalfOf),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Repay".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Repay".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes liquidation call operation
    fn decode_liquidation_call(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::liquidationCallCall::abi_decode(input).ok()?;

        // Resolve token symbols
        let collateral_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.collateralAsset))
            .unwrap_or_else(|| format!("{:?}", call.collateralAsset));
        let debt_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.debtAsset))
            .unwrap_or_else(|| format!("{:?}", call.debtAsset));

        // Format debt to cover
        let debt_u128: u128 = call.debtToCover.to_string().parse().unwrap_or(0);
        let (debt_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.debtAsset, debt_u128))
            .unwrap_or_else(|| (call.debtToCover.to_string(), debt_symbol.clone()));

        let receive_type = if call.receiveAToken {
            format!("a{}", collateral_symbol)
        } else {
            collateral_symbol.clone()
        };

        let summary = format!(
            "Liquidate {:?}: Cover {} {} debt, receive {}",
            call.user, debt_str, debt_symbol, receive_type
        );

        // Create detailed parameter fields
        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.user),
                        label: "User Being Liquidated".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.user),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", debt_str, debt_symbol),
                        label: "Debt to Cover".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(
                            "{} {} ({:?}, raw: {})",
                            debt_str, debt_symbol, call.debtAsset, call.debtToCover
                        ),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: collateral_symbol.clone(),
                        label: "Collateral Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", collateral_symbol, call.collateralAsset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: receive_type.clone(),
                        label: "Receive As".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if call.receiveAToken {
                            format!("{} (aToken)", receive_type)
                        } else {
                            format!("{} (underlying)", receive_type)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Liquidation".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Liquidation Call".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes supplyWithPermit operation
    fn decode_supply_with_permit(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::supplyWithPermitCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        let behalf_text = if call.onBehalfOf != Address::ZERO {
            format!(" on behalf of {:?}", call.onBehalfOf)
        } else {
            String::new()
        };

        let summary = format!(
            "Supply {} {} with permit{}",
            amount_str, token_symbol, behalf_text
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, call.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.onBehalfOf),
                        label: "On Behalf Of".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.onBehalfOf),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "ERC-2612 Permit".to_string(),
                        label: "Authorization".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Using gasless ERC-2612 permit signature".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Supply with Permit".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Supply with Permit".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes repayWithPermit operation
    fn decode_repay_with_permit(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::repayWithPermitCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let is_max = call.amount == U256::MAX;
        let amount_display = if is_max {
            "Full debt".to_string()
        } else {
            let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
                .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));
            format!("{} {}", amount_str, token_symbol)
        };

        let rate_mode_str = call.interestRateMode.to_string();
        let rate_mode = match rate_mode_str.as_str() {
            "1" => "Stable (Deprecated)",
            "2" => "Variable",
            _ => &rate_mode_str,
        };

        let behalf_text = if call.onBehalfOf != Address::ZERO {
            format!(" on behalf of {:?}", call.onBehalfOf)
        } else {
            String::new()
        };

        let summary = format!(
            "Repay {} at {} rate with permit{}",
            amount_display, rate_mode, behalf_text
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_display.clone(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full debt ({})", call.amount)
                        } else {
                            format!("{} (raw: {})", amount_display, call.amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, call.interestRateMode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.onBehalfOf),
                        label: "On Behalf Of".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.onBehalfOf),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "ERC-2612 Permit".to_string(),
                        label: "Authorization".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Using gasless ERC-2612 permit signature".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Repay with Permit".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Repay with Permit".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes repayWithATokens operation
    fn decode_repay_with_atokens(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::repayWithATokensCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let is_max = call.amount == U256::MAX;
        let amount_display = if is_max {
            "Full debt".to_string()
        } else {
            let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
                .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));
            format!("{} {}", amount_str, token_symbol)
        };

        let rate_mode_str = call.interestRateMode.to_string();
        let rate_mode = match rate_mode_str.as_str() {
            "1" => "Stable (Deprecated)",
            "2" => "Variable",
            _ => &rate_mode_str,
        };

        let summary = format!(
            "Repay {} using aTokens at {} rate",
            amount_display, rate_mode
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_display.clone(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full debt ({})", call.amount)
                        } else {
                            format!("{} (raw: {})", amount_display, call.amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, call.interestRateMode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Using aTokens".to_string(),
                        label: "Repayment Method".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Repaying with aTokens (no transfer required)".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Repay with aTokens".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Repay with aTokens".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes setUserUseReserveAsCollateral operation
    fn decode_set_user_use_reserve_as_collateral(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::setUserUseReserveAsCollateralCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let action = if call.useAsCollateral {
            "Enable"
        } else {
            "Disable"
        };

        let summary = format!("{} {} as collateral", action, token_symbol);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: action.to_string(),
                        label: "Action".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if call.useAsCollateral {
                            "Enable as collateral (allows borrowing)"
                        } else {
                            "Disable as collateral (reduces borrowing power)"
                        }
                        .to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Collateral Setting".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Set Collateral".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    fn decode_flash_loan(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::flashLoanCall::abi_decode(input).ok()?;

        let assets_count = call.assets.len();
        let assets_text = if assets_count == 1 {
            let asset = call.assets[0];
            let token_symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, asset))
                .unwrap_or_else(|| format!("{:?}", asset));

            let amount_u128: u128 = call.amounts[0].to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, asset, amount_u128))
                .unwrap_or_else(|| (call.amounts[0].to_string(), token_symbol.clone()));

            format!("{} {}", amount_str, token_symbol)
        } else {
            format!("{} assets", assets_count)
        };

        let summary = format!("Flash loan {}", assets_text);

        let mut fields = vec![];

        for (i, (asset, amount)) in call.assets.iter().zip(call.amounts.iter()).enumerate() {
            let token_symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, *asset))
                .unwrap_or_else(|| format!("{:?}", asset));

            let amount_u128: u128 = amount.to_string().parse().unwrap_or(0);
            let (amount_str, _) = registry
                .and_then(|r| r.format_token_amount(chain_id, *asset, amount_u128))
                .unwrap_or_else(|| (amount.to_string(), token_symbol.clone()));

            fields.push(AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: format!("Asset {}", i + 1),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} ({:?})", amount_str, token_symbol, asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }

        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.receiverAddress),
                    label: "Receiver".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{:?}", call.receiverAddress),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{:?}", call.onBehalfOf),
                    label: "On Behalf Of".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{:?}", call.onBehalfOf),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Flash Loan".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Flash Loan".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    fn decode_flash_loan_simple(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IPool::flashLoanSimpleCall::abi_decode(input).ok()?;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, call.asset))
            .unwrap_or_else(|| format!("{:?}", call.asset));

        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, call.asset, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        let summary = format!("Flash loan {} {}", amount_str, token_symbol);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} ({:?})", token_symbol, call.asset),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, call.asset),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, call.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.receiverAddress),
                        label: "Receiver".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.receiverAddress),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave Flash Loan".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 Simple Flash Loan".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }
}

/// ContractVisualizer implementation for Aave v3 Pool
pub struct PoolContractVisualizer {
    inner: PoolVisualizer,
}

impl PoolContractVisualizer {
    pub fn new() -> Self {
        Self {
            inner: PoolVisualizer::new(),
        }
    }
}

impl Default for PoolContractVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::visualizer::ContractVisualizer for PoolContractVisualizer {
    fn contract_type(&self) -> &str {
        AaveV3PoolContract::short_type_id()
    }

    fn visualize(
        &self,
        context: &VisualizerContext,
    ) -> Result<Option<Vec<AnnotatedPayloadField>>, visualsign::vsptrait::VisualSignError> {
        let contract_registry = ContractRegistry::with_default_protocols();

        if let Some(field) = self.inner.visualize_pool_operation(
            &context.calldata,
            context.chain_id,
            Some(&contract_registry),
        ) {
            let annotated = AnnotatedPayloadField {
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
    use alloy_primitives::{FixedBytes, address};

    #[test]
    fn test_decode_supply() {
        let call = IPool::supplyCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            amount: U256::from(1000000000u64),                           // 1000 USDC (6 decimals)
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            referralCode: 0,
        };

        let input = IPool::supplyCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some(), "Should decode supply successfully");

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Supply");
            assert!(common.fallback_text.contains("Supply"));

            assert!(preview_layout.expanded.is_some());
            if let Some(expanded) = preview_layout.expanded {
                assert_eq!(expanded.fields.len(), 3, "Should have 3 parameter fields");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_borrow() {
        let call = IPool::borrowCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::from(500000000u64),
            interestRateMode: U256::from(2), // Variable
            referralCode: 0,
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
        };

        let input = IPool::borrowCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some(), "Should decode borrow successfully");

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave Borrow");
            assert!(common.fallback_text.contains("Borrow"));
            assert!(common.fallback_text.contains("Variable"));
        }
    }

    #[test]
    fn test_decode_withdraw_max() {
        let call = IPool::withdrawCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::MAX, // Withdraw all
            to: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
        };

        let input = IPool::withdrawCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some(), "Should decode withdraw successfully");

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(
                common.fallback_text.contains("Maximum"),
                "Should indicate max withdrawal"
            );
        }
    }

    #[test]
    fn test_decode_repay_full_debt() {
        let call = IPool::repayCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::MAX, // Repay full debt
            interestRateMode: U256::from(2),
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
        };

        let input = IPool::repayCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some(), "Should decode repay successfully");

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(
                common.fallback_text.contains("Full debt"),
                "Should indicate full debt repayment"
            );
        }
    }

    #[test]
    fn test_real_aave_supply_110k_usdt() {
        // Real transaction: 0x394da4860478e24eaf99007a617f2009ed6a4c2f3a9ac43cf4da1e8ad1db2400
        // Just the calldata!
        let input_hex = "617ba037000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7000000000000000000000000000000000000000000000000000000199c82cc00000000000000000000000000b6559478b59836376da9937c4c697ddb21779e490000000000000000000000000000000000000000000000000000000000000000";
        let input = hex::decode(input_hex).unwrap();

        let registry = ContractRegistry::with_default_protocols();
        let result = PoolVisualizer::new()
            .visualize_pool_operation(&input, 1, Some(&registry))
            .expect("Should decode successfully");

        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = result
        {
            println!("\n=== Real Aave Supply Transaction ===");
            println!("Label: {}", common.label);
            println!("Summary: {}", common.fallback_text);

            if let Some(title) = &preview_layout.title {
                println!("Title: {}", title.text);
            }
            if let Some(subtitle) = &preview_layout.subtitle {
                println!("Subtitle: {}", subtitle.text);
            }

            if let Some(expanded) = &preview_layout.expanded {
                println!("\nDetailed Parameters:");
                for field in &expanded.fields {
                    match &field.signable_payload_field {
                        SignablePayloadField::TextV2 { common, text_v2 } => {
                            println!("  {}: {}", common.label, text_v2.text);
                        }
                        _ => {}
                    }
                }
            }
            println!("=== End ===\n");

            // Assertions
            assert_eq!(common.label, "Aave Supply");
            assert!(common.fallback_text.contains("USDT"));
            assert!(common.fallback_text.contains("110000"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_l2_liquidation_call() {
        let mut args1_bytes = [0u8; 32];
        let mut args2_bytes = [0u8; 32];

        let collateral_asset_id: u16 = 12;
        let debt_asset_id: u16 = 0;
        let user_addr = address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb");
        let debt_to_cover: u128 = 1000000;
        let receive_atoken = true;

        args1_bytes[30..32].copy_from_slice(&collateral_asset_id.to_be_bytes());
        args1_bytes[28..30].copy_from_slice(&debt_asset_id.to_be_bytes());
        args1_bytes[8..28].copy_from_slice(user_addr.as_slice());

        args2_bytes[16..32].copy_from_slice(&debt_to_cover.to_be_bytes());
        if receive_atoken {
            args2_bytes[15] = 1;
        }

        let call = IL2Pool::liquidationCallCall {
            args1: FixedBytes::from(args1_bytes),
            args2: FixedBytes::from(args2_bytes),
        };

        let input = IL2Pool::liquidationCallCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 42161, None);

        assert!(
            result.is_some(),
            "Should decode L2 liquidation call successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave L2 Liquidation");
            assert!(common.fallback_text.contains("Liquidate"));
            assert!(common.fallback_text.contains("debt"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_real_arbitrum_liquidation_standard_pool() {
        // Real Arbitrum liquidation transaction
        // Tx uses STANDARD Pool liquidationCall(address,address,address,uint256,bool)
        // NOT the L2Pool liquidationCall(bytes32,bytes32)
        //
        // Details:
        // - collateralAsset: 0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9 (USDT on Arbitrum)
        // - debtAsset: 0xfd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9 (USDT)
        // - borrower: 0x6cd6f60cf17566f145713b0f909fc5ce6ef5eb75
        // - debtToCover: 8165790531 (8.165 USDT, 6 decimals)
        // - receiveAToken: false

        let input_hex = "00a718a9000000000000000000000000fd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb9000000000000000000000000fd086bc7cd5c481dcc9c85ebe478a1c0b69fcbb90000000000000000000000006cd6f60cf17566f145713b0f909fc5ce6ef5eb7500000000000000000000000000000000000000000000000000000001e6b813430000000000000000000000000000000000000000000000000000000000000000";
        let input = hex::decode(input_hex).unwrap();

        let result = PoolVisualizer::new().visualize_pool_operation(&input, 42161, None);

        assert!(
            result.is_some(),
            "Should decode real liquidation transaction"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave Liquidation");
            assert!(common.fallback_text.contains("Liquidate"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_l2_set_user_use_reserve_as_collateral() {
        let mut args_bytes = [0u8; 32];

        let asset_id: u16 = 12;
        let use_as_collateral = true;

        args_bytes[30..32].copy_from_slice(&asset_id.to_be_bytes());
        if use_as_collateral {
            args_bytes[29] = 1;
        }

        let call = IL2Pool::setUserUseReserveAsCollateralCall {
            args: FixedBytes::from(args_bytes),
        };

        let input = IL2Pool::setUserUseReserveAsCollateralCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 42161, None);

        assert!(
            result.is_some(),
            "Should decode L2 set collateral successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave L2 Collateral Setting");
            assert!(common.fallback_text.contains("Enable"));
            assert!(common.fallback_text.contains("collateral"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_repay_with_atokens() {
        let call = IPool::repayWithATokensCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            amount: U256::from(500000000u64),                            // 500 USDC
            interestRateMode: U256::from(2),                             // Variable
        };

        let input = IPool::repayWithATokensCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode repayWithATokens successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave Repay with aTokens");
            assert!(common.fallback_text.contains("Repay"));
            assert!(common.fallback_text.contains("aTokens"));
            assert!(common.fallback_text.contains("Variable"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_repay_with_atokens_max() {
        let call = IPool::repayWithATokensCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::MAX, // Full debt repayment
            interestRateMode: U256::from(2),
        };

        let input = IPool::repayWithATokensCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some());

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("Full debt"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_set_user_use_reserve_as_collateral_enable() {
        let call = IPool::setUserUseReserveAsCollateralCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            useAsCollateral: true,
        };

        let input = IPool::setUserUseReserveAsCollateralCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode setUserUseReserveAsCollateral successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave Collateral Setting");
            assert!(common.fallback_text.contains("Enable"));
            assert!(common.fallback_text.contains("collateral"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_set_user_use_reserve_as_collateral_disable() {
        let call = IPool::setUserUseReserveAsCollateralCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            useAsCollateral: false,
        };

        let input = IPool::setUserUseReserveAsCollateralCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some());

        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("Disable"));
            assert!(common.fallback_text.contains("collateral"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_supply_with_permit() {
        let call = IPool::supplyWithPermitCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            amount: U256::from(1000000000u64),                           // 1000 USDC
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            referralCode: 0,
            deadline: U256::from(1700000000u64),
            permitV: 27,
            permitR: FixedBytes::from([1u8; 32]),
            permitS: FixedBytes::from([2u8; 32]),
        };

        let input = IPool::supplyWithPermitCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode supplyWithPermit successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Supply with Permit");
            assert!(common.fallback_text.contains("Supply"));
            assert!(common.fallback_text.contains("permit"));

            // Verify permit field is present
            if let Some(expanded) = preview_layout.expanded {
                let has_permit_field = expanded.fields.iter().any(|f| {
                    if let SignablePayloadField::TextV2 { common, text_v2 } =
                        &f.signable_payload_field
                    {
                        common.label == "Authorization" && text_v2.text.contains("ERC-2612")
                    } else {
                        false
                    }
                });
                assert!(has_permit_field, "Should have ERC-2612 permit field");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_repay_with_permit() {
        let call = IPool::repayWithPermitCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
            amount: U256::from(500000000u64),                            // 500 USDC
            interestRateMode: U256::from(2),                             // Variable
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            deadline: U256::from(1700000000u64),
            permitV: 27,
            permitR: FixedBytes::from([1u8; 32]),
            permitS: FixedBytes::from([2u8; 32]),
        };

        let input = IPool::repayWithPermitCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(
            result.is_some(),
            "Should decode repayWithPermit successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave Repay with Permit");
            assert!(common.fallback_text.contains("Repay"));
            assert!(common.fallback_text.contains("permit"));
            assert!(common.fallback_text.contains("Variable"));

            // Verify permit field is present
            if let Some(expanded) = preview_layout.expanded {
                let has_permit_field = expanded.fields.iter().any(|f| {
                    if let SignablePayloadField::TextV2 { common, text_v2 } =
                        &f.signable_payload_field
                    {
                        common.label == "Authorization" && text_v2.text.contains("ERC-2612")
                    } else {
                        false
                    }
                });
                assert!(has_permit_field, "Should have ERC-2612 permit field");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_l2_supply_with_permit() {
        let mut args_bytes = [0u8; 32];

        let asset_id: u16 = 12; // USDC on Arbitrum
        let amount: u128 = 1000000000; // 1000 USDC
        let referral_code: u16 = 0;
        let deadline: u32 = 1700000000;
        let permit_v: u8 = 27;

        // Pack according to IL2Pool.sol comment:
        // | 0-padding | permitV | shortenedDeadline | referralCode | shortenedAmount | assetId |
        // |  48 bits  | 8 bits  |     32 bits       |   16 bits    |    128 bits     | 16 bits |
        args_bytes[30..32].copy_from_slice(&asset_id.to_be_bytes());
        args_bytes[14..30].copy_from_slice(&amount.to_be_bytes());
        args_bytes[12..14].copy_from_slice(&referral_code.to_be_bytes());
        args_bytes[8..12].copy_from_slice(&deadline.to_be_bytes());
        args_bytes[7] = permit_v;

        let call = IL2Pool::supplyWithPermitCall {
            args: FixedBytes::from(args_bytes),
            r: FixedBytes::from([1u8; 32]),
            s: FixedBytes::from([2u8; 32]),
        };

        let input = IL2Pool::supplyWithPermitCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 42161, None);

        if result.is_none() {
            eprintln!(
                "Input bytes (first 20): {:?}",
                &input[0..20.min(input.len())]
            );
            eprintln!("Input length: {}", input.len());
        }

        assert!(
            result.is_some(),
            "Should decode L2 supplyWithPermit successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave L2 Supply with Permit");
            assert!(common.fallback_text.contains("Supply"));
            assert!(common.fallback_text.contains("permit"));

            // Verify permit field is present
            if let Some(expanded) = preview_layout.expanded {
                let has_permit_field = expanded.fields.iter().any(|f| {
                    if let SignablePayloadField::TextV2 { common, text_v2 } =
                        &f.signable_payload_field
                    {
                        common.label == "Authorization" && text_v2.text.contains("ERC-2612")
                    } else {
                        false
                    }
                });
                assert!(has_permit_field, "Should have ERC-2612 permit field");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_l2_repay_with_permit() {
        let mut args_bytes = [0u8; 32];

        let asset_id: u16 = 0; // WETH on Arbitrum
        let amount: u128 = 500000000000000000; // 0.5 ETH
        let interest_rate_mode: u8 = 2; // Variable
        let deadline: u32 = 1700000000;
        let permit_v: u8 = 27;

        // Pack according to IL2Pool.sol comment:
        // | 0-padding | permitV | shortenedDeadline | shortenedInterestRateMode | shortenedAmount | assetId |
        // |  64 bits  | 8 bits  |     32 bits       |         8 bits            |    128 bits     | 16 bits |
        args_bytes[30..32].copy_from_slice(&asset_id.to_be_bytes());
        args_bytes[14..30].copy_from_slice(&amount.to_be_bytes());
        args_bytes[13] = interest_rate_mode;
        args_bytes[9..13].copy_from_slice(&deadline.to_be_bytes());
        args_bytes[8] = permit_v;

        let call = IL2Pool::repayWithPermitCall {
            args: FixedBytes::from(args_bytes),
            r: FixedBytes::from([1u8; 32]),
            s: FixedBytes::from([2u8; 32]),
        };

        let input = IL2Pool::repayWithPermitCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 42161, None);

        assert!(
            result.is_some(),
            "Should decode L2 repayWithPermit successfully"
        );

        if let Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) = result
        {
            assert_eq!(common.label, "Aave L2 Repay with Permit");
            assert!(common.fallback_text.contains("Repay"));
            assert!(common.fallback_text.contains("permit"));
            assert!(common.fallback_text.contains("Variable"));

            // Verify permit field is present
            if let Some(expanded) = preview_layout.expanded {
                let has_permit_field = expanded.fields.iter().any(|f| {
                    if let SignablePayloadField::TextV2 { common, text_v2 } =
                        &f.signable_payload_field
                    {
                        common.label == "Authorization" && text_v2.text.contains("ERC-2612")
                    } else {
                        false
                    }
                });
                assert!(has_permit_field, "Should have ERC-2612 permit field");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_visualize_empty_input() {
        let result = PoolVisualizer::new().visualize_pool_operation(&[], 1, None);
        assert_eq!(result, None, "Empty input should return None");
    }

    #[test]
    fn test_visualize_too_short_input() {
        // Input shorter than function selector (4 bytes)
        let result = PoolVisualizer::new().visualize_pool_operation(&[0x01, 0x02, 0x03], 1, None);
        assert_eq!(result, None, "Too-short input should return None");
    }

    #[test]
    fn test_visualize_invalid_function_selector() {
        // Valid length but unrecognized function selector
        let mut invalid_input = vec![0xff, 0xff, 0xff, 0xff]; // Invalid selector
        invalid_input.extend_from_slice(&[0u8; 128]); // Add some data

        let result = PoolVisualizer::new().visualize_pool_operation(&invalid_input, 1, None);
        assert_eq!(
            result, None,
            "Unrecognized function selector should return None"
        );
    }

    #[test]
    fn test_decode_borrow_invalid_interest_rate_mode() {
        let call = IPool::borrowCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::from(1000000u64),
            interestRateMode: U256::from(999), // Invalid
            referralCode: 0,
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
        };

        let input = IPool::borrowCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some());
        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(
                common.fallback_text.contains("Unknown") || common.fallback_text.contains("999")
            );
        }
    }

    #[test]
    fn test_supply_without_registry_shows_raw_address() {
        let call = IPool::supplyCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::from(1000000000u64),
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            referralCode: 0,
        };

        let input = IPool::supplyCall::abi_encode(&call);
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, None);

        assert!(result.is_some());
        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(
                common.fallback_text.contains("0xA0b86991") || common.fallback_text.contains("0xa0b86991")
            );
        }
    }

    #[test]
    fn test_supply_with_registry_shows_token_symbol() {
        let call = IPool::supplyCall {
            asset: address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            amount: U256::from(1000000000u64),
            onBehalfOf: address!("0742d35Cc6634C0532925a3b844Bc9e7595f0bEb"),
            referralCode: 0,
        };

        let input = IPool::supplyCall::abi_encode(&call);
        let registry = ContractRegistry::with_default_protocols();
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, Some(&registry));

        assert!(result.is_some());
        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert!(common.fallback_text.contains("USDC"));
        }
    }

    #[test]
    fn test_real_mainnet_borrow_transaction() {
        let input_hex = "a415bcad000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48000000000000000000000000000000000000000000000000000000003b9aca0000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000b6559478b59836376da9937c4c697ddb21779e49";
        let input = hex::decode(input_hex).unwrap();

        let registry = ContractRegistry::with_default_protocols();
        let result = PoolVisualizer::new().visualize_pool_operation(&input, 1, Some(&registry));

        assert!(result.is_some());
        if let Some(SignablePayloadField::PreviewLayout { common, .. }) = result {
            assert_eq!(common.label, "Aave Borrow");
            assert!(common.fallback_text.contains("USDC"));
            assert!(common.fallback_text.contains("Variable"));
        }
    }
}
