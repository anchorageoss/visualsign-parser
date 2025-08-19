mod config;

use config::{CETUS_CONFIG, PoolScriptV2Functions, SwapB2AIndexes};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_address_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::{create_amount_field, create_text_field},
};

pub struct CetusVisualizer;

impl CommandVisualizer for CetusVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected to get MoveCall for Cetus parsing".into(),
            ));
        };

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(e) => return Err(VisualSignError::DecodeError(e)),
        };

        match function {
            PoolScriptV2Functions::SwapB2A => Ok(self.handle_swap_b2a(context, pwc)?),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(&*CETUS_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Cetus")
    }
}

impl CetusVisualizer {
    fn handle_swap_b2a(
        &self,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let input_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let output_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

        let input_amount = SwapB2AIndexes::get_input_amount(context.inputs(), &pwc.arguments);
        let min_output_amount =
            SwapB2AIndexes::get_min_output_amount(context.inputs(), &pwc.arguments);

        let mut list_layout_fields = vec![
            create_address_field(
                "From",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_address_field("To", &context.sender().to_string(), None, None, None, None)?,
        ];

        list_layout_fields.push(match input_amount {
            Some(amount) => {
                create_amount_field("Input Amount", &amount.to_string(), input_coin.symbol())?
            }
            None => create_text_field("Input Amount", "N/A")?,
        });

        list_layout_fields.push(create_text_field("Input Coin", &input_coin.to_string())?);

        list_layout_fields.push(match min_output_amount {
            Some(amount) => create_amount_field(
                "Min Output Amount",
                &amount.to_string(),
                output_coin.symbol(),
            )?,
            None => create_text_field("Min Output Amount", "N/A")?,
        });

        list_layout_fields.push(create_text_field("Output Coin", &output_coin.to_string())?);

        {
            let title_text = match input_amount {
                Some(amount) => format!(
                    "CetusAMM Swap: {} {} → {}",
                    amount,
                    input_coin.symbol(),
                    output_coin.symbol()
                ),
                None => format!(
                    "CetusAMM Swap: {} → {}",
                    input_coin.symbol(),
                    output_coin.symbol()
                ),
            };
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!(
                        "Swap {} to {} (min out: {})",
                        input_coin.symbol(),
                        output_coin.symbol(),
                        min_output_amount
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "N/A".to_string())
                    ),
                )?],
            };

            let expanded = SignablePayloadFieldListLayout {
                fields: list_layout_fields,
            };

            Ok(AnnotatedPayloadField {
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
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::{assert_has_field, payload_from_b64};

    const CETUS_SWAP_LABEL: &str = "CetusAMM Swap Command";

    #[test]
    fn test_cetus_amm_swap_b2a_commands() {
        // https://suivision.xyz/txblock/7Je4yeXMvvEHFcRSTD4WYv3eSsaDk2zqvdoSxWXdUYGx
        let test_data = "AQAAAAAACQEAEXs/ewhS1RZrUZQ2xQEliCJn40SK4PvEV75r2SGFMXhjUsAjAAAAACBSKqlrLdPXYeuzckz31NAkeSO09qmNPv/pkWggJMTC2QAIuMbAAQAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFK94o+ni1sq8pdp5wea/9ImVZqQhMh/DtaYZZkAXpg1nkOqBoAAAAAAQABAQAIuMbAAQAAAAAACI0+GgMAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAQAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB7eETiiahBDlD7PKSNaeuc8p4n0iPvkDU/4b2OJ/+PP4BGNvaW4EQ09JTgAJAQIAAQMAAgEAAgAAAQQAAQUAAQYAAQcAAQgArltnUkfA5IdctLm9N6YO1bz4kng0TThA3StCbiinZoUBZI8YcdbCiGOtIFCZV/M9U6lZTgf3lg6t7feHRsBBqR1jUsAjAAAAACCmwR6aeqn8D632smpzU9fbDhP3vPOQhgc806IrzekPH65bZ1JHwOSHXLS5vTemDtW8+JJ4NE04QN0rQm4op2aFBQIAAAAAAAC8YDQAAAAAAAABYQAdbFpPHuOPe/TYRMttj4FSzAN1ErZdI75GooTkFmiIVkvCM+lnSS3pR/qQt6j7K3gsrtBExfgOL/dffWapvuMEyeP1ig9kZWEaY4lMw99QxRTo2PcUhKsb1gquOOAGXP8=";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);
    }
}
