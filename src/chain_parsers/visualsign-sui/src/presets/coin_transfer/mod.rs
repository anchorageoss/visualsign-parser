use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::truncate_address;
use crate::utils::{CoinObject, decode_number, parse_numeric_argument, parse_result_command_index};

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

/// Maximum number of `SplitCoins`/`MergeCoins` references that `resolve_object`
/// will follow before bailing out. `command_index` in `SuiArgument::Result` /
/// `SuiArgument::NestedResult` is attacker-controlled, so an unbounded walk
/// can be steered into a cycle (e.g. command 0 references command 1 which
/// references command 0) or an arbitrarily deep chain. Either case would
/// blow the Tokio worker stack (default 2 MiB) and abort the parser process.
/// A fixed budget catches both shapes without needing a visited set.
///
/// The budget counts reference hops (`Result` / `NestedResult` indirections).
/// A chain of exactly `MAX_RESOLVE_OBJECT_DEPTH` reference hops terminating
/// on `GasCoin` or `Input` still resolves; one more hop bails.
const MAX_RESOLVE_OBJECT_DEPTH: usize = 32;

fn resolve_object(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    object_argument: SuiArgument,
) -> Result<CoinObject, VisualSignError> {
    let mut current = object_argument;
    let mut hops_remaining = MAX_RESOLVE_OBJECT_DEPTH;
    loop {
        match current {
            SuiArgument::GasCoin => return Ok(CoinObject::Sui),
            SuiArgument::Input(index) => {
                return match inputs
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
                };
            }
            SuiArgument::Result(command_index)
            | SuiArgument::NestedResult(command_index, _) => {
                if hops_remaining == 0 {
                    return Err(VisualSignError::ValidationError(format!(
                        "Sui coin reference chain exceeded max depth of {MAX_RESOLVE_OBJECT_DEPTH} hops (possible cycle)"
                    )));
                }
                hops_remaining -= 1;
                match commands.get(command_index as usize).ok_or(
                    VisualSignError::MissingData("Result command not found".into()),
                )? {
                    SuiCommand::SplitCoins(coin_type, _)
                    | SuiCommand::MergeCoins(coin_type, _) => {
                        current = *coin_type;
                    }
                    // TODO: extended chain_config to parse return results from transaction like this:
                    // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
                    SuiCommand::MoveCall(_) => {
                        return Ok(CoinObject::UnknownObject("Unknown".into()));
                    }
                    _ => {
                        return Err(TransactionParseError::UnsupportedVersion(
                            "Parsing Sui native transfer expected `SplitCoins` or `MergeCoins`"
                                .into(),
                        )
                        .into());
                    }
                }
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

    // `object_argument` is a `Result(N)`. Use the result-specific helper so the
    // command index is extracted explicitly, never silently coerced via the
    // input-index path.
    let command = commands
        .get(parse_result_command_index(object_argument)? as usize)
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{MAX_RESOLVE_OBJECT_DEPTH, resolve_object};
    use crate::utils::payload_from_b64;

    use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};

    use visualsign::SignablePayloadField;
    use visualsign::errors::VisualSignError;
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

    // Regression tests for unbounded resolve_object recursion: resolve_object used
    // to recurse without a depth bound or cycle check on the attacker-controlled
    // command_index, letting a crafted Sui tx blow the worker stack and abort the parser.

    #[test]
    fn test_resolve_object_self_cycle_returns_error() {
        // commands[0] = SplitCoins(Result(0), []) — self-loop.
        let commands = vec![SuiCommand::SplitCoins(SuiArgument::Result(0), vec![])];
        let inputs: Vec<SuiCallArg> = vec![];

        let err = resolve_object(&commands, &inputs, SuiArgument::Result(0))
            .expect_err("self-cycle must not resolve");

        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }

    #[test]
    fn test_resolve_object_two_command_cycle_returns_error() {
        // commands[0] = SplitCoins(Result(1), []), commands[1] = SplitCoins(Result(0), []).
        // Walking either index loops forever in the unfixed code.
        let commands = vec![
            SuiCommand::SplitCoins(SuiArgument::Result(1), vec![]),
            SuiCommand::SplitCoins(SuiArgument::Result(0), vec![]),
        ];
        let inputs: Vec<SuiCallArg> = vec![];

        let err = resolve_object(&commands, &inputs, SuiArgument::Result(0))
            .expect_err("two-command cycle must not resolve");

        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }

    #[test]
    fn test_resolve_object_nested_result_cycle_returns_error() {
        // Same shape as the self-cycle but via NestedResult, which the recursive
        // branch also followed unconditionally.
        let commands = vec![SuiCommand::SplitCoins(
            SuiArgument::NestedResult(0, 0),
            vec![],
        )];
        let inputs: Vec<SuiCallArg> = vec![];

        let err = resolve_object(&commands, &inputs, SuiArgument::NestedResult(0, 0))
            .expect_err("NestedResult self-cycle must not resolve");

        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }

    #[test]
    fn test_resolve_object_deep_acyclic_chain_exceeds_budget() {
        // Acyclic but longer than the budget: command i points to command i+1
        // for i in 0..N, last one points to GasCoin would terminate. Instead,
        // make the chain longer than MAX_RESOLVE_OBJECT_DEPTH so it must bail.
        let chain_len = MAX_RESOLVE_OBJECT_DEPTH + 5;
        let commands: Vec<SuiCommand> = (0..chain_len)
            .map(|i| {
                // Last command terminates on GasCoin so the chain is acyclic;
                // every other command refers to the next index.
                let next = if i + 1 < chain_len {
                    SuiArgument::Result(u16::try_from(i + 1).unwrap())
                } else {
                    SuiArgument::GasCoin
                };
                SuiCommand::SplitCoins(next, vec![])
            })
            .collect();
        // Sanity: we constructed exactly chain_len commands.
        assert_eq!(commands.len(), chain_len);

        let err = resolve_object(&commands, &[], SuiArgument::Result(0))
            .expect_err("chain longer than budget must not resolve");

        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }

    #[test]
    fn test_resolve_object_short_legal_chain_still_resolves() {
        // Guards against an off-by-one in the bound: a chain of 5 SplitCoins
        // commands terminating on GasCoin must still resolve to Sui.
        let commands: Vec<SuiCommand> = (0..5)
            .map(|i| {
                let next = if i + 1 < 5 {
                    SuiArgument::Result(i + 1)
                } else {
                    SuiArgument::GasCoin
                };
                SuiCommand::SplitCoins(next, vec![])
            })
            .collect();

        let resolved = resolve_object(&commands, &[], SuiArgument::Result(0))
            .expect("short legal chain must resolve");

        assert_eq!(resolved, crate::utils::CoinObject::Sui);
    }

    #[test]
    fn test_resolve_object_chain_at_exact_budget_resolves() {
        // Boundary case: a chain of exactly MAX_RESOLVE_OBJECT_DEPTH reference
        // hops terminating on GasCoin must resolve. The doc-comment advertises
        // this as the maximum resolvable depth; this guards the off-by-one.
        // commands[0..MAX-1] each reference the next index, commands[MAX-1]
        // terminates on GasCoin. Starting from Result(0) that is exactly
        // MAX_RESOLVE_OBJECT_DEPTH reference follows before the terminal.
        let chain_len = MAX_RESOLVE_OBJECT_DEPTH;
        let commands: Vec<SuiCommand> = (0..chain_len)
            .map(|i| {
                let next = if i + 1 < chain_len {
                    SuiArgument::Result(u16::try_from(i + 1).unwrap())
                } else {
                    SuiArgument::GasCoin
                };
                SuiCommand::SplitCoins(next, vec![])
            })
            .collect();

        let resolved = resolve_object(&commands, &[], SuiArgument::Result(0))
            .expect("chain at exact budget must resolve");

        assert_eq!(resolved, crate::utils::CoinObject::Sui);
    }

    #[test]
    fn test_resolve_object_chain_one_past_budget_bails() {
        // Boundary case: one more hop than the budget must bail.
        let chain_len = MAX_RESOLVE_OBJECT_DEPTH + 1;
        let commands: Vec<SuiCommand> = (0..chain_len)
            .map(|i| {
                let next = if i + 1 < chain_len {
                    SuiArgument::Result(u16::try_from(i + 1).unwrap())
                } else {
                    SuiArgument::GasCoin
                };
                SuiCommand::SplitCoins(next, vec![])
            })
            .collect();

        let err = resolve_object(&commands, &[], SuiArgument::Result(0))
            .expect_err("chain one past budget must bail");

        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "expected ValidationError, got {err:?}"
        );
    }
}
