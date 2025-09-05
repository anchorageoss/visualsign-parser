use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::truncate_address;
use crate::utils::{CoinObject, decode_number, parse_numeric_argument};

use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand, SuiObjectArg};
use sui_types::base_types::SuiAddress;

use sui_types::gas_coin::MIST_PER_SUI;
use visualsign::errors::{TransactionParseError, VisualSignError};
use visualsign::field_builders::{create_address_field, create_amount_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub struct CoinTransferVisualizer;

impl CommandVisualizer for CoinTransferVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::TransferObjects(objects_to_send, receiver_argument)) =
            context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected `TransferObjects` for coin transfer parsing".into(),
            ));
        };

        let receiver = resolve_receiver(context.inputs(), *receiver_argument)?;
        let objects_sent_to_receiver = objects_to_send
            .iter()
            .map(|object| resolve_object(context.commands(), context.inputs(), *object))
            .collect::<Result<Vec<CoinObject>, VisualSignError>>()?;

        objects_to_send
            .iter()
            .enumerate()
            .map(|(object_index, object_argument)| visualize_transfer_command(context, receiver, objects_sent_to_receiver.get(object_index).expect("Object to exist as objects_sent_to_receiver should be the same length as objects_to_send"), *object_argument))
            .collect::<_>()
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

fn resolve_receiver(
    inputs: &[SuiCallArg],
    receiver_argument: SuiArgument,
) -> Result<SuiAddress, VisualSignError> {
    let receiver_input = inputs
        .get(parse_numeric_argument(receiver_argument)? as usize)
        .ok_or(VisualSignError::MissingData(
            "Receiver input not found".into(),
        ))?;

    match receiver_input
        .pure()
        .ok_or(VisualSignError::MissingData(
            "Receiver input not found".into(),
        ))?
        .to_sui_address()
    {
        Ok(address) => Ok(address),
        Err(e) => Err(VisualSignError::ConversionError(e.to_string())),
    }
}

fn resolve_object(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    object_argument: SuiArgument,
) -> Result<CoinObject, VisualSignError> {
    match object_argument {
        SuiArgument::GasCoin => Ok(CoinObject::Sui),
        SuiArgument::Input(index) => {
            match inputs
                .get(index as usize)
                .ok_or(VisualSignError::MissingData("Input not found".into()))?
            {
                SuiCallArg::Object(e) => match e {
                    SuiObjectArg::ImmOrOwnedObject { object_id, .. }
                    | SuiObjectArg::SharedObject { object_id, .. }
                    | SuiObjectArg::Receiving { object_id, .. } => {
                        Ok(CoinObject::UnknownObject(object_id.to_hex()))
                    }
                },
                SuiCallArg::Pure(_) => Err(TransactionParseError::UnsupportedVersion(
                    "Parsing Sui native transfer input expected `Object`".into(),
                )
                .into()),
            }
        }
        SuiArgument::Result(command_index) | SuiArgument::NestedResult(command_index, _) => {
            match commands
                .get(command_index as usize)
                .ok_or(VisualSignError::MissingData(
                    "Result command not found".into(),
                ))? {
                SuiCommand::SplitCoins(coin_type, _) | SuiCommand::MergeCoins(coin_type, _) => {
                    resolve_object(commands, inputs, *coin_type)
                }
                // TODO: extended chain_config to parse return results from transaction like this:
                // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
                SuiCommand::MoveCall(_) => Ok(CoinObject::UnknownObject("Unknown".into())),
                _ => Err(TransactionParseError::UnsupportedVersion(
                    "Parsing Sui native transfer expected `SplitCoins` or `MergeCoins`".into(),
                )
                .into()),
            }
        }
    }
}

fn resolve_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    object_argument: SuiArgument,
) -> Result<Option<u64>, VisualSignError> {
    let SuiArgument::Result(_) = object_argument else {
        return Ok(None);
    };

    let command = commands
        .get(parse_numeric_argument(object_argument)? as usize)
        .ok_or(VisualSignError::MissingData("Command not found".into()))?;

    match command {
        SuiCommand::SplitCoins(_, input_coin_args) if input_coin_args.len() == 1 => {
            let amount_arg = inputs
                .get(parse_numeric_argument(input_coin_args[0])? as usize)
                .ok_or(VisualSignError::MissingData(
                    "Amount argument not found".into(),
                ))?;

            Ok(Some(decode_number::<u64>(amount_arg)?))
        }
        _ => Ok(None),
    }
}

