mod config;

use config::{CETUS_CONFIG, PoolScriptV2Functions, SwapB2AIndexes};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext};
use crate::utils::{SuiCoin, create_address_field, get_tx_type_arg};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

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

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(_) => return None,
        };

        match function {
            PoolScriptV2Functions::SwapB2A => Some(self.handle_swap_b2a(context, pwc)),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(&*CETUS_CONFIG)
    }
}

impl CetusVisualizer {
    fn handle_swap_b2a(
        &self,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> SignablePayloadField {
        let input_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let output_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

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
            ),
            create_address_field("To", &context.sender().to_string(), None, None, None, None),
        ];

        list_layout_fields.push(match input_amount {
            Some(amount) => {
                create_amount_field("Input Amount", &amount.to_string(), input_coin.symbol())
            }
            None => create_text_field("Input Amount", "N/A"),
        });

        list_layout_fields.push(create_text_field("Input Coin", input_coin.symbol()));

        list_layout_fields.push(match min_output_amount {
            Some(amount) => create_amount_field(
                "Min Output Amount",
                &amount.to_string(),
                output_coin.symbol(),
            ),
            None => create_text_field("Min Output Amount", "N/A"),
        });

        list_layout_fields.push(create_text_field("Output Coin", output_coin.symbol()));

        SignablePayloadField::ListLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: "CetusAMM Swap Command".to_string(),
                label: "CetusAMM Swap Command".to_string(),
            },
            list_layout: SignablePayloadFieldListLayout {
                fields: list_layout_fields,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::transaction_string_to_visual_sign;
    use visualsign::vsptrait::VisualSignOptions;

    const CETUS_SWAP_LABEL: &str = "CetusAMM Swap Command";

    #[test]
    fn test_cetus_amm_swap_b2a_commands() {
        // https://suivision.xyz/txblock/7Je4yeXMvvEHFcRSTD4WYv3eSsaDk2zqvdoSxWXdUYGx
        let test_data = "AQAAAAAACQEAEXs/ewhS1RZrUZQ2xQEliCJn40SK4PvEV75r2SGFMXhjUsAjAAAAACBSKqlrLdPXYeuzckz31NAkeSO09qmNPv/pkWggJMTC2QAIuMbAAQAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFK94o+ni1sq8pdp5wea/9ImVZqQhMh/DtaYZZkAXpg1nkOqBoAAAAAAQABAQAIuMbAAQAAAAAACI0+GgMAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAQAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB7eETiiahBDlD7PKSNaeuc8p4n0iPvkDU/4b2OJ/+PP4BGNvaW4EQ09JTgAJAQIAAQMAAgEAAgAAAQQAAQUAAQYAAQcAAQgArltnUkfA5IdctLm9N6YO1bz4kng0TThA3StCbiinZoUBZI8YcdbCiGOtIFCZV/M9U6lZTgf3lg6t7feHRsBBqR1jUsAjAAAAACCmwR6aeqn8D632smpzU9fbDhP3vPOQhgc806IrzekPH65bZ1JHwOSHXLS5vTemDtW8+JJ4NE04QN0rQm4op2aFBQIAAAAAAAC8YDQAAAAAAAABYQAdbFpPHuOPe/TYRMttj4FSzAN1ErZdI75GooTkFmiIVkvCM+lnSS3pR/qQt6j7K3gsrtBExfgOL/dffWapvuMEyeP1ig9kZWEaY4lMw99QxRTo2PcUhKsb1gquOOAGXP8=";

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
            .find(|f| f.label() == CETUS_SWAP_LABEL)
            .expect("Should have a CetusAMM Swap Command field");
    }
}
