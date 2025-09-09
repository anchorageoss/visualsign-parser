mod config;

use config::{
    AddLiquidityByFixCoinIndexes, CETUS_CONFIG, CetusModules, Config,
    OpenPositionWithLiquidityByFixCoinIndexes, PoolScriptClosePositionIndexes, PoolScriptFunctions,
    PoolScriptRemoveLiquidityIndexes, PoolScriptSwapA2BIndexes,
    PoolScriptSwapA2BWithPartnerIndexes, PoolScriptSwapB2AIndexes,
    PoolScriptSwapB2AWithPartnerIndexes, PoolScriptV2Functions, RouterCheckCoinThresholdIndexes,
    RouterFunctions, RouterSwapIndexes, SwapA2BIndexes, SwapB2AIndexes, UtilsFunctions,
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use crate::presets::cetus::config::{
    PoolScriptV3Functions, SwapA2BWithPartnerIndexes, SwapB2AWithPartnerIndexes,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
};

pub struct CetusVisualizer;

impl CommandVisualizer for CetusVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Cetus parsing".into(),
            ));
        };

        match pwc.module.as_str().try_into()? {
            CetusModules::PoolScript => match pwc.function.as_str().try_into()? {
                PoolScriptFunctions::SwapA2B => Self::handle_swap_pool_script(true, context, pwc),
                PoolScriptFunctions::SwapB2A => Self::handle_swap_pool_script(false, context, pwc),
                PoolScriptFunctions::SwapA2BWithPartner => {
                    Self::handle_swap_pool_script_with_partner(true, context, pwc)
                }
                PoolScriptFunctions::SwapB2AWithPartner => {
                    Self::handle_swap_pool_script_with_partner(false, context, pwc)
                }
                PoolScriptFunctions::ClosePosition => {
                    Self::handle_close_position_pool_script(context, pwc)
                }
                PoolScriptFunctions::RemoveLiquidity => {
                    Self::handle_remove_liquidity_pool_script(context, pwc)
                }
                PoolScriptFunctions::OpenPositionWithLiquidityWithAll => {
                    Self::handle_pool_script_liquidity_ops(context, pwc)
                }
            },
            CetusModules::PoolScriptV2 => match pwc.function.as_str().try_into()? {
                PoolScriptV2Functions::SwapB2A => Self::handle_swap_v2(false, context, pwc),
                PoolScriptV2Functions::SwapA2B => Self::handle_swap_v2(true, context, pwc),
                PoolScriptV2Functions::SwapA2BWithPartner => {
                    Self::handle_swap_v2_with_partner(true, context, pwc)
                }
                PoolScriptV2Functions::SwapB2AWithPartner => {
                    Self::handle_swap_v2_with_partner(false, context, pwc)
                }
                PoolScriptV2Functions::CollectReward => Self::handle_collect_reward(context, pwc),
                PoolScriptV2Functions::CollectFee => Self::handle_collect_fee(context, pwc),
                PoolScriptV2Functions::OpenPositionWithLiquidityByFixCoin => {
                    Self::handle_open_position_with_liquidity_by_fix_coin_v2(context, pwc)
                }
                PoolScriptV2Functions::AddLiquidityByFixCoin => {
                    Self::handle_add_liquidity_by_fix_coin_v2(context, pwc)
                }
            },
            CetusModules::Router => match pwc.function.as_str().try_into()? {
                RouterFunctions::Swap => Self::handle_router_swap(context, pwc),
                RouterFunctions::CheckCoinThreshold => {
                    Self::handle_check_coin_threshold(context, pwc)
                }
            },
            CetusModules::Utils => match pwc.function.as_str().try_into()? {
                UtilsFunctions::TransferCoinToSender => {
                    Self::handle_transfer_coin_to_sender(context, pwc)
                }
            },
            CetusModules::PoolScriptV3 => match pwc.function.as_str().try_into()? {
                PoolScriptV3Functions::CollectFee => Self::handle_collect_fee(context, pwc),
                PoolScriptV3Functions::CollectReward => Self::handle_collect_reward(context, pwc),
            },
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(CETUS_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Cetus")
    }
}

