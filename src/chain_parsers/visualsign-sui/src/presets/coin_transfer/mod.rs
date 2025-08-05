use crate::core::{CommandVisualizer, VisualizerContext};
use crate::utils::{CoinObject, create_address_field, get_index, parse_numeric_argument};

use move_core_types::runtime_value::MoveValue;
use sui_json::{MoveTypeLayout, SuiJsonValue};
use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

use visualsign::{
    SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    field_builders::{create_amount_field, create_text_field},
};

pub struct CoinTransferVisualizer;

impl CommandVisualizer for CoinTransferVisualizer {
    fn visualize_tx_commands(&self, context: &VisualizerContext) -> Option<SignablePayloadField> {
        let Some(SuiCommand::TransferObjects(args, arg)) =
            context.commands().get(context.command_index())
        else {
            return None;
        };

        let coin = get_coin(context.commands(), context.inputs(), args).unwrap_or_default();
        let amount =
            get_coin_amount(context.commands(), context.inputs(), args).unwrap_or_default();
        let receiver = get_receiver(context.inputs(), arg).unwrap_or_default();

        Some(SignablePayloadField::ListLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: "Transfer Command".to_string(),
                label: "Transfer Command".to_string(),
            },
            list_layout: SignablePayloadFieldListLayout {
                fields: vec![
                    create_text_field("Asset Object ID", &coin.to_string()),
                    create_address_field(
                        "From",
                        &context.sender().to_string(),
                        None,
                        None,
                        None,
                        None,
                    ),
                    create_address_field("To", &receiver.to_string(), None, None, None, None),
                    create_amount_field("Amount", &amount.to_string(), &coin.get_label()),
                ],
            },
        })
    }

    fn can_handle(&self, context: &VisualizerContext) -> bool {
        if let Some(command) = context.commands().get(context.command_index()) {
            matches!(command, SuiCommand::TransferObjects(_, _))
        } else {
            false
        }
    }
}

fn get_receiver(inputs: &[SuiCallArg], transfer_arg: &SuiArgument) -> Option<SuiAddress> {
    let receiver_input = inputs.get(parse_numeric_argument(transfer_arg)? as usize)?;

    receiver_input.pure()?.to_sui_address().ok()
}

fn get_coin(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    transfer_args: &[SuiArgument],
) -> Option<CoinObject> {
    let result_index = get_index(transfer_args, Some(0))? as usize;
    let result_command = commands.get(result_index)?;

    match result_command {
        SuiCommand::SplitCoins(input_coin_arg, _) => {
            let coin_arg = inputs.get(parse_numeric_argument(input_coin_arg)? as usize)?;
            coin_arg.object().map(|id| CoinObject::Unknown(id.to_hex()))
        }
        _ => None,
    }
}

fn get_coin_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    transfer_args: &[SuiArgument],
) -> Option<u64> {
    let result_index = get_index(transfer_args, Some(0))? as usize;
    let result_command = commands.get(result_index)?;

    match result_command {
        SuiCommand::SplitCoins(_, input_coin_args) => {
            let amount_arg = inputs.get(get_index(input_coin_args, Some(0))? as usize)?;
            let Ok(MoveValue::U64(decoded_value)) = SuiJsonValue::to_move_value(
                &amount_arg.pure()?.to_json_value(),
                &MoveTypeLayout::U64,
            ) else {
                return None;
            };
            Some(decoded_value)
        }
        _ => None,
    }
}
