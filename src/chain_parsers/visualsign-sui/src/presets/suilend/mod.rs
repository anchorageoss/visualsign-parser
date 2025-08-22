use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
mod config;

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::presets::suilend::config::{SUILEND_CONFIG, SuiLendMarketFunction};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_address_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::{create_amount_field, create_text_field},
};

use crate::utils::{
    SuiCoin, SuiPackage, decode_number, get_index, get_nested_result_value, get_tx_type_arg,
    truncate_address,
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

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(e) => return Err(VisualSignError::DecodeError(e)),
        };

        match function {
            SuiLendMarketFunction::Repay => {
                let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
                let package: SuiPackage =
                    get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

                let amount = get_repay_amount(context.commands(), context.inputs(), &pwc.arguments)
                    .unwrap_or_default();

                {
                    let (title_text, amount_str, amount_field) = match amount {
                        Some(amount) => (
                            format!("Suilend: Repay {} {}", amount, coin.symbol()),
                            amount.to_string(),
                            create_amount_field("Amount", &amount.to_string(), "MIST")?,
                        ),
                        None => (
                            format!("Suilend: Repay {} {}", "N/A", coin.symbol()),
                            "N/A".to_string(),
                            create_text_field("Amount", "N/A MIST")?,
                        ),
                    };

                    let subtitle_text =
                        format!("From {}", truncate_address(&context.sender().to_string()));

                    let condensed = SignablePayloadFieldListLayout {
                        fields: vec![create_text_field(
                            "Summary",
                            &format!("Repay {} {} via {}", amount_str, coin.symbol(), package),
                        )?],
                    };

                    let expanded = SignablePayloadFieldListLayout {
                        fields: vec![
                            create_address_field(
                                "From",
                                &context.sender().to_string(),
                                None,
                                None,
                                None,
                                None,
                            )?,
                            create_text_field("Package", &package.to_string())?,
                            create_text_field("Coin", &coin.to_string())?,
                            amount_field,
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
                                label: "Suilend Repay Command".to_string(),
                            },
                            preview_layout,
                        },
                    }])
                }
            }
            SuiLendMarketFunction::ClaimRewardsAndDeposit => {
                let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
                let package: SuiPackage =
                    get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

                {
                    let title_text =
                        format!("Suilend: Claim Rewards and Deposit ({})", coin.symbol());
                    let subtitle_text =
                        format!("From {}", truncate_address(&context.sender().to_string()));

                    let condensed = SignablePayloadFieldListLayout {
                        fields: vec![create_text_field(
                            "Summary",
                            &format!(
                                "Claim rewards and deposit {} via {}",
                                coin.symbol(),
                                package
                            ),
                        )?],
                    };

                    let expanded = SignablePayloadFieldListLayout {
                        fields: vec![
                            create_address_field(
                                "From",
                                &context.sender().to_string(),
                                None,
                                None,
                                None,
                                None,
                            )?,
                            create_text_field("Package", &package.to_string())?,
                            create_text_field("Coin", &coin.to_string())?,
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
            }
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(&*SUILEND_CONFIG)
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

#[cfg(test)]
mod tests {
    use crate::test_utils::{assert_has_field, payload_from_b64};

    const SUILEND_REPAY_LABEL: &str = "Suilend Repay Command";

    #[test]
    fn test_suilend_repay_commands() {
        // https://suivision.xyz/txblock/FTckS194eV3LBGCfcqiW8LxD7E3Nif5MNWqZa21jE5fn
        let test_data = "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";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, SUILEND_REPAY_LABEL);
    }
}
