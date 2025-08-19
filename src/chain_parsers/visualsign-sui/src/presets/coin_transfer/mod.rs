use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{CoinObject, get_index, parse_numeric_argument, truncate_address};

use move_core_types::runtime_value::MoveValue;

use sui_json::{MoveTypeLayout, SuiJsonValue};
use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

use sui_types::gas_coin::MIST_PER_SUI;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_address_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::{create_amount_field, create_text_field},
};

pub struct CoinTransferVisualizer;

impl CommandVisualizer for CoinTransferVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let Some(SuiCommand::TransferObjects(args, arg)) =
            context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected to get TransferObjects for coin transfer parsing".into(),
            ));
        };

        let coin = get_coin(context.commands(), context.inputs(), args).unwrap_or_default();
        let amount =
            get_coin_amount(context.commands(), context.inputs(), args).unwrap_or_default();
        let receiver = get_receiver(context.inputs(), arg).unwrap_or_default();

        let title_text = if amount > 0 {
            match &coin {
                CoinObject::Sui => {
                    format!("Transfer: {} MIST ({} SUI)", amount, amount / MIST_PER_SUI)
                }
                CoinObject::Unknown(id) => format!("Transfer: {} {}", amount, id),
            }
        } else {
            "Transfer Command".to_string()
        };

        let subtitle_text = format!(
            "From {} to {}",
            truncate_address(&context.sender().to_string()),
            truncate_address(&receiver.to_string())
        );

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Transfer {} {} from {} to {}",
                    amount,
                    coin.get_label(),
                    truncate_address(&context.sender().to_string()),
                    truncate_address(&receiver.to_string())
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_text_field("Asset Object ID", &coin.to_string())?,
                create_address_field(
                    "From",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_address_field("To", &receiver.to_string(), None, None, None, None)?,
                create_amount_field("Amount", &amount.to_string(), &coin.get_label())?,
            ],
        };

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Transfer Command".to_string(),
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
        })
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        None
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Native Transfer")
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
        SuiCommand::SplitCoins(input_coin_arg, _) => match input_coin_arg {
            SuiArgument::GasCoin => Some(CoinObject::Sui),
            _ => {
                let coin_arg = inputs.get(parse_numeric_argument(input_coin_arg)? as usize)?;
                coin_arg.object().map(|id| CoinObject::Unknown(id.to_hex()))
            }
        },
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

#[cfg(test)]
mod tests {
    use crate::test_utils::{assert_has_field, payload_from_b64};

    #[test]
    fn test_transfer_commands() {
        // https://suivision.xyz/txblock/CE46w3GYgWnZU8HF4P149m6ANGebD22xuNqA64v7JykJ
        let test_data = "AQAAAAAABQEAm9cmP35lHGKppWJLgoYU7aexd43oTT2ci4QzxDXFNv92CAsjAAAAACANp0teIzSyzZ4Pj5dL3YaYBdeVmiWScWL/9RCV4mUINwEAARQFJheK7qwbpqmQudEhsSyQ6AjVawfLpN4XRBhe12FH6TIiAAAAACDXzuT2xanZ36QNQSYtDhZn31zfzIlhRk5H6pTsqGdRDAEAXpykdGz3KJdaAVjyAMZQxufRYJfqzNXfOu8jVCAjEjIzfYIhAAAAACA5hk9rACYb1i5fqrUBJIgXhdUFOqOaouNWmQINCW4/WQAIAPLhNQAAAAAAIEutPmqkZpN81fwdos/haXZAQJoZsX8SvKilyMRxrv/pAwMBAAACAQEAAQIAAgEAAAEBAwABAQIBAAEEAA4x8k3bZAV+p192pmk9h7U2nGDwuTmW8EY6c95JyFHCAaCnde0j6aiVXUd/1gCf3q5Uuj1mPVIuuEpJn1teueghdggLIwAAAAAgNhuP2zGpc0qF3gRzxQC5B0lpAZR7xyssXC3gKbH8uxwOMfJN22QFfqdfdqZpPYe1Npxg8Lk5lvBGOnPeSchRwugDAAAAAAAAoIVIAAAAAAAAAWEAFrlPuI8JOSzIoIBc0xwfWia7T5uPf1PS+aSSphoTTq0lRpNuTOg8eOggpBxpLsQDrbAx3jDoWg1R8hZKR62LBex1R808U6AgiY8V7LxOVsChXFf8nSAEGaeSLQc7mJbx";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Transfer Command");
    }
}
