use crate::core::{CommandVisualizer, VisualizerContext};
use crate::utils::{Coin, create_address_field, get_index};

use move_core_types::runtime_value::MoveValue;
use sui_json::{MoveTypeLayout, SuiJsonValue};
use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
use sui_types::base_types::ObjectID;

use visualsign::{
    SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    field_builders::{create_amount_field, create_text_field},
};

pub struct CetusVisualizer;

impl CommandVisualizer for CetusVisualizer {
    fn visualize_tx_commands(&self, context: &VisualizerContext) -> Option<SignablePayloadField> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return None;
        };

        if pwc.function.contains("swap_b2a") {
            // We can't receive the token amount from the tx data
            let token1_amount =
                get_token1_amount(context.inputs(), &pwc.arguments).unwrap_or_default();
            let token1_coin = get_token_1_coin(&pwc.type_arguments).unwrap_or_default();
            let token2_coin = get_token_2_coin(&pwc.type_arguments).unwrap_or_default();

            return Some(SignablePayloadField::ListLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: "CetusAMM Swap Command".to_string(),
                    label: "CetusAMM Swap Command".to_string(),
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
                        create_address_field(
                            "To",
                            &context.sender().to_string(),
                            None,
                            None,
                            None,
                            None,
                        ),
                        create_amount_field(
                            "Coin 1 Amount",
                            &token1_amount.to_string(),
                            token1_coin.label(),
                        ),
                        create_text_field("Coin 1", token1_coin.label()),
                        create_text_field("Coin 2", token2_coin.label()),
                    ],
                },
            });
        }

        None
    }

    fn can_handle(&self, context: &VisualizerContext) -> bool {
        if let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index()) {
            pwc.package
                == ObjectID::from_hex_literal(
                    "0xb2db7142fa83210a7d78d9c12ac49c043b3cbbd482224fea6e3da00aa5a5ae2d",
                )
                .unwrap()
                && pwc.function.contains("swap_b2a")
        } else {
            false
        }
    }
}

fn get_token_1_coin(type_args: &[String]) -> Option<Coin> {
    if type_args.is_empty() {
        return None;
    }

    type_args[0].parse().ok()
}

fn get_token_2_coin(type_args: &[String]) -> Option<Coin> {
    if type_args.len() == 1 {
        return None;
    }

    type_args[1].parse().ok()
}

fn get_token1_amount(inputs: &[SuiCallArg], args: &[SuiArgument]) -> Option<u64> {
    // TODO: Failed to deconstruct inputs, receive result below. We need to fix tx data decoding
    // Pure(
    //     SuiPureValue {
    //         value_type: None, // HERE! it is not U64
    //         value: [184,198,192,1,0,0,0,0],
    //     },
    // ),
    let amount_input = inputs.get(get_index(args, Some(5))? as usize)?;
    let Ok(MoveValue::U64(decoded_value)) =
        SuiJsonValue::to_move_value(&amount_input.pure()?.to_json_value(), &MoveTypeLayout::U64)
    else {
        return None;
    };
    Some(decoded_value)
}
