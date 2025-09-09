mod config;

use config::{Config, NATIVE_STAKING_CONFIG, SuiSystemFunctions};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{decode_number, get_index, parse_numeric_argument, truncate_address};

use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall};
use sui_types::base_types::SuiAddress;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
};

pub struct SuiNativeStakingVisualizer;

impl CommandVisualizer for SuiNativeStakingVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for staking parsing".into(),
            ));
        };

        match pwc.function.as_str().try_into()? {
            SuiSystemFunctions::AddStake => Self::handle_add_stake(context, pwc),
            SuiSystemFunctions::WithdrawStake => Self::handle_withdraw_stake(context, pwc),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(NATIVE_STAKING_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::StakingPools("Sui Native Staking")
    }
}

impl SuiNativeStakingVisualizer {
    fn handle_add_stake(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let amount = get_stake_amount(context.commands(), context.inputs(), &pwc.arguments)
            .unwrap_or_default();
        let receiver = get_stake_receiver(context.inputs(), &pwc.arguments).unwrap_or_default();

        {
            let (title_text, amount_field) = match amount {
                Some(amount) => (
                    format!("Stake: {amount} MIST"),
                    create_amount_field("Amount", &amount.to_string(), "MIST")?,
                ),
                None => (
                    "Stake Command".to_string(),
                    create_text_field("Amount", "N/A MIST")?,
                ),
            };

            let subtitle_text = format!(
                "From {} to validator {}",
                truncate_address(&context.sender().to_string()),
                truncate_address(&receiver.to_string())
            );

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![amount_field.clone()],
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
                    create_address_field(
                        "Validator",
                        &receiver.to_string(),
                        None,
                        None,
                        None,
                        None,
                    )?,
                    amount_field,
                ],
            };

            Ok(vec![AnnotatedPayloadField {
                static_annotation: None,
                dynamic_annotation: None,
                signable_payload_field: SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: title_text.clone(),
                        label: "Stake Command".to_string(),
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

    fn handle_withdraw_stake(
        context: &VisualizerContext,
        _pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let title_text = "Withdraw Stake".to_string();
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_address_field(
                "From",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![create_address_field(
                "From",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?],
        };

        Ok(vec![AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: title_text.clone(),
                    label: "Withdraw Command".to_string(),
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

fn get_stake_receiver(
    inputs: &[SuiCallArg],
    args: &[SuiArgument],
) -> Result<SuiAddress, VisualSignError> {
    let receiver_input = inputs
        .get(get_index(args, Some(args.len() - 1))? as usize)
        .ok_or(VisualSignError::MissingData("Command not found".into()))?;

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

fn get_stake_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    args: &[SuiArgument],
) -> Result<Option<u64>, VisualSignError> {
    let command = commands
        .get(get_index(args, Some(1))? as usize)
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

#[cfg(test)]
mod tests {
    use crate::utils::payload_from_b64;

    use visualsign::test_utils::assert_has_field;

    #[test]
    fn test_stake_commands() {
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "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";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Stake Command");
    }

    #[test]
    fn test_withdraw_commands() {
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "AQAAAAAAAgEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAUBAAAAAAAAAAEBADtuZRRZcXabYn2eLpOPGq3onyss/0Kyuv3BoB3PQPiIJHpFHQAAAAAgDlI1Bti2mpZBb/rDxYkyB+lyANUGRTtYgKbRoBow53cBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCnN1aV9zeXN0ZW0WcmVxdWVzdF93aXRoZHJhd19zdGFrZQACAQAAAQEAPmYGcGNxVi5pj8Tk1ufHEB6SYs6TFjQYj+JG7623BnUCN8ccpwVmcafDNOXvnEAo6kzltjdniobA56to42fHdUio9wcjAAAAACDQVC4fMhsmX6OlHpAhyPR8LaRzgu43Bj8xrhlRY6YKG/Yv6m2ncHpPhbrEkOrSiyh1ID3T4FARE+raMUofCsQPqPcHIwAAAAAg5qp+jjoniUXPNG4N0/9XDFSpoUt0isbEUMiXjNtGivA+ZgZwY3FWLmmPxOTW58cQHpJizpMWNBiP4kbvrbcGdSECAAAAAAAADAqcAAAAAAAAAWEAkj0EN51BkbIUE/6lMi967MHGsBMl2i8TtntUnFhlC2rK8AW2fGQxc8mg1gTbV+2eHs1CsZ9m67cU4CWzA+9PAg//ECUrmzUzzsg0xYRgwDQDy9lAF8e6bpAa8/5Yec6s";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, "Withdraw Command");
    }
}
