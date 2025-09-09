mod config;

use config::{
    BorrowRequestIndexes, ClaimRewardsAndDepositIndexes, ClaimRewardsIndexes, Config,
    DepositCTokensIntoObligationIndexes, DepositLiquidityAndMintCTokensIndexes,
    FulfillLiquidityRequestIndexes, LendingMarketFunctions, RebalanceStakerIndexes,
    RedeemCTokensAndWithdrawLiquidityRequestIndexes, RefreshReservePriceIndexes, SUILEND_CONFIG,
    WithdrawCTokensIndexes,
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{
    SuiCoin, SuiPackage, decode_number, get_index, get_nested_result_value, get_tx_type_arg,
    truncate_address,
};

use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall};

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
};

pub struct SuilendVisualizer;

impl CommandVisualizer for SuilendVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Suilend parsing".into(),
            ));
        };

        match pwc.function.as_str().try_into()? {
            LendingMarketFunctions::BorrowRequest => Self::handle_borrow_request(context, pwc),
            LendingMarketFunctions::ClaimRewards => Self::handle_claim_rewards(context, pwc),
            LendingMarketFunctions::Repay => Self::handle_repay(context, pwc),
            LendingMarketFunctions::ClaimRewardsAndDeposit => {
                Self::handle_claim_rewards_and_deposit(context, pwc)
            }
            LendingMarketFunctions::CreateObligation => {
                Self::handle_create_obligation(context, pwc)
            }
            LendingMarketFunctions::DepositCTokensIntoObligation => {
                Self::handle_deposit_ctokens_into_obligation(context, pwc)
            }
            LendingMarketFunctions::DepositLiquidityAndMintCTokens => {
                Self::handle_deposit_liquidity_and_mint_ctokens(context, pwc)
            }
            LendingMarketFunctions::FulfillLiquidityRequest => {
                Self::handle_fulfill_liquidity_request(context, pwc)
            }
            LendingMarketFunctions::RebalanceStaker => Self::handle_rebalance_staker(context, pwc),
            LendingMarketFunctions::RedeemCTokensAndWithdrawLiquidityRequest => {
                Self::handle_redeem_ctokens_and_withdraw_liquidity_request(context, pwc)
            }
            LendingMarketFunctions::RefreshReservePrice => {
                Self::handle_refresh_reserve_price(context, pwc)
            }
            LendingMarketFunctions::UnstakeSuiFromStaker => {
                Self::handle_unstake_sui_from_staker(context, pwc)
            }
            LendingMarketFunctions::WithdrawCTokens => Self::handle_withdraw_ctokens(context, pwc),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(SUILEND_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending("Suilend")
    }
}

fn get_repay_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    transfer_args: &[SuiArgument],
) -> Result<Option<u64>, VisualSignError> {
    let command_index_with_input_amount = get_nested_result_value(transfer_args, 4, 0);
    let command_with_input_amount = commands
        .get(command_index_with_input_amount? as usize)
        .ok_or(VisualSignError::MissingData("Command not found".into()))?;

    match command_with_input_amount {
        SuiCommand::SplitCoins(_, args_with_input_index) => {
            let amount_arg = inputs
                .get(get_index(args_with_input_index, Some(0))? as usize)
                .ok_or(VisualSignError::MissingData(
                    "Amount argument not found".into(),
                ))?;
            Ok(Some(decode_number::<u64>(amount_arg)?))
        }
        _ => Ok(None),
    }
}

