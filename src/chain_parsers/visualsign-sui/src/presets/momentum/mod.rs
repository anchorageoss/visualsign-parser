mod config;

use config::{Config, LiquidityFunctions, MOMENTUM_CONFIG};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{SuiCoin, get_tx_type_arg, truncate_address};

use sui_json_rpc_types::{SuiCommand, SuiProgrammableMoveCall};

use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_address_field;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    field_builders::create_text_field,
};

pub struct MomentumVisualizer;

impl CommandVisualizer for MomentumVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Momentum parsing".into(),
            ));
        };

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(e) => return Err(VisualSignError::DecodeError(e)),
        };

        println!("1");

        match function {
            LiquidityFunctions::RemoveLiquidity => Ok(self.handle_remove_liquidity(context, pwc)?),
            LiquidityFunctions::ClosePosition => Ok(self.handle_close_position(context)?),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(MOMENTUM_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Cetus")
    }
}

impl MomentumVisualizer {
    fn handle_remove_liquidity(
        &self,
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let coin_1: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 0).unwrap_or_default();
        let coin_2: SuiCoin = get_tx_type_arg(&pwc.type_arguments, 1).unwrap_or_default();

        let mut list_layout_fields = vec![create_address_field(
            "Sender",
            &context.sender().to_string(),
            None,
            None,
            None,
            None,
        )?];

        list_layout_fields.push(create_text_field("Coin 1", &coin_1.to_string())?);
        list_layout_fields.push(create_text_field("Coin 2", &coin_2.to_string())?);

        {
            let title_text = format!(
                "Momentum Remove Liquidity from pair {}/{}",
                coin_1.symbol(),
                coin_2.symbol()
            );
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!(
                        "Remove liquidity from pair {}/{} to {}",
                        coin_1.symbol(),
                        coin_2.symbol(),
                        &context.sender().to_string(),
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
                        label: "Momentum Remove Liquidity Command".to_string(),
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

    fn handle_close_position(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let list_layout_fields = vec![create_address_field(
            "Sender",
            &context.sender().to_string(),
            None,
            None,
            None,
            None,
        )?];

        {
            let title_text = "Momentum Close Position".to_string();
            let subtitle_text = format!("From {}", truncate_address(&context.sender().to_string()));

            let condensed = SignablePayloadFieldListLayout {
                fields: vec![create_text_field(
                    "Summary",
                    &format!("Close position for {}", &context.sender().to_string()),
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
                        label: "Momentum Close Position Command".to_string(),
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

    use visualsign::test_utils::assert_has_field;

    #[test]
    fn test_momentum_remove_liquidity() {
        // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
        let test_data = "AQAAAAAACQEBPaCQ0SWho3nWCgPDOKD6unBgRzh8TFJfRUXNyoR8CztrLWshAAAAAAEBAGui4JnRVsicDXzXmGFNQvRmndeFmEicY7+jg9JMG+ZQfMd1JAAAAAAgCSKS99j5XY79h/qhe06kf9pgB7VObJ06G/l6Ud9XAGQAEBxPFM1qAQAAAAAAAAAAAAAACAAAAAAAAAAAAAgAAAAAAAAAAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAABASN1oLHsEgEKrqOyVFrPoq00z7ugPOS1n0w54eJe7RsqZMDJHQAAAAAAACAfXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9QAgH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUFAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eRByZW1vdmVfbGlxdWlkaXR5AgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAHAQAAAQEAAQIAAQMAAQQAAQUAAQYAAQIDAAAAAAMAAAEAAQcAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRB2NvbGxlY3QDZmVlAgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAEAQAAAQEAAQUAAQYAAQIDAgAAAAMCAAEAAQgAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eQ5jbG9zZV9wb3NpdGlvbgACAQEAAQYAH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUCkbzKYJjNnW1dS+OSg47AfzhenXHE5j3YEbVj3w12vjXw4nUkAAAAACBtbk7awxrfKFU5O/j7O18DlbaWBF5AuSr4VpAuZYT9myUkbWMUm6dirPubSAoZWYWkarBH6bfjxezwFmpyxTOW8OJ1JAAAAAAgMZMdJNCNAIa0d8vNuiN4ghW7faU/0/TTTP670s5Pq0ofXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9fQBAAAAAAAAYOMWAAAAAAAAAWEA6Rn4TrqLBl72XmEPSColPnONOY5JiYtLk6F/aQKMWL88mC9+MptS02/JP1+LD8sFsJQD1f8LngMtuLPHny5cAB1S0wCE/sDcB5tDvq1+juWWCcJmS9clXEb99ez37zYB";

        let payload = payload_from_b64(test_data);

        assert_has_field(&payload, "Momentum Remove Liquidity Command");
    }

    #[test]
    fn test_momentum_close_position() {
        // https://suivision.xyz/txblock/5QMTpn34NuBvMMAU1LeKhWKSNTMoJEriEier3DA8tjNU
        let test_data = "AQAAAAAACQEBPaCQ0SWho3nWCgPDOKD6unBgRzh8TFJfRUXNyoR8CztrLWshAAAAAAEBAGui4JnRVsicDXzXmGFNQvRmndeFmEicY7+jg9JMG+ZQfMd1JAAAAAAgCSKS99j5XY79h/qhe06kf9pgB7VObJ06G/l6Ud9XAGQAEBxPFM1qAQAAAAAAAAAAAAAACAAAAAAAAAAAAAgAAAAAAAAAAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAYBAAAAAAAAAAABASN1oLHsEgEKrqOyVFrPoq00z7ugPOS1n0w54eJe7RsqZMDJHQAAAAAAACAfXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9QAgH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUFAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eRByZW1vdmVfbGlxdWlkaXR5AgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAHAQAAAQEAAQIAAQMAAQQAAQUAAQYAAQIDAAAAAAMAAAEAAQcAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRB2NvbGxlY3QDZmVlAgfxbmtyPyQux0Xf12NK0HLELVwdmsnWKjnDgTA+qldpOgVmZHVzZAVGRFVTRAAHAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDc3VpA1NVSQAEAQAAAQEAAQUAAQYAAQIDAgAAAAMCAAEAAQgAAM9gpA9F1G/B6CiHGmR8HiWgkV3shg0mYusQ/bOCw8HRCWxpcXVpZGl0eQ5jbG9zZV9wb3NpdGlvbgACAQEAAQYAH15kwx8D9kZbtJi352HL2SByIB0qPJeWET9+xjQisvUCkbzKYJjNnW1dS+OSg47AfzhenXHE5j3YEbVj3w12vjXw4nUkAAAAACBtbk7awxrfKFU5O/j7O18DlbaWBF5AuSr4VpAuZYT9myUkbWMUm6dirPubSAoZWYWkarBH6bfjxezwFmpyxTOW8OJ1JAAAAAAgMZMdJNCNAIa0d8vNuiN4ghW7faU/0/TTTP670s5Pq0ofXmTDHwP2Rlu0mLfnYcvZIHIgHSo8l5YRP37GNCKy9fQBAAAAAAAAYOMWAAAAAAAAAWEA6Rn4TrqLBl72XmEPSColPnONOY5JiYtLk6F/aQKMWL88mC9+MptS02/JP1+LD8sFsJQD1f8LngMtuLPHny5cAB1S0wCE/sDcB5tDvq1+juWWCcJmS9clXEb99ez37zYB";

        let payload = payload_from_b64(test_data);

        assert_has_field(&payload, "Momentum Close Position Command");
    }
}
