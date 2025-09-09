mod config;

use config::{
    CollectFunctions, Config, LiquidityFunctions, MOMENTUM_CONFIG, MomentumModules, TradeFunctions,
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use crate::presets::momentum::config::{AddLiquidityIndexes, RemoveLiquidityIndexes};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_address_field, create_amount_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::create_text_field,
};

pub struct MomentumVisualizer;

impl CommandVisualizer for MomentumVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Momentum parsing".into(),
            ));
        };

        match pwc.module.as_str().try_into()? {
            MomentumModules::Liquidity => match pwc.function.as_str().try_into()? {
                LiquidityFunctions::RemoveLiquidity => {
                    Ok(Self::handle_remove_liquidity(context, pwc)?)
                }
                LiquidityFunctions::ClosePosition => Ok(Self::handle_close_position(context)?),
                LiquidityFunctions::AddLiquidity => Ok(Self::handle_add_liquidity(context, pwc)?),
                LiquidityFunctions::OpenPosition => Ok(Self::handle_open_position(context, pwc)?),
            },
            MomentumModules::Collect => match pwc.function.as_str().try_into()? {
                CollectFunctions::Fee => Ok(Self::handle_collect_fee(context, pwc)?),
                CollectFunctions::Reward => Ok(Self::handle_collect_reward(context, pwc)?),
            },
            MomentumModules::Trade => match pwc.function.as_str().try_into()? {
                TradeFunctions::FlashSwap => Ok(Self::handle_trade_flash_swap(context, pwc)?),
                TradeFunctions::RepayFlashSwap => {
                    Ok(Self::handle_trade_repay_flash_swap(context, pwc)?)
                }
                TradeFunctions::SwapReceiptDebts => {
                    Ok(Self::handle_trade_swap_receipt_debts(context)?)
                }
                TradeFunctions::FlashLoan => {
                    todo!("Have not found tx for testing yet")
                }
                TradeFunctions::RepayFlashLoan => {
                    todo!("Have not found tx for testing yet")
                }
            },
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(MOMENTUM_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Momentum")
    }
}

impl MomentumVisualizer {
    fn handle_remove_liquidity(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let liquidity = RemoveLiquidityIndexes::get_liquidity(context.inputs(), &pwc.arguments)?;
        let min_amount_x =
            RemoveLiquidityIndexes::get_min_amount_x(context.inputs(), &pwc.arguments)?;
        let min_amount_y =
            RemoveLiquidityIndexes::get_min_amount_y(context.inputs(), &pwc.arguments)?;

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
            create_text_field("Liquidity", &liquidity.to_string())?,
            create_amount_field(
                "Min Amount X",
                &min_amount_x.to_string(),
                coin_1.base_unit_symbol(),
            )?,
            create_amount_field(
                "Min Amount Y",
                &min_amount_y.to_string(),
                coin_2.base_unit_symbol(),
            )?,
        ];

        {
            let title_text = format!(
                "Momentum Remove Liquidity from pair {}/{}",
                coin_1.symbol(),
                coin_2.symbol()
            );
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!(
                        "Remove liquidity from pair {}/{} to {}",
                        coin_1.symbol(),
                        coin_2.symbol(),
                        &context.sender().to_string(),
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
                        label: "Momentum Remove Liquidity Command".to_string(),
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
    }

    fn handle_close_position(
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let list_layout_fields = vec![create_address_field(
            "User Address",
            &context.sender().to_string(),
            None,
            None,
            None,
            None,
        )?];

        {
            let title_text = "Momentum Close Position".to_string();
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!("Close position for {}", &context.sender().to_string()),
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
                        label: "Momentum Close Position Command".to_string(),
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
    }

    fn handle_add_liquidity(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let min_amount_x = AddLiquidityIndexes::get_min_amount_x(context.inputs(), &pwc.arguments)?;
        let min_amount_y = AddLiquidityIndexes::get_min_amount_y(context.inputs(), &pwc.arguments)?;

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
            create_amount_field(
                "Min Amount X",
                &min_amount_x.to_string(),
                coin_1.base_unit_symbol(),
            )?,
            create_amount_field(
                "Min Amount Y",
                &min_amount_y.to_string(),
                coin_2.base_unit_symbol(),
            )?,
        ];

        let title_text = format!(
            "Momentum Add Liquidity to pair {}/{}",
            coin_1.symbol(),
            coin_2.symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Add liquidity to pair {}/{} by {}",
                    coin_1.symbol(),
                    coin_2.symbol(),
                    &context.sender().to_string()
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
                    label: "Momentum Add Liquidity Command".to_string(),
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

    fn handle_open_position(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        // TODO: think how to pipe lower and upper ticks
        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
        ];

        let title_text = "Momentum Open Position".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Open position in pool {}/{}",
                    coin_1.symbol(),
                    coin_2.symbol()
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
                    label: "Momentum Open Position Command".to_string(),
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

    fn handle_collect_fee(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
        ];

        let title_text = "Momentum Collect Fee".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Collect fee from pool {}/{}",
                    coin_1.symbol(),
                    coin_2.symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Momentum Collect Fee Command".to_string(),
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
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
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
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
            create_text_field("Reward Coin", &reward_coin.to_string())?,
        ];

        let title_text = format!("Momentum Collect Reward ({})", reward_coin.symbol());
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Collect rewards ({}) from pool {}/{}",
                    reward_coin.symbol(),
                    coin_1.symbol(),
                    coin_2.symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Momentum Collect Reward Command".to_string(),
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

    fn handle_trade_flash_swap(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
        ];

        let title_text = "Momentum Flash Swap".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!("Flash swap in pool {}/{}", coin_1.symbol(), coin_2.symbol()),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Momentum Flash Swap Command".to_string(),
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

    fn handle_trade_repay_flash_swap(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_text_field("Pool Coin A", &coin_1.to_string())?,
            create_text_field("Pool Coin B", &coin_2.to_string())?,
        ];

        let title_text = "Momentum Repay Flash Swap".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Repay flash swap in pool {}/{}",
                    coin_1.symbol(),
                    coin_2.symbol()
                ),
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Momentum Repay Flash Swap Command".to_string(),
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

    fn handle_trade_swap_receipt_debts(
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let list_layout_fields = vec![create_address_field(
            "User Address",
            &context.sender().to_string(),
            None,
            None,
            None,
            None,
        )?];

        let title_text = "Momentum Swap Receipt Debts".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));
        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field("Summary", "Compute swap receipt debts")?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Momentum Swap Receipt Debts Command".to_string(),
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

