mod config;

use config::{CETUS_CONFIG, PoolScriptV2Functions, SwapA2BIndexes, SwapB2AIndexes};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use crate::presets::cetus::config::Config;
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
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Cetus parsing".into(),
            ));
        };

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(e) => return Err(VisualSignError::DecodeError(e)),
        };

        match function {
            PoolScriptV2Functions::SwapB2A => Ok(self.handle_swap_b2a(context, pwc)?),
            PoolScriptV2Functions::SwapA2B => Ok(self.handle_swap_a2b(context, pwc)?),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(CETUS_CONFIG.get_or_init(Config::new))
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
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let input_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();
        let output_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();

        let input_amount = SwapB2AIndexes::get_amount_in(context.inputs(), &pwc.arguments)?;
        let min_output_amount =
            SwapB2AIndexes::get_min_amount_out(context.inputs(), &pwc.arguments)?;

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_amount_field(
                "Input Amount",
                &input_amount.to_string(),
                input_coin.symbol(),
            )?,
            create_text_field("Input Coin", &input_coin.to_string())?,
            create_amount_field(
                "Min Output Amount",
                &min_output_amount.to_string(),
                output_coin.symbol(),
            )?,
            create_text_field("Output Coin", &output_coin.to_string())?,
        ];

        {
            let title_text = format!(
                "CetusAMM Swap: {} {} → {}",
                input_amount,
                input_coin.symbol(),
                output_coin.symbol()
            );
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!(
                        "Swap {} to {} (min out: {})",
                        input_coin.symbol(),
                        output_coin.symbol(),
                        min_output_amount
                    ),
                )?],
            };

            let expanded = SignablePayloadFieldListLayout {
                fields: list_layout_fields,
            };

            Ok(vec![AnnotatedPayloadField {
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
            }])
        }
    }

    fn handle_swap_a2b(
        &self,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let input_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let output_coin: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let max_input_amount = SwapA2BIndexes::get_max_amount_in(context.inputs(), &pwc.arguments)?;
        let amount_out = SwapA2BIndexes::get_amount_out(context.inputs(), &pwc.arguments)?;

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_amount_field(
                "Max Input Amount",
                &max_input_amount.to_string(),
                input_coin.symbol(),
            )?,
            create_text_field("Input Coin", &input_coin.to_string())?,
            create_amount_field("Amount Out", &amount_out.to_string(), output_coin.symbol())?,
            create_text_field("Output Coin", &output_coin.to_string())?,
        ];

        let title_text = format!(
            "CetusAMM Swap: {} {} → {}",
            max_input_amount,
            input_coin.symbol(),
            output_coin.symbol()
        );
        let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Swap {} to {} (max in: {})",
                    input_coin.symbol(),
                    output_coin.symbol(),
                    max_input_amount
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: list_layout_fields,
        };

        Ok(vec![AnnotatedPayloadField {
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
        }])
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::payload_from_b64;

    use visualsign::test_utils::{assert_has_field, assert_has_field_with_value};

    const CETUS_SWAP_LABEL: &str = "CetusAMM Swap Command";

    #[test]
    fn test_cetus_amm_swap_b2a_commands() {
        // https://suivision.xyz/txblock/7Je4yeXMvvEHFcRSTD4WYv3eSsaDk2zqvdoSxWXdUYGx
        let test_data = "AQAAAAAACQEAEXs/ewhS1RZrUZQ2xQEliCJn40SK4PvEV75r2SGFMXhjUsAjAAAAACBSKqlrLdPXYeuzckz31NAkeSO09qmNPv/pkWggJMTC2QAIuMbAAQAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFK94o+ni1sq8pdp5wea/9ImVZqQhMh/DtaYZZkAXpg1nkOqBoAAAAAAQABAQAIuMbAAQAAAAAACI0+GgMAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAQAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB7eETiiahBDlD7PKSNaeuc8p4n0iPvkDU/4b2OJ/+PP4BGNvaW4EQ09JTgAJAQIAAQMAAgEAAgAAAQQAAQUAAQYAAQcAAQgArltnUkfA5IdctLm9N6YO1bz4kng0TThA3StCbiinZoUBZI8YcdbCiGOtIFCZV/M9U6lZTgf3lg6t7feHRsBBqR1jUsAjAAAAACCmwR6aeqn8D632smpzU9fbDhP3vPOQhgc806IrzekPH65bZ1JHwOSHXLS5vTemDtW8+JJ4NE04QN0rQm4op2aFBQIAAAAAAAC8YDQAAAAAAAABYQAdbFpPHuOPe/TYRMttj4FSzAN1ErZdI75GooTkFmiIVkvCM+lnSS3pR/qQt6j7K3gsrtBExfgOL/dffWapvuMEyeP1ig9kZWEaY4lMw99QxRTo2PcUhKsb1gquOOAGXP8=";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0xae5b675247c0e4875cb4b9bd37a60ed5bcf89278344d3840dd2b426e28a76685",
        );
        assert_has_field_with_value(&payload, "Input Amount", "29411000");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xb7844e289a8410e50fb3ca48d69eb9cf29e27d223ef90353fe1bd8e27ff8f3f8::coin::COIN",
        );
        assert_has_field_with_value(&payload, "Min Output Amount", "52051597");
        assert_has_field_with_value(
            &payload,
            "Output Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
    }

    #[test]
    fn test_cetus_amm_swap_a2b_commands() {
        // https://suivision.xyz/txblock/7t6iLtevYDEpXrr3rhpmDcwf8cMMV1sgspppvvnXiguR
        let test_data = "AQAAAAAACgEAkfGWz0JGPLt14gdQVPgAPvGv100NtFt2InDcGDyMZQRIPXMkAAAAACBzf79a+nciTqmPBgQycQyP7VMyWjP2waulu8LKtlZ2ggEA4pyQKsylAKpoN702neQpT4smpbXaopiWRMOhnNQk+dxIPXMkAAAAACDPA2LUvkZAkhsDL9IAPA5XEMTFk44RZFMN/UrpVT0aOwAIqBEHZwAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFR6IO6fAtWaibLyKlM0z6wq9QYp3zB5grSL9mx8pzSq/uacRYAAAAAAQABAAAIAIhSanQAAAAACKgRB2cAAAAAABBQOwEAAQAAAAAAAAAAAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAQDAQAAAQEBAAIBAAABAQIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBGNvaW4EemVybwEHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYTJiAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACA3N1aQNTVUkACQEDAAEEAAIBAAICAAEFAAEGAAEHAAEIAAEJABxoihUeyy/EpwFkjSZ1URYLuz/mqwyC1srAaO1syYLJAoo43JFE0wNBKCG9zE6pVWaENgEg5OfSW8ZgI9xfwZ0JSD1zJAAAAAAgHKrS7Xzyr+wSIwY1SfiwUh3kR/gsbnB5wy14YgB8JlUweXl6kiml0me3PakkjYuFIPJ+CJMElcVq6NGPtGy26Ug9cyQAAAAAINXWx8S5GTFIRWp1oY/IAkEhRVrywXZhCYVXXzVPcQ7fHGiKFR7LL8SnAWSNJnVRFgu7P+arDILWysBo7WzJgsn0AQAAAAAAAOhyLwAAAAAAAAFhAIB1hvQj0FnB2h+j3lZjYL1en1K3A7ITWXhVpj1Oslz0FVgkC3Es9xS5JGDgXByYelNgSJ4bFzB+Sn+9LwOJVAKAiyGXhnmh12WYynVXlQH2doDZ0v5LCrXENXPauhOQWQ==";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0x1c688a151ecb2fc4a701648d267551160bbb3fe6ab0c82d6cac068ed6cc982c9",
        );
        assert_has_field_with_value(&payload, "Max Input Amount", "1728516520");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
        assert_has_field_with_value(&payload, "Amount Out", "500000000000");
        assert_has_field_with_value(&payload, "Output Coin", "0x2::sui::SUI");
    }

    #[test]
    fn test_cetus_amm_swap_a2b_commands_second_tx() {
        // https://suivision.xyz/txblock/HAHk4BVvAFNKVneS6P8k2vhFycqADUE3K5xC985yyuRK
        let test_data = "AQAAAAAACgEAw+ZNZCvOxq7J3o65MNV5y/ZOmrBuxzrjiMZ39jYCgV6gPXMkAAAAACDHOpPg4dhKc0PTcsK5jNrLZnlELVawzYRr/NquL8w1OQEAKKcVN5CJ76uLkF+rV24M6g8C8ipZxat8MUHwXYOknwygPXMkAAAAACA9av6Nm8LlBB3jj1AlGskZi5GDHx86PnP7Is5qtsuu+gAIUJg+CwAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQE7E6xwAw1YdiTkB7vnkRYLRZxI8QSeBCaeuO5zH1RCtCOcRhYAAAAAAQABAAAIACBKqdEBAAAACFCYPgsAAAAAABBQOwEAAQAAAAAAAAAAAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAQDAQAAAQEBAAIBAAABAQIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBGNvaW4EemVybwEHBoZKb5IYBIYJMNtt2+Lhas34UESV6nSBY3oci5qP5UsFY2V0dXMFQ0VUVVMAAACy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92Mghzd2FwX2EyYgIH26NGcuMMsGWx+T46tVMYdo/W/vZsFZQsn3y4RuL5AOcEdXNkYwRVU0RDAAcGhkpvkhgEhgkw223b4uFqzfhQRJXqdIFjehyLmo/lSwVjZXR1cwVDRVRVUwAJAQMAAQQAAgEAAgIAAQUAAQYAAQcAAQgAAQkAfuQPMdt580iiv08Lf3VkXC5PT8Kp+Sqr9F58z4HzNhMBUF2Leac3CRveYxCEsmc6glCbzKyQJD2tzbe2ifztGs+gPXMkAAAAACBBHD5oTyAs9hne3HC6hkK8UDg2rxub87pwBwWVh4LjYH7kDzHbefNIor9PC391ZFwuT0/Cqfkqq/RefM+B8zYT9AEAAAAAAAAQSjQAAAAAAAABYQArHjvGJ4BPm76w6zZhJJFJG48kKRKMWvVjVqvCCLM34nNS21hRNndWp+BXsXKmr02xGcFFu49rXHKD+nBUTvsJxIolJy4E6xigemDv5pKmQcbCxx/3y77AMfNkDNRF8fY=";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0x7ee40f31db79f348a2bf4f0b7f75645c2e4f4fc2a9f92aabf45e7ccf81f33613",
        );
        assert_has_field_with_value(&payload, "Max Input Amount", "188651600");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
        assert_has_field_with_value(&payload, "Amount Out", "2000000000000");
        assert_has_field_with_value(
            &payload,
            "Output Coin",
            "0x6864a6f921804860930db6ddbe2e16acdf8504495ea7481637a1c8b9a8fe54b::cetus::CETUS",
        );
    }

    // https://suivision.xyz/txblock/5GD7JBnjTZDqspScsY2SzY3iy1LKUBJBp7y3NzVnfVdP => collect reward, remove liquidity, close position
    // https://suivision.xyz/txblock/46xdnvVfcCwW5FVEFJd9CyvJgUN2E2ajiqwrL9GtfBxP => open_position_with_liquidity_by_fix_coin
    // https://suivision.xyz/txblock/Bk4uGiLBvgffAm1xbMouH8DGHEJhfhxSgbti813oWAVh => b2a
    // https://suivision.xyz/txblock/2egauw5nHEaFxVjF77a6JC6ZPoUWEK9VrM7UUroFSQkj => collect fee, collect reward, close position
    // https://suivision.xyz/txblock/HzxMo7djbQkec2rauZkWAM553HeXKY6xmPqYBZ2r1MAG => b2a
}
