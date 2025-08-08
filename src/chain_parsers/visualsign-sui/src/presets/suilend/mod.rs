use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
mod config;

use visualsign::{
    SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    field_builders::{create_amount_field, create_text_field},
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext};
use crate::presets::suilend::config::{LendingMarketFunction, SUILEND_CONFIG};

use crate::utils::{
    SuiCoin, SuiPackage, create_address_field, decode_number, get_index, get_nested_result_value,
    get_tx_type_arg,
};

pub struct SuilendVisualizer;

impl CommandVisualizer for SuilendVisualizer {
    fn visualize_tx_commands(&self, context: &VisualizerContext) -> Option<SignablePayloadField> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return None;
        };

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(_) => return None,
        };

        match function {
            LendingMarketFunction::Repay => {
                let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
                let package: SuiPackage =
                    get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

                let amount = get_repay_amount(context.commands(), context.inputs(), &pwc.arguments)
                    .unwrap_or_default();

                Some(SignablePayloadField::ListLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Suilend Repay Command".to_string(),
                        label: "Suilend Repay Command".to_string(),
                    },
                    list_layout: SignablePayloadFieldListLayout {
                        fields: vec![
                            create_address_field(
                                "From",
                                &context.sender().to_string(),
                                None,
                                None,
                                None,
                                None,
                            ),
                            create_text_field("Package", &package.to_string()),
                            create_text_field("Coin", &coin.to_string()),
                            create_amount_field("Repay Amount", &amount.to_string(), coin.symbol()),
                        ],
                    },
                })
            }
            LendingMarketFunction::ClaimRewardsAndDeposit => {
                let coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
                let package: SuiPackage =
                    get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

                Some(SignablePayloadField::ListLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Suilend Claim Rewards and Deposit Command".to_string(),
                        label: "Suilend Claim Rewards and Deposit Command".to_string(),
                    },
                    list_layout: SignablePayloadFieldListLayout {
                        fields: vec![
                            create_address_field(
                                "From",
                                &context.sender().to_string(),
                                None,
                                None,
                                None,
                                None,
                            ),
                            create_text_field("Package", &package.to_string()),
                            create_text_field("Coin", &coin.to_string()),
                        ],
                    },
                })
            }
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(&*SUILEND_CONFIG)
    }
}

fn get_repay_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    transfer_args: &[SuiArgument],
) -> Option<u64> {
    let command_index_with_input_amount = get_nested_result_value(transfer_args, 4, 0);
    let command_with_input_amount = commands.get(command_index_with_input_amount? as usize)?;

    match command_with_input_amount {
        SuiCommand::SplitCoins(_, args_with_input_index) => {
            let amount_arg = inputs.get(get_index(args_with_input_index, Some(0))? as usize)?;
            decode_number::<u64>(amount_arg)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::transaction_string_to_visual_sign;
    use visualsign::vsptrait::VisualSignOptions;

    const SUILEND_REPAY_LABEL: &str = "Suilend Repay Command";

    #[test]
    fn test_suilend_repay_commands() {
        // https://suivision.xyz/txblock/FTckS194eV3LBGCfcqiW8LxD7E3Nif5MNWqZa21jE5fn
        let test_data = "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";

        let payload = transaction_string_to_visual_sign(
            test_data,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .expect("Failed to visualize tx commands");

        let _ = payload
            .fields
            .iter()
            .find(|f| f.label() == SUILEND_REPAY_LABEL)
            .expect("Should have a Suilend Repay Command field");
    }
}