impl CetusVisualizer {
    fn handle_swap_v2(
        is_a2b: bool,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (by_amount_in, amount, amount_limit) = if is_a2b {
            (
                SwapA2BIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapA2BIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapA2BIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        } else {
            (
                SwapB2AIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapB2AIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapB2AIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        };

        Self::render_swap_fields(context, by_amount_in, amount, amount_limit, is_a2b, pwc)
    }

    fn handle_swap_v2_with_partner(
        is_a2b: bool,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (by_amount_in, amount, amount_limit) = if is_a2b {
            (
                SwapA2BWithPartnerIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapA2BWithPartnerIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapA2BWithPartnerIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        } else {
            (
                SwapB2AWithPartnerIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapB2AWithPartnerIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapB2AWithPartnerIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        };

        Self::render_swap_fields(context, by_amount_in, amount, amount_limit, is_a2b, pwc)
    }

    fn handle_swap_pool_script(
        is_a2b: bool,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (by_amount_in, amount, amount_limit) = if is_a2b {
            (
                PoolScriptSwapA2BIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapA2BIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapA2BIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        } else {
            (
                PoolScriptSwapB2AIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapB2AIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapB2AIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        };

        Self::render_swap_fields(context, by_amount_in, amount, amount_limit, is_a2b, pwc)
    }

    fn handle_swap_pool_script_with_partner(
        is_a2b: bool,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (by_amount_in, amount, amount_limit) = if is_a2b {
            (
                PoolScriptSwapA2BWithPartnerIndexes::get_by_amount_in(
                    context.inputs(),
                    &pwc.arguments,
                )?,
                PoolScriptSwapA2BWithPartnerIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapA2BWithPartnerIndexes::get_amount_limit(
                    context.inputs(),
                    &pwc.arguments,
                )?,
            )
        } else {
            (
                PoolScriptSwapB2AWithPartnerIndexes::get_by_amount_in(
                    context.inputs(),
                    &pwc.arguments,
                )?,
                PoolScriptSwapB2AWithPartnerIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                PoolScriptSwapB2AWithPartnerIndexes::get_amount_limit(
                    context.inputs(),
                    &pwc.arguments,
                )?,
            )
        };

        Self::render_swap_fields(context, by_amount_in, amount, amount_limit, is_a2b, pwc)
    }

    fn handle_router_swap(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let is_a2b = RouterSwapIndexes::get_is_a2b(context.inputs(), &pwc.arguments)?;
        let by_amount_in = RouterSwapIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?;
        let amount = RouterSwapIndexes::get_amount(context.inputs(), &pwc.arguments)?;

        // Router has no explicit min-out/max-in; show 0 and surface price-limit knobs.
        let amount_limit = 0u64;

        let sqrt_price_limit =
            RouterSwapIndexes::get_sqrt_price_limit(context.inputs(), &pwc.arguments)?;
        let use_all_coin = RouterSwapIndexes::get_use_all_coin(context.inputs(), &pwc.arguments)?;

        let (input_coin, output_coin): (SuiCoin, SuiCoin) =
            Self::determine_input_output_coins(is_a2b, pwc);
        let (primary_label, primary_symbol, limit_label, limit_symbol) =
            Self::determine_primary_limit_labels(&input_coin, &output_coin, by_amount_in);

        let mut list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_amount_field(primary_label, &amount.to_string(), primary_symbol)?,
            create_text_field("Input Coin", &input_coin.to_string())?,
            create_amount_field(limit_label, &amount_limit.to_string(), limit_symbol)?,
            create_text_field("Output Coin", &output_coin.to_string())?,
        ];

        let price_limit_text = if sqrt_price_limit == 0 {
            "None".to_string()
        } else {
            sqrt_price_limit.to_string()
        };
        list_layout_fields.push(create_text_field("Sqrt Price Limit", &price_limit_text)?);
        list_layout_fields.push(create_text_field(
            "Use All Coin",
            &use_all_coin.to_string(),
        )?);

        let title_text = format!(
            "CetusAMM Swap: {} {} → {}",
            amount,
            input_coin.base_unit_symbol(),
            output_coin.base_unit_symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let price_hint = if sqrt_price_limit == 0 { "none" } else { "set" };
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Swap {} to {} ({}: {} {}, Price Limit: {})",
                    input_coin.base_unit_symbol(),
                    output_coin.base_unit_symbol(),
                    limit_label,
                    amount_limit,
                    if by_amount_in {
                        output_coin.base_unit_symbol()
                    } else {
                        input_coin.base_unit_symbol()
                    },
                    price_hint,
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: list_layout_fields,
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Swap Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(expanded),
                },
            },
        }])
    }

    fn determine_input_output_coins(
        is_a2b: bool,
        pwc: &SuiProgrammableMoveCall,
    ) -> (SuiCoin, SuiCoin) {
        if is_a2b {
            (
                get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default(),
                get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default(),
            )
        } else {
            (
                get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default(),
                get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default(),
            )
        }
    }

    fn determine_primary_limit_labels<'a>(
        input_coin: &'a SuiCoin,
        output_coin: &'a SuiCoin,
        by_amount_in: bool,
    ) -> (&'a str, &'a str, &'a str, &'a str) {
        if by_amount_in {
            (
                "Amount In",
                input_coin.base_unit_symbol(),
                "Min Out",
                output_coin.base_unit_symbol(),
            )
        } else {
            (
                "Amount Out",
                output_coin.base_unit_symbol(),
                "Max In",
                input_coin.base_unit_symbol(),
            )
        }
    }