    use visualsign::test_utils::assert_has_field;

    #[test]
    fn test_momentum_remove_liquidity() {
        // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
        let test_data = "AQAAAAAACQEBPaCQ0SWho3nWCgPDOKD6unBgRzh8TFJfRUXNyoR8CztrLWshAAAAAAEBAGui4JnRVsicDXzXmGFNQvRmndeFmEicY7+jg9JMG+ZQfMd1JAAAAAAgCSKS99j5XY79h/qhe06kf9pgB7VObJ06G/l6Ud9XAGQAEBxPFM1qAQAAAAAAAAAAAAAACAAAAAAAAAAAAAgAAAAAAAAAAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAABASN1oLHsEgEKrqOyVFrPoq00z7ugPOS1n0w54eJe7RsqZMDJHQAAAAAAACAfXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9QAgH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUFAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eRByZW1vdmVfbGlxdWlkaXR5AgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAHAQAAAQEAAQIAAQMAAQQAAQUAAQYAAQIDAAAAAAMAAAEAAQcAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRB2NvbGxlY3QDZmVlAgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAEAQAAAQEAAQUAAQYAAQIDAgAAAAMCAAEAAQgAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eQ5jbG9zZV9wb3NpdGlvbgACAQEAAQYAH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUCkbzKYJjNnW1dS+OSg47AfzhenXHE5j3YEbVj3w12vjXw4nUkAAAAACBtbk7awxrfKFU5O/j7O18DlbaWBF5AuSr4VpAuZYT9myUkbWMUm6dirPubSAoZWYWkarBH6bfjxezwFmpyxTOW8OJ1JAAAAAAgMZMdJNCNAIa0d8vNuiN4ghW7faU/0/TTTP670s5Pq0ofXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9fQBAAAAAAAAYOMWAAAAAAAAAWEA6Rn4TrqLBl72XmEPSColPnONOY5JiYtLk6F/aQKMWL88mC9+MptS02/JP1+LD8sFsJQD1f8LngMtuLPHny5cAB1S0wCE/sDcB5tDvq1+juWWCcJmS9clXEb99ez37zYB";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Momentum Remove Liquidity Command");
    }

    #[test]
    fn test_momentum_close_position() {
        // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
        let test_data = "AQAAAAAACQEBPaCQ0SWho3nWCgPDOKD6unBgRzh8TFJfRUXNyoR8CztrLWshAAAAAAEBAGui4JnRVsicDXzXmGFNQvRmndeFmEicY7+jg9JMG+ZQfMd1JAAAAAAgCSKS99j5XY79h/qhe06kf9pgB7VObJ06G/l6Ud9XAGQAEBxPFM1qAQAAAAAAAAAAAAAACAAAAAAAAAAAAAgAAAAAAAAAAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAABASN1oLHsEgEKrqOyVFrPoq00z7ugPOS1n0w54eJe7RsqZMDJHQAAAAAAACAfXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9QAgH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUFAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eRByZW1vdmVfbGlxdWlkaXR5AgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAHAQAAAQEAAQIAAQMAAQQAAQUAAQYAAQIDAAAAAAMAAAEAAQcAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRB2NvbGxlY3QDZmVlAgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAEAQAAAQEAAQUAAQYAAQIDAgAAAAMCAAEAAQgAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eQ5jbG9zZV9wb3NpdGlvbgACAQEAAQYAH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUCkbzKYJjNnW1dS+OSg47AfzhenXHE5j3YEbVj3w12vjXw4nUkAAAAACBtbk7awxrfKFU5O/j7O18DlbaWBF5AuSr4VpAuZYT9myUkbWMUm6dirPubSAoZWYWkarBH6bfjxezwFmpyxTOW8OJ1JAAAAAAgMZMdJNCNAIa0d8vNuiN4ghW7faU/0/TTTP670s5Pq0ofXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9fQBAAAAAAAAYOMWAAAAAAAAAWEA6Rn4TrqLBl72XmEPSColPnONOY5JiYtLk6F/aQKMWL88mC9+MptS02/JP1+LD8sFsJQD1f8LngMtuLPHny5cAB1S0wCE/sDcB5tDvq1+juWWCcJmS9clXEb99ez37zYB";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Momentum Close Position Command");
    }

    #[test]
    fn test_momentum_aggregated() {
        run_aggregated_fixture(include_str!("aggregated_test_data.json"), "Momentum");
    }
}