impl SuilendVisualizer {
    fn handle_borrow_request(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index =
            BorrowRequestIndexes::get_reserve_array_index(context.inputs(), &pwc.arguments)?;
        let amount = BorrowRequestIndexes::get_amount(context.inputs(), &pwc.arguments)?;

        let title_text = format!("Suilend: Borrow Request {} {}", amount, coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Borrow {} {} from reserve #{} via {}",
                    amount,
                    coin.symbol(),
                    reserve_index,
                    package
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Borrowed Coin", &coin.to_string())?,
                create_text_field("Borrowed Reserve Index", &reserve_index.to_string())?,
                create_amount_field("Borrowed Amount", &amount.to_string(), coin.symbol())?,
            ],
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Borrow Request".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_claim_rewards(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let reward_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_id = ClaimRewardsIndexes::get_reserve_id(context.inputs(), &pwc.arguments)?;
        let reward_index = ClaimRewardsIndexes::get_reward_index(context.inputs(), &pwc.arguments)?;
        let is_deposit =
            ClaimRewardsIndexes::get_is_deposit_reward(context.inputs(), &pwc.arguments)?;

        let reward_side = if is_deposit { "Deposit" } else { "Borrow" };
        let title_text = format!(
            "Suilend: Claim Rewards ({}) {}",
            reward_side,
            reward_coin.symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Claim {reward_side} rewards from reserve #{reserve_id} (reward #{reward_index}) via {package}"
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Reward Coin", &reward_coin.to_string())?,
                create_text_field("Claim Rewards Reserve Index", &reserve_id.to_string())?,
                create_text_field("Claim Reward Index", &reward_index.to_string())?,
                create_text_field("Claim Reward Side", reward_side)?,
            ],
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Claim Rewards".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_claim_rewards_and_deposit(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reward_reserve_id =
            ClaimRewardsAndDepositIndexes::get_reward_reserve_id(context.inputs(), &pwc.arguments)?;
        let reward_index =
            ClaimRewardsAndDepositIndexes::get_reward_index(context.inputs(), &pwc.arguments)?;
        let is_deposit =
            ClaimRewardsAndDepositIndexes::get_is_deposit_reward(context.inputs(), &pwc.arguments)?;
        let deposit_reserve_id = ClaimRewardsAndDepositIndexes::get_deposit_reserve_id(
            context.inputs(),
            &pwc.arguments,
        )?;

        let reward_side = if is_deposit { "Deposit" } else { "Borrow" };
        let title_text = format!("Suilend: Claim Rewards and Deposit ({})", coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Claim {reward_side} rewards from reserve #{reward_reserve_id} (reward #{reward_index}) and deposit to reserve #{deposit_reserve_id} via {package}"
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Claim and Deposit Coin", &coin.to_string())?,
                create_text_field("Reward Side", reward_side)?,
                create_text_field("Reward Reserve Index", &reward_reserve_id.to_string())?,
                create_text_field("Reward Index", &reward_index.to_string())?,
                create_text_field("Deposit Reserve Index", &deposit_reserve_id.to_string())?,
            ],
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Claim Rewards and Deposit Command".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_create_obligation(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let title_text = "Suilend: Create Obligation".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Create new obligation via {package}"),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Create Obligation".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_deposit_ctokens_into_obligation(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index = DepositCTokensIntoObligationIndexes::get_reserve_array_index(
            context.inputs(),
            &pwc.arguments,
        )?;

        let title_text = format!("Suilend: Deposit cTokens ({})", coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Deposit cTokens of {} into obligation from reserve #{} via {}",
                    coin.symbol(),
                    reserve_index,
                    package
                ),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("CToken (underlying)", &coin.to_string())?,
                create_text_field("Deposit cTokens Reserve Index", &reserve_index.to_string())?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Deposit cTokens".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_deposit_liquidity_and_mint_ctokens(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index = DepositLiquidityAndMintCTokensIndexes::get_reserve_array_index(
            context.inputs(),
            &pwc.arguments,
        )?;

        let title_text = format!(
            "Suilend: Deposit Liquidity and Mint cTokens ({})",
            coin.symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Deposit liquidity of {} into reserve #{} via {}",
                    coin.symbol(),
                    reserve_index,
                    package
                ),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Deposit Liquidity and Mint cTokens Coin", &coin.to_string())?,
                create_text_field(
                    "Deposit Liquidity and Mint cTokens Reserve Index",
                    &reserve_index.to_string(),
                )?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Deposit Liquidity".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_fulfill_liquidity_request(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index = FulfillLiquidityRequestIndexes::get_reserve_array_index(
            context.inputs(),
            &pwc.arguments,
        )?;
        let title_text = format!("Suilend: Fulfill Liquidity Request ({})", coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Fulfill liquidity request for reserve #{reserve_index} via {package}"),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Fulfill Liquidity Coin", &coin.to_string())?,
                create_text_field(
                    "Fulfill Liquidity Reserve Index",
                    &reserve_index.to_string(),
                )?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Fulfill Liquidity".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_rebalance_staker(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let sui_reserve_index =
            RebalanceStakerIndexes::get_sui_reserve_array_index(context.inputs(), &pwc.arguments)?;
        let title_text = "Suilend: Rebalance Staker".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Rebalance SUI staker for reserve #{sui_reserve_index} via {package}"),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field(
                    "Rebalance Staker SUI Reserve Index",
                    &sui_reserve_index.to_string(),
                )?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Rebalance Staker".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_redeem_ctokens_and_withdraw_liquidity_request(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index =
            RedeemCTokensAndWithdrawLiquidityRequestIndexes::get_reserve_array_index(
                context.inputs(),
                &pwc.arguments,
            )?;
        let title_text = format!(
            "Suilend: Redeem cTokens and Withdraw Liquidity ({})",
            coin.symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Redeem cTokens of {} and withdraw liquidity from reserve #{} via {}",
                    coin.symbol(),
                    reserve_index,
                    package
                ),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field(
                    "Redeem cTokens and Withdraw Liquidity Coin",
                    &coin.to_string(),
                )?,
                create_text_field(
                    "Redeem cTokens and Withdraw Liquidity Reserve Index",
                    &reserve_index.to_string(),
                )?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Redeem cTokens".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_refresh_reserve_price(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index =
            RefreshReservePriceIndexes::get_reserve_array_index(context.inputs(), &pwc.arguments)?;
        let title_text = "Suilend: Refresh Reserve Price".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Refresh price for reserve #{reserve_index} via {package}"),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Refresh Price Reserve Index", &reserve_index.to_string())?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Refresh Reserve Price".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_repay(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index =
            RefreshReservePriceIndexes::get_reserve_array_index(context.inputs(), &pwc.arguments)?;
        let amount = get_repay_amount(context.commands(), context.inputs(), &pwc.arguments)
            .unwrap_or_default();

        let (title_text, amount_str, amount_field) = match amount {
            Some(amount) => (
                format!("Suilend: Repay {} {}", amount, coin.symbol()),
                amount.to_string(),
                create_amount_field("Repay Amount", &amount.to_string(), coin.symbol())?,
            ),
            None => (
                format!("Suilend: Repay {} {}", "N/A", coin.symbol()),
                "N/A".to_string(),
                create_text_field("Repay Amount", "N/A")?,
            ),
        };

        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let mut summary = format!("Repay {} {} via {}", amount_str, coin.symbol(), package);
        summary.push_str(&format!(" (reserve #{reserve_index})"));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field("Summary", &summary)?],
        };

        let expanded_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Address", &package.to_string())?,
            create_text_field("Repay Coin", &coin.to_string())?,
            amount_field,
            create_text_field("Repay Reserve Index", &reserve_index.to_string())?,
        ];

        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Repay Command".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_unstake_sui_from_staker(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        // Uses SUI reserve index at arg 1
        let sui_reserve_index =
            RebalanceStakerIndexes::get_sui_reserve_array_index(context.inputs(), &pwc.arguments)?;

        let title_text = "Suilend: Unstake SUI from Staker".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Unstake SUI from staker (reserve #{sui_reserve_index}) via {package}"),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field(
                    "Unstake SUI from Staker Reserve Index",
                    &sui_reserve_index.to_string(),
                )?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Unstake SUI".to_string(),
                },
                preview_layout,
            },
        }])
    }

    fn handle_withdraw_ctokens(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let package: SuiPackage = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let reserve_index =
            WithdrawCTokensIndexes::get_reserve_array_index(context.inputs(), &pwc.arguments)?;
        let amount = WithdrawCTokensIndexes::get_amount(context.inputs(), &pwc.arguments)?;

        let title_text = format!("Suilend: Withdraw cTokens ({})", coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Withdraw {} cTokens ({}) from reserve #{} via {}",
                    amount,
                    coin.symbol(),
                    reserve_index,
                    package
                ),
            )?],
        };
        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_address_field(
                    "User Address",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Pool Address", &package.to_string())?,
                create_text_field("Withdraw cTokens Coin", &coin.to_string())?,
                create_amount_field(
                    "Withdraw cTokens Amount",
                    &amount.to_string(),
                    coin.symbol(),
                )?,
                create_text_field("Withdraw cTokens Reserve Index", &reserve_index.to_string())?,
            ],
        };
        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: title_text.clone(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text,
                    label: "Suilend Withdraw cTokens".to_string(),
                },
                preview_layout,
            },
        }])
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::run_aggregated_fixture;

    #[test]
    fn test_suilend_aggregated() {
        run_aggregated_fixture(include_str!("aggregated_test_data.json"), "Suilend");
    }
}