    fn render_swap_fields(
        context: &VisualizerContext,
        by_amount_in: bool,
        amount: u64,
        amount_limit: u64,
        is_a2b: bool,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (input_coin, output_coin): (SuiCoin, SuiCoin) =
            Self::determine_input_output_coins(is_a2b, pwc);

        let (primary_label, primary_symbol, limit_label, limit_symbol) =
            Self::determine_primary_limit_labels(&input_coin, &output_coin, by_amount_in);

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_amount_field(primary_label, &amount.to_string(), primary_symbol)?,
            create_text_field("Input Coin", &input_coin.to_string())?,
            create_amount_field(limit_label, &amount_limit.to_string(), limit_symbol)?,
            create_text_field("Output Coin", &output_coin.to_string())?,
        ];

        let title_text = format!(
            "CetusAMM Swap: {} {} → {}",
            amount,
            input_coin.base_unit_symbol(),
            output_coin.base_unit_symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Swap {} to {} ({}: {} {})",
                    input_coin.base_unit_symbol(),
                    output_coin.base_unit_symbol(),
                    limit_label,
                    amount_limit,
                    if by_amount_in {
                        output_coin.base_unit_symbol()
                    } else {
                        input_coin.base_unit_symbol()
                    }
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: list_layout_fields,
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Swap Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(expanded),
                },
            },
        }])
    }

    fn handle_check_coin_threshold(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let threshold =
            RouterCheckCoinThresholdIndexes::get_threshold(context.inputs(), &pwc.arguments)?;

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Coin", &coin.to_string())?,
            create_amount_field("Threshold", &threshold.to_string(), coin.base_unit_symbol())?,
        ];

        let title_text = format!(
            "Cetus Router: Check Coin Threshold {} {}",
            threshold,
            coin.base_unit_symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Check {} balance threshold {}",
                    coin.base_unit_symbol(),
                    threshold
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Cetus Router Check Coin Threshold".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_collect_reward(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let reward_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 2).unwrap_or_default();

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
            create_text_field("Reward Coin", &reward_coin.to_string())?,
        ];

        let title_text = format!(
            "CetusAMM Collect Reward ({})",
            reward_coin.base_unit_symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Collect rewards ({}) from pool {}/{}",
                    reward_coin.base_unit_symbol(),
                    coin_a.base_unit_symbol(),
                    coin_b.base_unit_symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Collect Reward Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_collect_fee(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
        ];

        let title_text = "CetusAMM Collect Fee".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Collect fee from pool {}/{}",
                    coin_a.base_unit_symbol(),
                    coin_b.base_unit_symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Collect Fee Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_close_position_pool_script(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let min_a =
            PoolScriptClosePositionIndexes::get_min_amount_a(context.inputs(), &pwc.arguments)?;
        let min_b =
            PoolScriptClosePositionIndexes::get_min_amount_b(context.inputs(), &pwc.arguments)?;

        Self::render_close_position(context, &coin_a, &coin_b, min_a, min_b)
    }

    fn render_close_position(
        context: &VisualizerContext,
        coin_a: &SuiCoin,
        coin_b: &SuiCoin,
        min_a: u64,
        min_b: u64,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
            create_amount_field("Min Out A", &min_a.to_string(), coin_a.base_unit_symbol())?,
            create_amount_field("Min Out B", &min_b.to_string(), coin_b.base_unit_symbol())?,
        ];

        let title_text = "CetusAMM Close Position".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Close position and withdraw at least {} {} and {} {}",
                    min_a,
                    coin_a.base_unit_symbol(),
                    min_b,
                    coin_b.base_unit_symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Close Position Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_remove_liquidity_pool_script(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let liquidity =
            PoolScriptRemoveLiquidityIndexes::get_liquidity(context.inputs(), &pwc.arguments)?;
        let min_a =
            PoolScriptRemoveLiquidityIndexes::get_min_amount_a(context.inputs(), &pwc.arguments)?;
        let min_b =
            PoolScriptRemoveLiquidityIndexes::get_min_amount_b(context.inputs(), &pwc.arguments)?;

        Self::render_remove_liquidity(context, &coin_a, &coin_b, liquidity, min_a, min_b)
    }

    fn render_remove_liquidity(
        context: &VisualizerContext,
        coin_a: &SuiCoin,
        coin_b: &SuiCoin,
        liquidity: u128,
        min_a: u64,
        min_b: u64,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
            create_amount_field("Liquidity", &liquidity.to_string(), "RAW")?,
            create_amount_field("Min Out A", &min_a.to_string(), coin_a.base_unit_symbol())?,
            create_amount_field("Min Out B", &min_b.to_string(), coin_b.base_unit_symbol())?,
        ];

        let title_text = "CetusAMM Remove Liquidity".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Remove liquidity {} from {}/{} (min {} {}, {} {})",
                    liquidity,
                    coin_a.base_unit_symbol(),
                    coin_b.base_unit_symbol(),
                    min_a,
                    coin_a.base_unit_symbol(),
                    min_b,
                    coin_b.base_unit_symbol()
                ),
            )?],
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Remove Liquidity Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_add_liquidity_by_fix_coin_v2(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let amount_a =
            AddLiquidityByFixCoinIndexes::get_amount_a(context.inputs(), &pwc.arguments)?;
        let amount_b =
            AddLiquidityByFixCoinIndexes::get_amount_b(context.inputs(), &pwc.arguments)?;
        let is_fix_a =
            AddLiquidityByFixCoinIndexes::get_is_fix_a(context.inputs(), &pwc.arguments)?;

        let fix_coin = if is_fix_a { &coin_a } else { &coin_b };

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
            create_text_field("Fix Coin", &fix_coin.to_string())?,
            create_amount_field("Amount A", &amount_a.to_string(), coin_a.base_unit_symbol())?,
            create_amount_field("Amount B", &amount_b.to_string(), coin_b.base_unit_symbol())?,
        ];

        let title_text = "CetusAMM Add Liquidity (Fix Coin)".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Add liquidity with {} fixed (A: {} {}, B: {} {})",
                    fix_coin.base_unit_symbol(),
                    amount_a,
                    coin_a.base_unit_symbol(),
                    amount_b,
                    coin_b.base_unit_symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Add Liquidity (Fix Coin) Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_open_position_with_liquidity_by_fix_coin_v2(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let amount_a = OpenPositionWithLiquidityByFixCoinIndexes::get_amount_a(
            context.inputs(),
            &pwc.arguments,
        )?;
        let amount_b = OpenPositionWithLiquidityByFixCoinIndexes::get_amount_b(
            context.inputs(),
            &pwc.arguments,
        )?;
        let is_fix_a = OpenPositionWithLiquidityByFixCoinIndexes::get_is_fix_a(
            context.inputs(),
            &pwc.arguments,
        )?;

        Self::render_open_position_with_liquidity(
            context, &coin_a, &coin_b, amount_a, amount_b, is_fix_a,
        )
    }

    fn render_open_position_with_liquidity(
        context: &VisualizerContext,
        coin_a: &SuiCoin,
        coin_b: &SuiCoin,
        amount_a: u64,
        amount_b: u64,
        is_fix_a: bool,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let fix_coin = if is_fix_a { &coin_a } else { &coin_b };
        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_a.to_string())?,
            create_text_field("Pool Coin B", &coin_b.to_string())?,
            create_text_field("Fix Coin", &fix_coin.to_string())?,
            create_amount_field("Amount A", &amount_a.to_string(), coin_a.base_unit_symbol())?,
            create_amount_field("Amount B", &amount_b.to_string(), coin_b.base_unit_symbol())?,
        ];

        let title_text = "CetusAMM Open Position With Liquidity".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Open position with {} fixed (A: {} {}, B: {} {})",
                    fix_coin.base_unit_symbol(),
                    amount_a,
                    coin_a.base_unit_symbol(),
                    amount_b,
                    coin_b.base_unit_symbol()
                ),
            )?],
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Open Position With Liquidity Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }

    fn handle_pool_script_liquidity_ops(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        // For pool_script liquidity-related helpers, just show coin types; amounts are already
        // captured in other flows or depend on nested receipts. Keep a simple summary.
        let coin_a: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_b: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let title_text = "CetusAMM Liquidity Operation".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Operate liquidity on pool {}/{}",
                    coin_a.base_unit_symbol(),
                    coin_b.base_unit_symbol()
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
                create_text_field("Pool Coin A", &coin_a.to_string())?,
                create_text_field("Pool Coin B", &coin_b.to_string())?,
            ],
        };
        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "CetusAMM Liquidity Command".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(expanded),
                },
            },
        }])
    }

    fn handle_transfer_coin_to_sender(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Coin", &coin.to_string())?,
        ];

        let title_text = format!(
            "Cetus Utils: Transfer {} to Sender",
            coin.base_unit_symbol()
        );
        let subtitle_text = format!("To {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Transfer {} to sender", coin.base_unit_symbol()),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Cetus Utils Transfer Coin To Sender".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: subtitle_text,
                    }),
                    condensed: Some(condensed),
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: list_layout_fields,
                    }),
                },
            },
        }])
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::{payload_from_b64, run_aggregated_fixture};

    use visualsign::test_utils::{assert_has_field, assert_has_field_with_value};

    const CETUS_SWAP_LABEL: &str = "CetusAMM Swap Command";

    #[test]
    fn test_cetus_amm_swap_b2a_commands() {
        // https://suivision.xyz/txblock/7Je4yeXMvvEHFcRSTD4WYv3eSsaDk2zqvdoSxWXdUYGx
        let test_data = "AQAAAAAACQEAEXs/ewhS1RZrUZQ2xQEliCJn40SK4PvEV75r2SGFMXhjUsAjAAAAACBSKqlrLdPXYeuzckz31NAkeSO09qmNPv/pkWggJMTC2QAIuMbAAQAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFK94o+ni1sq8pdp5wea/9ImVZqQhMh/DtaYZZkAXpg1nkOqBoAAAAAAQABAQAIuMbAAQAAAAAACI0+GgMAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAQAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB7eETiiahBDlD7PKSNaeuc8p4n0iPvkDU/4b2OJ/+PP4BGNvaW4EQ09JTgAJAQIAAQMAAgEAAgAAAQQAAQUAAQYAAQcAAQgArltnUkfA5IdctLm9N6YO1bz4kng0TThA3StCbiinZoUBZI8YcdbCiGOtIFCZV/M9U6lZTgf3lg6t7feHRsBBqR1jUsAjAAAAACCmwR6aeqn8D632smpzU9fbDhP3vPOQhgc806IrzekPH65bZ1JHwOSHXLS5vTemDtW8+JJ4NE04QN0rQm4op2aFBQIAAAAAAAC8YDQAAAAAAAABYQAdbFpPHuOPe/TYRMttj4FSzAN1ErZdI75GooTkFmiIVkvCM+lnSS3pR/qQt6j7K3gsrtBExfgOL/dffWapvuMEyeP1ig9kZWEaY4lMw99QxRTo2PcUhKsb1gquOOAGXP8=";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0xae5b675247c0e4875cb4b9bd37a60ed5bcf89278344d3840dd2b426e28a76685",
        );
        assert_has_field_with_value(&payload, "Amount In", "29411000");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xb7844e289a8410e50fb3ca48d69eb9cf29e27d223ef90353fe1bd8e27ff8f3f8::coin::COIN",
        );
        assert_has_field_with_value(&payload, "Min Out", "52051597");
        assert_has_field_with_value(
            &payload,
            "Output Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
    }

    #[test]
    fn test_cetus_amm_swap_a2b_commands() {
        // https://suivision.xyz/txblock/7t6iLtevYDEpXrr3rhpmDcwf8cMMV1sgspppvvnXiguR
        let test_data = "AQAAAAAACgEAkfGWz0JGPLt14gdQVPgAPvGv100NtFt2InDcGDyMZQRIPXMkAAAAACBzf79a+nciTqmPBgQycQyP7VMyWjP2waulu8LKtlZ2ggEA4pyQKsylAKpoN702neQpT4smpbXaopiWRMOhnNQk+dxIPXMkAAAAACDPA2LUvkZAkhsDL9IAPA5XEMTFk44RZFMN/UrpVT0aOwAIqBEHZwAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFR6IO6fAtWaibLyKlM0z6wq9QYp3zB5grSL9mx8pzSq/uacRYAAAAAAQABAAAIAIhSanQAAAAACKgRB2cAAAAAABBQOwEAAQAAAAAAAAAAAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAQDAQAAAQEBAAIBAAABAQIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBGNvaW4EemVybwEHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYTJiAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACA3N1aQNTVUkACQEDAAEEAAIBAAICAAEFAAEGAAEHAAEIAAEJABxoihUeyy/EpwFkjSZ1URYLuz/mqwyC1srAaO1syYLJAoo43JFE0wNBKCG9zE6pVWaENgEg5OfSW8ZgI9xfwZ0JSD1zJAAAAAAgHKrS7Xzyr+wSIwY1SfiwUh3kR/gsbnB5wy14YgB8JlUweXl6kiml0me3PakkjYuFIPJ+CJMElcVq6NGPtGy26Ug9cyQAAAAAINXWx8S5GTFIRWp1oY/IAkEhRVrywXZhCYVXXzVPcQ7fHGiKFR7LL8SnAWSNJnVRFgu7P+arDILWysBo7WzJgsn0AQAAAAAAAOhyLwAAAAAAAAFhAIB1hvQj0FnB2h+j3lZjYL1en1K3A7ITWXhVpj1Oslz0FVgkC3Es9xS5JGDgXByYelNgSJ4bFzB+Sn+9LwOJVAKAiyGXhnmh12WYynVXlQH2doDZ0v5LCrXENXPauhOQWQ==";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0x1c688a151ecb2fc4a701648d267551160bbb3fe6ab0c82d6cac068ed6cc982c9",
        );
        assert_has_field_with_value(&payload, "Max In", "1728516520");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
        assert_has_field_with_value(&payload, "Amount Out", "500000000000");
        assert_has_field_with_value(&payload, "Output Coin", "0x2::sui::SUI");
    }

    #[test]
    fn test_cetus_amm_aggregated() {
        run_aggregated_fixture(include_str!("aggregated_test_data.json"), "Cetus");
    }
}
