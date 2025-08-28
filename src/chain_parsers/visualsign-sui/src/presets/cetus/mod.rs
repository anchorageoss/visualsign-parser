mod config;

use config::{CETUS_CONFIG, Config, PoolScriptV2Functions, SwapA2BIndexes, SwapB2AIndexes};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
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
            PoolScriptV2Functions::SwapB2A => self.handle_swap(false, context, pwc),
            PoolScriptV2Functions::SwapA2B => self.handle_swap(true, context, pwc),
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
    fn handle_swap(
        &self,
        is_a2b: bool,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (input_coin, output_coin): (SuiCoin, SuiCoin) = if is_a2b {
            (
                get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default(),
                get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default(),
            )
        } else {
            (
                get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default(),
                get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default(),
            )
        };

        let (by_amount_in, amount, amount_limit) = if is_a2b {
            (
                SwapA2BIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapA2BIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapA2BIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        } else {
            (
                SwapB2AIndexes::get_by_amount_in(context.inputs(), &pwc.arguments)?,
                SwapB2AIndexes::get_amount(context.inputs(), &pwc.arguments)?,
                SwapB2AIndexes::get_amount_limit(context.inputs(), &pwc.arguments)?,
            )
        };

        let (primary_label, primary_symbol, limit_label, limit_symbol) = if by_amount_in {
            (
                "Amount In",
                input_coin.symbol(),
                "Min Out",
                output_coin.symbol(),
            )
        } else {
            (
                "Amount Out",
                output_coin.symbol(),
                "Max In",
                input_coin.symbol(),
            )
        };

        let list_layout_fields = vec![
            create_address_field(
                "User Address",
                &context.sender().to_string(),
                None,
                None,
                None,
                None,
            )?,
            create_amount_field(primary_label, &amount.to_string(), primary_symbol)?,
            create_text_field("Input Coin", &input_coin.to_string())?,
            create_amount_field(limit_label, &amount_limit.to_string(), limit_symbol)?,
            create_text_field("Output Coin", &output_coin.to_string())?,
        ];

        {
            let title_text = if by_amount_in {
                format!(
                    "CetusAMM Swap: {} {} → {}",
                    amount,
                    input_coin.symbol(),
                    output_coin.symbol()
                )
            } else {
                format!(
                    "CetusAMM Swap: {} {} ← {}",
                    amount,
                    output_coin.symbol(),
                    input_coin.symbol()
                )
            };
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!(
                        "Swap {} to {} ({}: {})",
                        input_coin.symbol(),
                        output_coin.symbol(),
                        limit_label,
                        amount_limit
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
        assert_has_field_with_value(&payload, "Amount In", "29411000");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xb7844e289a8410e50fb3ca48d69eb9cf29e27d223ef90353fe1bd8e27ff8f3f8::coin::COIN",
        );
        assert_has_field_with_value(&payload, "Min Out", "52051597");
        assert_has_field_with_value(
            &payload,
            "Output Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
    }

    #[test]
    fn test_cetus_amm_swap_b2a_commands_second_tx() {
        // https://suivision.xyz/txblock/e9Re5Wyn9DxKDhBvGdm4B33kWQ5vy3WxTYV2VAiAZkY
        let test_data = "AQAAAAAABwAIgJaYAAAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFR6IO6fAtWaibLyKlM0z6wq9QYp3zB5grSL9mx8pzSq/uacRYAAAAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAAAAQEACAAAAAAAAAAAABCvMxuoMn+7NbHE/v8AAAAAAwIAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgRjb2luBHplcm8BB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYjJhAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACA3N1aQNTVUkACQEBAAECAAIBAAIAAAEEAAEAAAEFAAEGAAEDANbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrCAVjBjfrs4mwio33j60iZ8UjB6mGUyrtTBexLo2AffAsnQaRiJQAAAAAgje7Flwm0mf6pOwOHMRyljUNAIx5biyq8sA9hjzzGvdzW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwvQBAAAAAAAAQEtMAAAAAAAAAWEAmv0HqeZZ8XOfkxmBB62RbVfdjSO8RiD/poT82lU1wq8fuEJ3+monWXJZN3mm0h665bgWDx4XvjYCkth/odtKBgzQ7OmIzoPhw5nTC3tMzLjAySqs8CGINPAk+pl4i3Nm";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2",
        );
        assert_has_field_with_value(&payload, "Amount In", "10000000");
        assert_has_field_with_value(&payload, "Input Coin", "0x2::sui::SUI");
        assert_has_field_with_value(&payload, "Min Out", "0");
        assert_has_field_with_value(
            &payload,
            "Output Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
    }

    #[test]
    fn test_cetus_amm_swap_b2a_commands_third_tx() {
        // https://suivision.xyz/txblock/Bk4uGiLBvgffAm1xbMouH8DGHEJhfhxSgbti813oWAVh
        let test_data = "AQAAAAAACgAIxuLhDgsAAAABANrjBiF37dGB1OKbfljX5WgQ2q8YxkUKu2fHnce9BdSfEkBzJAAAAAAgRRAInzDvA4nI8OQ4s+88vErw2BJuUiK8P0VGgekuM4UBAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAABAQECVHR/XKBZoZcs1/YBZIXVE5Kj/eYIEHuTu66+pVD3A/Cu4QMAAAAAAQEAwIkrVFBIet9fD5OBXSlkfYiaINyGzLuQcPqcj8fTNwZQInMkAAAAACAQciN3HpDyJL7WVzaXrdyV9+JEICXyDNcJFixUrIg03QABAQAIxuLhDgsAAAAACIql3RyyAwAAABAAS6QhaDqlGwAAAAAAAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAICAQEAAQEAAACy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92Mghzd2FwX2IyYQIHqZuJUtT32Ufqd/4OzcyeX8C8qyhB1uKlqgDDBE5VRLUEbmF2eAROQVZYAAcAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgNzdWkDU1VJAAkBAgABAwABBAACAAABBQABBgABBwABCAABCQDTkatcUr8pYbYQ5Xwi0WJ4CJ5aSIqRmEGadstC52IoXwH84LYZx3/Y7zRNAxGOfynRbufmu1Ik1YMdlJq1/8W2BxJAcyQAAAAAIJmvqceKEaVtjr3EHxiMg/Dg+freKcbIEa56lMRn9s/v05GrXFK/KWG2EOV8ItFieAieWkiKkZhBmnbLQudiKF/0AQAAAAAAAMDGLQAAAAAAAAFhABWrbCk9cY5prO2fssnzj0d7sR6nyQWc6NByj3aZOMvlfOk5InGeiBVLOnAwTWeNLCUAWU1YB05FJK1VRr6YSQcokjX6Mv1y6tBhdsW9DTK2oFpe6mZpxbjjdyAuxH4CtA==";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_swap_b2a_commands_forth_tx() {
        // https://suivision.xyz/txblock/HzxMo7djbQkec2rauZkWAM553HeXKY6xmPqYBZ2r1MAG
        let test_data = "AQAAAAAACAAIAIhSanQAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFR6IO6fAtWaibLyKlM0z6wq9QYp3zB5grSL9mx8pzSq/uacRYAAAAAAQABAQAIAIhSanQAAAAACPz4HGYAAAAAABCvMxuoMn+7NbHE/v8AAAAAAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMCAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIEY29pbgR6ZXJvAQfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAAACy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92Mghzd2FwX2IyYQIH26NGcuMMsGWx+T46tVMYdo/W/vZsFZQsn3y4RuL5AOcEdXNkYwRVU0RDAAcAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgNzdWkDU1VJAAkBAQABAgACAQACAAABAwABBAABBQABBgABBwAcaIoVHssvxKcBZI0mdVEWC7s/5qsMgtbKwGjtbMmCyQGKONyRRNMDQSghvcxOqVVmhDYBIOTn0lvGYCPcX8GdCa9AcyQAAAAAIGO1zG8DupR7dx8zhJ0ejiWWYKnHuj+sgpRfO2AiVyGDHGiKFR7LL8SnAWSNJnVRFgu7P+arDILWysBo7WzJgsn0AQAAAAAAANyoMwAAAAAAAAFhAJskqwniXiTogMoofpB9dxQMs7nZLK1Qj6G2zvJhKSYJGITIcSW/AUruq0JQb8g19jG3rLa469wif46raZlR2A6AiyGXhnmh12WYynVXlQH2doDZ0v5LCrXENXPauhOQWQ==";

        let _payload = payload_from_b64(test_data);
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
        assert_has_field_with_value(&payload, "Max In", "1728516520");
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
        assert_has_field_with_value(&payload, "Max In", "188651600");
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

    #[test]
    fn test_cetus_amm_swap_a2b_commands_third_tx() {
        // https://suivision.xyz/txblock/FWTPqRt14LMk5E6MHmEeL8DrrP8LxBLZwiTNYv5C2VD2
        let test_data = "AQAAAAAACAEAKPL/nMBRjtJXaXgwAO5MjJyE3/r6crROZpN2jutuNpPCW2ElAAAAACBmSUQPADA5tcU3un74+OlSwCM5NzDjHAoPEWVTpV/jowAI6AMAAAAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQFR6IO6fAtWaibLyKlM0z6wq9QYp3zB5grSL9mx8pzSq/uacRYAAAAAAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAAAAQEACAAAAAAAAAAAABBQOwEAAQAAAAAAAAAAAAAAAwIBAAABAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBGNvaW4EemVybwEHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyCHN3YXBfYTJiAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACA3N1aQNTVUkACQECAAEDAAIAAAIBAAEFAAEBAAEGAAEHAAEEANbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrCAVjBjfrs4mwio33j60iZ8UjB6mGUyrtTBexLo2AffAsnwlthJQAAAAAgCQYRI4jFZ90G7uBUaTnnXL/em6I7yqCMpFMcgNUWwmHW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwvQBAAAAAAAAQEtMAAAAAAAAAWEAdvS3vusFOJ925KuD72lzpy2hrz3+Y/h/goPeG70udZem2mehbxBHOKaYmc+gFuaPZCTx1yFQyb78EZm/ZxCBBAzQ7OmIzoPhw5nTC3tMzLjAySqs8CGINPAk+pl4i3Nm";

        let payload = payload_from_b64(test_data);
        assert_has_field(&payload, CETUS_SWAP_LABEL);

        assert_has_field_with_value(
            &payload,
            "User Address",
            "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2",
        );
        assert_has_field_with_value(&payload, "Amount In", "1000");
        assert_has_field_with_value(
            &payload,
            "Input Coin",
            "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        );
        assert_has_field_with_value(&payload, "Min Out", "0");
        assert_has_field_with_value(&payload, "Output Coin", "0x2::sui::SUI");
    }

    #[test]
    fn test_cetus_amm_swap_a2b_with_partner_commands() {
        // https://suivision.xyz/txblock/ATxtv388auLq3k59UCH1fRkuEv5DqGE5sxCYPZj3kuLb
        let test_data = "AQAAAAAACgEABK2BDYzHL5IwPX7L2Y7ILkFRfrjFz8WRyqBrk9C6AIBC9mglAAAAACBx4VgVGsXhsixefZJH+UnHzApYl+TOVfC8ZqcKW3KDVwEAApk8pAQ+5WjlZ7K0t/pQKG58BCLcWtyyprOsSyT8rw5R9mglAAAAACAc3cwsssOMqQn3qBLnTy0ctl3UnmBALkZfSdhVFJ53EgEB2qRikmMsPE2PMfI+oPmzaij/NnfpaEmA5EOEA6Z6PY8uBRgAAAAAAAABAZ5Z3lDZ5ZefwDrFvKzbWByCPb0n1joDYTHhezkfL6yIxLxEFgAAAAABAAQ8av//AAR4av//AAgHuJNRAAAAAAAIqUSABgAAAAAAAQABAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAGAQAAAAAAAAAAAQCy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92MihvcGVuX3Bvc2l0aW9uX3dpdGhfbGlxdWlkaXR5X2J5X2ZpeF9jb2luAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB9Domyr15JEHJvvNi43Te7ebKeX4P3SRvKgw6U9/Im0pA2V0aANFVEgACgECAAEDAAEEAAEFAAEAAAEBAAEGAAEHAAEIAAEJAPidXAKZNb/4jeKkmC4Z+xHpq0T2HLqzpiMI/SMnBnPxAiNs4ll7dicNOdAobXtuAbjN3Tukt+M4TCrbsrv9+zvmUfZoJQAAAAAgmvNdUmEts7/PgyFbFDzF8ouiH8HkaIlGiuY+C0cPkZ49NsDGyKcW00rLfgKdDRHvPdvhVHLae1JM1Brh3sTx/lH2aCUAAAAAIP0Z4d9iXDPpfSGj7E3E8WiR5xgwKZRZa0TS3knAatxU+J1cApk1v/iN4qSYLhn7EemrRPYcurOmIwj9IycGc/H0AQAAAAAAAIRbmgAAAAAAAAFhAMHrIHOhoOBf24fr4+0F5laIMjDmqmOs7LTSkO96OHjnXJpqW72R84mivD3nWf5Ooz2Ecer5cYCzMC3qJ/4ZFAQxtQQzrW/0W0s0vU+R2sptnCRv7CcTtEl2m5cEqh/DUw==";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_open_position_with_liquidity_by_fix_coin() {
        // https://suivision.xyz/txblock/8u8iPWkdXZ7RBGNbJr5VQpPJX65WcQmk6YG8DAgBevKS
        let test_data = "AQAAAAAACgEABK2BDYzHL5IwPX7L2Y7ILkFRfrjFz8WRyqBrk9C6AIBC9mglAAAAACBx4VgVGsXhsixefZJH+UnHzApYl+TOVfC8ZqcKW3KDVwEAApk8pAQ+5WjlZ7K0t/pQKG58BCLcWtyyprOsSyT8rw5R9mglAAAAACAc3cwsssOMqQn3qBLnTy0ctl3UnmBALkZfSdhVFJ53EgEB2qRikmMsPE2PMfI+oPmzaij/NnfpaEmA5EOEA6Z6PY8uBRgAAAAAAAABAZ5Z3lDZ5ZefwDrFvKzbWByCPb0n1joDYTHhezkfL6yIxLxEFgAAAAABAAQ8av//AAR4av//AAgHuJNRAAAAAAAIqUSABgAAAAAAAQABAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAGAQAAAAAAAAAAAQCy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92MihvcGVuX3Bvc2l0aW9uX3dpdGhfbGlxdWlkaXR5X2J5X2ZpeF9jb2luAgfbo0Zy4wywZbH5Pjq1Uxh2j9b+9mwVlCyffLhG4vkA5wR1c2RjBFVTREMAB9Domyr15JEHJvvNi43Te7ebKeX4P3SRvKgw6U9/Im0pA2V0aANFVEgACgECAAEDAAEEAAEFAAEAAAEBAAEGAAEHAAEIAAEJAPidXAKZNb/4jeKkmC4Z+xHpq0T2HLqzpiMI/SMnBnPxAiNs4ll7dicNOdAobXtuAbjN3Tukt+M4TCrbsrv9+zvmUfZoJQAAAAAgmvNdUmEts7/PgyFbFDzF8ouiH8HkaIlGiuY+C0cPkZ49NsDGyKcW00rLfgKdDRHvPdvhVHLae1JM1Brh3sTx/lH2aCUAAAAAIP0Z4d9iXDPpfSGj7E3E8WiR5xgwKZRZa0TS3knAatxU+J1cApk1v/iN4qSYLhn7EemrRPYcurOmIwj9IycGc/H0AQAAAAAAAIRbmgAAAAAAAAFhAMHrIHOhoOBf24fr4+0F5laIMjDmqmOs7LTSkO96OHjnXJpqW72R84mivD3nWf5Ooz2Ecer5cYCzMC3qJ/4ZFAQxtQQzrW/0W0s0vU+R2sptnCRv7CcTtEl2m5cEqh/DUw==";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_open_position_with_liquidity_by_fix_coin_second_tx() {
        // https://suivision.xyz/txblock/3UQJTfxUsNi3sFf1pGAMySSps5JFoVemkRky4qS5Ucg5
        let test_data = "AQAAAAAADQEAIdG9id643w/m5RPHruFQV5vdWGZAJGQCcWfHpsO/f11E9mglAAAAACAbjbCqWl246M/gnk0lnlMjFUdqE8iFDlnwKgXH0BgPcQEAUWcAF17cmisYCpGEqrSNBZalq+Ba3HvgkYO3+04YgfVD9mglAAAAACAivTcVwkG8J59+eq2Bc+1RPzCf+jHa5ixyDqV6ieUOgwEAUs9eDepAhkTpBcXuUGU2BFs7YEhAmbb6gt5II40u1jFD9mglAAAAACDWeKXTffppR6KjtTEjcx11fqdPZWoMByO1J2NL16767QEA+5zLwnc32U9x+qaDZNZPqtGWcZv1elushXboh6sg/3xE9mglAAAAACBS5fPemy4sod7zIN7RuxeoOQ5vHssL/eBQd0WkBeb4VAAIB1/mCwAAAAABAdqkYpJjLDxNjzHyPqD5s2oo/zZ36WhJgORDhAOmej2PLgUYAAAAAAAAAQGeWd5Q2eWXn8A6xbys21gcgj29J9Y6A2Ex4Xs5Hy+siMS8RBYAAAAAAQAEPGr//wAEeGr//wAIMSoTfgAAAAAACAdf5gsAAAAAAAEBAQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgEAAAAAAAAAAAMDAQEAAgECAAEDAAIBAQABAQQAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tDnBvb2xfc2NyaXB0X3YyKG9wZW5fcG9zaXRpb25fd2l0aF9saXF1aWRpdHlfYnlfZml4X2NvaW4CB9ujRnLjDLBlsfk+OrVTGHaP1v72bBWULJ98uEbi+QDnBHVzZGMEVVNEQwAH0OibKvXkkQcm+82LjdN7t5sp5fg/dJG8qDDpT38ibSkDZXRoA0VUSAAKAQUAAQYAAQcAAQgAAQAAAgEAAQkAAQoAAQsAAQwAD5KsCrOxCAprOEjxSEm9Gd34JgvQPL98rNrhkKU0BdwCbLsUUlLf8hMg1bhpSOeMymcTonLs5XRw6lWg5ISgOp9E9mglAAAAACDubWOYdEomFl7kBvjLNEZKYv+O8hm6m00r2+Sd2fLdCHuEUnGjtQPzJIkELU5Xyybf1QizTVpFRGHJC2NE3YADRPZoJQAAAAAgoIFxetLAPtOZq6hG4UQh1y14AmEKvDnlFV2X4IZ6M/YPkqwKs7EICms4SPFISb0Z3fgmC9A8v3ys2uGQpTQF3PQBAAAAAAAABJSGAAAAAAAAAWEAKHWtwDLVtSKtYEZzoNqpNvxP2ejTXotkBIFaiKaxeP2KDW1VokDzbwAWGvg6QGaBZ4iXpaX7/8yC8xm7Tg3DCK3P6V54vH5i7dFu38FRxaTqZ34hdpw45HE8cK4YsbZO";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_open_position_with_liquidity_by_fix_coin_third_tx() {
        // https://suivision.xyz/txblock/46xdnvVfcCwW5FVEFJd9CyvJgUN2E2ajiqwrL9GtfBxP
        let test_data = "AQAAAAAADAEAd+lbtz5UMkffMEa0/+uEhgwm3dBiZxS7yPeC6xB1hUwjzHIkAAAAACA95iTQ52RdxERuramQegdRR5UzxGqcYVXTc80uf2kExQEAH/r7/9l7D+YuBdh0oToFNJwIZaOkqHRtcGxl+G1HeH6iQXMkAAAAACCZQxjpaX6MweDFgWcSMIUHwv0gSZ1o+RhvOQ/mErQ+PwAIAEDlnDASAAAACBhqfSArAAAAAQHapGKSYyw8TY8x8j6g+bNqKP82d+loSYDkQ4QDpno9jy4FGAAAAAAAAAEBE0FGfFrKhHr/RZ4H1qd3pvv3qntx/9ZswvS9/aFirdTX6ksFAAAAAAEABEh3//8ABBB4//8ACABA5ZwwEgAAAAgYan0gKwAAAAABAQEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAAEAwEAAAEBAQACAQAAAQECAAIAAQEDAACy23FC+oMhCn142cEqxJwEOzy71IIiT+puPaAKpaWuLQ5wb29sX3NjcmlwdF92MihvcGVuX3Bvc2l0aW9uX3dpdGhfbGlxdWlkaXR5X2J5X2ZpeF9jb2luAgdwFqrnLPxn8vrfVXacCn3VQpGlg7YwUaXtcQgczoNqxgNzY2EDU0NBAAcAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgNzdWkDU1VJAAoBBAABBQABBgABBwACAQACAgABCAABCQABCgABCwCaZN53MlJii982njfAGlP+cN+321aPg3dqAOqa0GtyywKctetqLfOc6zMv86glne35EBFMJBMp1yCmyE0n2ktlI6JBcyQAAAAAICM6YgQD2viZ7zbOLAwXHOxIPSSAhDad1z7TjuiGPPX6Ok52zALqYk9QRLQtfDE0TZZ0wVavvZ/pxFCrWgcxWUqiQXMkAAAAACCVVgT9OyuYVT6jOkqR82AbjWf6bzvge5yha5GfiqBIvZpk3ncyUmKL3zaeN8AaU/5w37fbVo+Dd2oA6prQa3LL9AEAAAAAAACc+aUAAAAAAAABYQAhlixbARONgX0KfhIOG1FzmqPbLNKmuONxKKC0xIGugccKido/VDb2pTmq4QyEuBFYctVm1DE+rK/wLeLCFIYJHuxex3WGJrKazMVDQYtli/DtJI3VSFwPPiV3JKnAiOo=";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_collect_reward_and_fee_close_position() {
        // https://suivision.xyz/txblock/4eCYtHQoiamuSh6fUvQCYrk8sszU6z6RvisKYQpq2cLv
        let test_data = "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";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_collect_reward_and_fee_close_position_second_tx() {
        // https://suivision.xyz/txblock/7WPL27d8LNsK9HoQaNriV3sz2YbucuzdfcdwxjGn6W3k
        let test_data = "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";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_collect_reward_and_fee_close_position_third_tx() {
        // https://suivision.xyz/txblock/2egauw5nHEaFxVjF77a6JC6ZPoUWEK9VrM7UUroFSQkj
        let test_data = "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";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_swap_and_check_coin_threshold() {
        // https://suivision.xyz/txblock/DtkWuUSgqMyNpxpRVGBHxt2WhkS1cDiis2By2veUAA97
        let test_data = "AQAAAAAADAEAKW8j+5q5TtyMcuDbDhwD0NlCokcg5e7kRPfuWiS2JhtD9mglAAAAACDXz5Q7UjNhTiys+54+/nXuIkmeEIeDTEFWTzwYCAt2BAEB2qRikmMsPE2PMfI+oPmzaij/NnfpaEmA5EOEA6Z6PY8uBRgAAAAAAAABAXJJGA687qoHAMT0hRUH6MNXkwuwbz26BBuIFsOdiOYlDhtkHgAAAAABAAEBAAEBAAgUAYKqBQAAAAAQUDsBAAEAAAAAAAAAAAAAAAABAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAAACMCxGaQCAAAAACBAO1SusxVl0r8BE9XhyMqqHmhsz7z7uyn7hIy8R4OtzgAgQDtUrrMVZdK/ARPV4cjKqh5obM+8+7sp+4SMvEeDrc4FAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACBGNvaW4EemVybwEHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tBnJvdXRlcgRzd2FwAgdt1Dne4FNVez3TQCh6S4EJmz5ynLSPva5ybdLf+Cc2wwVzbG92ZQVTTE9WRQAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAKAQEAAQIAAQAAAgAAAQMAAQQAAQUAAQYAAQcAAQgAALLbcUL6gyEKfXjZwSrEnAQ7PLvUgiJP6m49oAqlpa4tBnJvdXRlchRjaGVja19jb2luX3RocmVzaG9sZAEHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQACAwEAAQABCQABAQMBAAEAAQoAAQEDAQAAAAELAKBW+ly54yBFdmIQHIRBQFfEt4Tx2qx0lCYAHTwpL0HRApMD5tm09LNace6lr5TFveeS0D7jU5Is4vzCRGppj+cjfelnJQAAAAAg+Wt2/24GOhXGEWYPDTAMWmn3sNoqnHNQh5QS8KIJQNETxtQG2prdai3dceCif+u36YmHkPFkMd8V8oHR+JXop7vwaCUAAAAAIEex0IY4YXb2trareLH4sCyQME0PxZeWphfN0JRE0J+goFb6XLnjIEV2YhAchEFAV8S3hPHarHSUJgAdPCkvQdH0AQAAAAAAAMiHLgAAAAAAAAFhAIulKfTdl5w7PWdJO2JUvyuy2gRxm6guQIMg/Bj/ucPLZrgo547RdU0j15BPzHnBPVux7IfYqLgy9HmUF9Gdxg3yJzGwyIbWWjNuX/yDOwOQ/ueCGYe16TmM4mESU2OOkA==";

        let _payload = payload_from_b64(test_data);
    }

    #[test]
    fn test_cetus_amm_collect_reward_remove_liquidity_and_close_position() {
        // https://suivision.xyz/txblock/5GD7JBnjTZDqspScsY2SzY3iy1LKUBJBp7y3NzVnfVdP
        let test_data = "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";

        let _payload = payload_from_b64(test_data);
    }
}