fn visualize_transfer_command(
    context: &VisualizerContext,
    receiver: SuiAddress,
    object_sent_to_receiver: &CoinObject,
    object_argument: SuiArgument,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let amount = resolve_amount(context.commands(), context.inputs(), object_argument)?;

    let (amount_str, title_text, amount_field) = match amount {
        Some(amount) => {
            let title_text = match object_sent_to_receiver {
                CoinObject::Sui => {
                    format!("Transfer: {} MIST ({} SUI)", amount, amount / MIST_PER_SUI)
                }
                CoinObject::UnknownObject(id) => format!("Transfer: {amount} {id}"),
            };

            (
                format!("{amount} MIST"),
                title_text,
                create_amount_field(
                    "Amount",
                    &amount.to_string(),
                    &object_sent_to_receiver.get_label(),
                )?,
            )
        }
        None => (
            "N/A MIST".to_string(),
            "Transfer Command".to_string(),
            create_text_field("Amount", "N/A MIST")?,
        ),
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
                "Transfer {} from {} to {}",
                amount_str,
                truncate_address(&context.sender().to_string()),
                truncate_address(&receiver.to_string())
            ),
        )?],
    };

    let expanded = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Asset Object ID", &object_sent_to_receiver.to_string())?,
            create_address_field(
                "From",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_address_field("To", &receiver.to_string(), None, None, None, None)?,
            amount_field,
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

#[cfg(test)]
mod tests {
    use crate::utils::payload_from_b64;

    use visualsign::SignablePayloadField;
    use visualsign::test_utils::assert_has_field;

    #[test]
    fn test_transfer_commands() {
        // https://suivision.xyz/txblock/CE46w3GYgWnZU8HF4P149m6ANGebD22xuNqA64v7JykJ
        let test_data = "AQAAAAAABQEAm9cmP35lHGKppWJLgoYU7aexd43oTT2ci4QzxDXFNv92CAsjAAAAACANp0teIzSyzZ4Pj5dL3YaYBdeVmiWScWL/9RCV4mUINwEAARQFJheK7qwbpqmQudEhsSyQ6AjVawfLpN4XRBhe12FH6TIiAAAAACDXzuT2xanZ36QNQSYtDhZn31zfzIlhRk5H6pTsqGdRDAEAXpykdGz3KJdaAVjyAMZQxufRYJfqzNXfOu8jVCAjEjIzfYIhAAAAACA5hk9rACYb1i5fqrUBJIgXhdUFOqOaouNWmQINCW4/WQAIAPLhNQAAAAAAIEutPmqkZpN81fwdos/haXZAQJoZsX8SvKilyMRxrv/pAwMBAAACAQEAAQIAAgEAAAEBAwABAQIBAAEEAA4x8k3bZAV+p192pmk9h7U2nGDwuTmW8EY6c95JyFHCAaCnde0j6aiVXUd/1gCf3q5Uuj1mPVIuuEpJn1teueghdggLIwAAAAAgNhuP2zGpc0qF3gRzxQC5B0lpAZR7xyssXC3gKbH8uxwOMfJN22QFfqdfdqZpPYe1Npxg8Lk5lvBGOnPeSchRwugDAAAAAAAAoIVIAAAAAAAAAWEAFrlPuI8JOSzIoIBc0xwfWia7T5uPf1PS+aSSphoTTq0lRpNuTOg8eOggpBxpLsQDrbAx3jDoWg1R8hZKR62LBex1R808U6AgiY8V7LxOVsChXFf8nSAEGaeSLQc7mJbx";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Transfer Command");
    }

    #[test]
    fn test_transfer_commands_single_transfer() {
        // https://suivision.xyz/txblock/5S2D1qNww9QXDLgCzEr2UReFas7PGFRxdyjCJrYfjUxd
        let test_data = "AQAAAAAAAwEAVCf5McvToD8qhL+h2/xg7M2l287m7+8IGIpLQ/6cxBYgqxwkAAAAACDT4HJIX5m7UeyrSJQAHz+p5ZniCwngoTE8GX8E6Vu8HgAI5AYAAAAAAAAAIPEcpBpzFvUgalVqqqnn/Y6mrsto2zVvr1FpVbQvZUfiAgIBAAABAQEAAQECAAABAgCCWsIch38qw9SMwyrvCbO4KfA+TwtC/MZ6NYYnVJq0nAFzUrDKacSVDVVrzYCDnNWtV6Of8JseRtaWdzmHWx/eACCrHCQAAAAAIHZmDcOF5ICx52aJBITeT+GXuGbiP1LOdMK9ewrTvoU3glrCHId/KsPUjMMq7wmzuCnwPk8LQvzGejWGJ1SatJz0AQAAAAAAAOi2MgAAAAAAAAFhALPjB1b3CwKNTPZTHUWogbc9Wz5fgXzVTh1I0dhWVAPGoWxP8HzKFAKr7pZSF/eF1ls/V+m8by7W62K4GbDHLAbJHKJuw6P/F6xoTvR/p7PpYvz2kjD0Z+S3PwARYTCtiw==";

        let payload = payload_from_b64(test_data);

        let transfer_command: Option<&SignablePayloadField> = payload
            .fields
            .iter()
            .find(|f| f.label() == "Transfer Command");

        match transfer_command {
            Some(SignablePayloadField::PreviewLayout { preview_layout, .. }) => {
                assert_eq!(
                    preview_layout.expanded.clone().unwrap().fields[0]
                        .signable_payload_field
                        .fallback_text(),
                    "Object ID: 5427f931cbd3a03f2a84bfa1dbfc60eccda5dbcee6efef08188a4b43fe9cc416"
                );

                assert_eq!(
                    preview_layout.expanded.clone().unwrap().fields[1]
                        .signable_payload_field
                        .fallback_text(),
                    "0x825ac21c877f2ac3d48cc32aef09b3b829f03e4f0b42fcc67a358627549ab49c"
                );

                assert_eq!(
                    preview_layout.expanded.clone().unwrap().fields[2]
                        .signable_payload_field
                        .fallback_text(),
                    "0xf11ca41a7316f5206a556aaaa9e7fd8ea6aecb68db356faf516955b42f6547e2"
                );

                assert_eq!(
                    preview_layout.expanded.clone().unwrap().fields[3]
                        .signable_payload_field
                        .fallback_text(),
                    "1764 Unknown"
                );
            }
            _ => panic!("Expected a PreviewLayout for Transfer Command"),
        }
    }

    #[test]
    fn test_transfer_commands_multiple_transfers() {
        // https://suivision.xyz/txblock/CE46w3GYgWnZU8HF4P149m6ANGebD22xuNqA64v7JykJ
        let test_data = "AQAAAAAABwEA6Y5kz7fNxZOj6yZRvcQBtXykVIWvnEy6HQ4kpt8rkstEKr0jAAAAACDGqhNnbuG6uY3kzKZ3wji82QjhFjSBp/RhJBmLCmrq9wEANaDAaF3wfproXnOA3DQll20l0sKpI5/pwi7PgTQo2O1EKr0jAAAAACCV7ngIWPDBl8jcZ4ROZMvFsFd+l/bDIgRa5MnJ1U+O8QEAQZqheNp0/QSrywtlsaKcaLpNlWhnDe/rJLnDSL7EdwlJKr0jAAAAACCiuEQMpyzDWUeQLji6IZh2ZVQsJ3bfV9ohbFikWWK/SgEA/yIhccvTw3DNOX0eBnjuJoOlj0wVJnJUPrybRXWIqQNJKr0jAAAAACBS2RDiolpleMxI7YixmXfd0yg8qyjxDdU9AEmpbbEldQAINhwBAwAAAAAAIFyZUAMWUtWCEIOgr0t+NfSLnuhKon4e/foY3MuJlRuaACD8MuPzTp6pDb/8zoOsfdjhmlRpIVq8iqVCQI3qEAcc9AUDAQEAAQEAAAMBAwABAQIAAgABAQQAAQECAgABBQABAwEDAAEBAAABBgBcmVADFlLVghCDoK9LfjX0i57oSqJ+Hv36GNzLiZUbmgODyYnEoQPANAx0dAuxgpZm+6xO4Pe04Z0g0nm0ZewBsEkqvSMAAAAAIHzjp+LIH7ug3H6/wkA/rj8JYefB3x+6gBLpcCd8eSH5YU0cMRSH6QQ2aSXkllPWCW1/QjVC4OwdAmbY+9A8IXBJKr0jAAAAACAGZnIszNBh5u8vrd/vbQoGHT5HS/VtJSZSQjrBwjTvRR3Kxvc/VyIEFiN2ja7agdhYhyERH/driCiKwDVDXkX/SSq9IwAAAAAg4xzfi5cOl6aSFOyMzS9/o9mQYsVgLpDDjT8YYmoILEtcmVADFlLVghCDoK9LfjX0i57oSqJ+Hv36GNzLiZUbmiECAAAAAAAA0KEQAAAAAAAAAWEAzzUYs9lUE87bOysJcBeWH69UoPgvOH5rHStsap6apWhkAMoUnJuM+XoCT3rDH+BUdxw5Skoqdk1VEYCm13k8Bm+W3QJTREXUZtaOs+eopm2qifmjn1oezf2q05W79+rJ";
        let payload = payload_from_b64(test_data);

        let transfer_commands: Vec<&SignablePayloadField> = payload
            .fields
            .iter()
            .filter(|f| f.label() == "Transfer Command")
            .collect();

        assert_eq!(
            transfer_commands.len(),
            4,
            "Should have four Transfer Command fields"
        );
    }

    #[test]
    #[ignore]
    fn test_transfer_commands_with_move_calls_first() {
        // https://suivision.xyz/txblock/z6Y1KqnTGx3pTsjotbV4PHqHMJyigyXieT8qsSAGFLY
        let test_data = "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";
        let payload = payload_from_b64(test_data);

        assert_has_field(&payload, "Transfer Command");
    }

    #[test]
    fn test_transfer_commands_with_move_calls_second() {
        // https://suivision.xyz/txblock/FMrjghwv2xhaAZ5dYpsZgwwo4EGUr6dt28X9RbyADRTY
        let test_data = "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";
        let payload = payload_from_b64(test_data);

        assert_has_field(&payload, "Transfer Command");
    }
}
