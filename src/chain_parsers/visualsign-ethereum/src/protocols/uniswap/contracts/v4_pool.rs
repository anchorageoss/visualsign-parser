use alloy_sol_types::{sol, SolCall};
use alloy_primitives::{Address, I256, U256};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};
use crate::registry::ContractRegistry;

sol! {
    interface IPoolManager {
        struct PoolKey {
            address currency0;
            address currency1;
            uint24 fee;
            int24 tickSpacing;
            address hooks;
        }

        struct ModifyLiquidityParams {
            int24 tickLower;
            int24 tickUpper;
            int256 liquidityDelta;
            bytes32 salt;
        }

        struct SwapParams {
            bool zeroForOne;
            int256 amountSpecified;
            uint160 sqrtPriceLimitX96;
        }

        struct BalanceDelta {
            int256 amount0;
            int256 amount1;
        }

        function initialize(PoolKey memory key, uint160 sqrt_price_x96) external returns (int24 tick);

        function unlock(bytes calldata data) external returns (bytes memory);
        function swap(PoolKey memory key, SwapParams memory params, bytes calldata hookData) external returns (BalanceDelta memory delta);
        function modifyLiquidity(PoolKey memory key, ModifyLiquidityParams memory params, bytes calldata hookData) external returns (BalanceDelta memory delta);
        function donate(PoolKey memory key, uint256 amount0, uint256 amount1, bytes calldata hookData) external returns (BalanceDelta memory delta);
    }

    interface IPoolManagerTest {
        struct RouterSwapParams {
            address token;
            uint256 amount;
            bytes data;
        }
        // This signature swap((address,uint256,bytes)) corresponds to 0x5742f567
        function swap(RouterSwapParams memory params) external;
    }
}

pub struct V4PoolManagerVisualizer;

impl V4PoolManagerVisualizer {
    pub fn visualize_tx_commands(
        &self, 
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if let Ok(call) = IPoolManager::initializeCall::abi_decode(input) {
            return Some(visualize_initialize(call, chain_id, registry));
        }

        
        if let Ok(call) = IPoolManagerTest::swapCall::abi_decode(input) {
             return Some(visualize_swap(call, chain_id, registry));
        }

        None
    }
}

fn visualize_initialize(
    call: IPoolManager::initializeCall,
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    let key = call.key;
    
    // Resolve symbols
    let symbol0 = registry
        .and_then(|r| r.get_token_symbol(chain_id, key.currency0))
        .unwrap_or_else(|| format!("{:?}", key.currency0));
        
    let symbol1 = registry
        .and_then(|r| r.get_token_symbol(chain_id, key.currency1))
        .unwrap_or_else(|| format!("{:?}", key.currency1));

    create_initialize_preview_layout(symbol0, symbol1, key, call.sqrt_price_x96)
}

fn create_initialize_preview_layout(
    symbol0: String,
    symbol1: String,
    key: IPoolManager::PoolKey,
    sqrt_price_x96: alloy_primitives::Uint<160, 3>,
) -> SignablePayloadField {
    // Create fields for the PreviewLayout
    let fields = vec![
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: symbol0.clone(),
                    label: "Currency 0".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: symbol0.clone(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: symbol1.clone(),
                    label: "Currency 1".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: symbol1.clone(),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{}", key.fee),
                    label: "Fee".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{} ({}%)", key.fee, (key.fee.to::<u32>() as f64) / 10000.0),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{}", key.tickSpacing),
                    label: "Tick Spacing".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{}", key.tickSpacing),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{}", U256::from(sqrt_price_x96)),
                    label: "Sqrt Price X96".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{}", U256::from(sqrt_price_x96)),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
    ];

    SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("Uniswap V4: Initialize Pool ({}/{})", symbol0, symbol1),
            label: "Initialize Pool".to_string(),
        },
        preview_layout: SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: "Initialize V4 Pool".to_string(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: format!("Fee: {}%", (key.fee.to::<u32>() as f64) / 10000.0),
            }),
            condensed: None,
            expanded: Some(SignablePayloadFieldListLayout { fields }),
        },
    }
}

fn visualize_swap(
    call: IPoolManagerTest::swapCall,
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    let params = call.params;
    
    let token = params.token;
    let amount = params.amount;
    
     // Resolve symbol
    let (symbol, name, verified) = registry
        .and_then(|r| r.get_token_metadata(chain_id, token))
        .map(|m| (m.symbol.clone(), m.name.clone(), true))
        .unwrap_or_else(|| (format!("{:?}", token), format!("{:?}", token), false));
        
    // Format amount
    let (amount_fmt, _) = registry
        .and_then(|r| {
            r.format_token_amount(chain_id, token, amount.try_into().unwrap_or(0))
        })
        .unwrap_or_else(|| (amount.to_string(), symbol.clone()));
        
    let fields = vec![
         AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{} ({})", symbol, token),
                    label: "Token".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: token.to_string(),
                    name: name,
                    memo: None,
                    asset_label: symbol.clone(),
                    badge_text: if verified { Some("Verified".to_string()) } else { None },
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
        AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{} {}", amount_fmt, symbol),
                    label: "Amount".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: amount_fmt.clone(),
                    abbreviation: Some(symbol.clone()),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        },
    ];

    SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("Swap {} {}", amount_fmt, symbol),
            label: "Swap".to_string(),
        },
        preview_layout: SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: "Swap".to_string(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: "Test Router".to_string(),
            }),
            condensed: None,
            expanded: Some(SignablePayloadFieldListLayout { fields }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_initialize_result() {
        let visualizer = V4PoolManagerVisualizer;
        
        // Initialize calldata
        let calldata_str = concat!(
            "695c5bf5",
            "0000000000000000000000000000000000000001000000000000000000000000",
            "0000000000000000000000000000000000000002000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000bb8",
            "000000000000000000000000000000000000000000000000000000000000003c",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000100000000000000000000000000000000000000",
            "00000000000000000000000000000000000000000000000000000000000000e0",
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
        
        let calldata = hex::decode(calldata_str).unwrap();

        if let Some(field) = visualizer.visualize_tx_commands(&calldata, 1, None) {
            println!("\n=== INITIALIZE VISUALIZATION RESULT ===\n");
            println!("{:#?}", field);
            println!("\n=======================================\n");
        } else {
            println!("Failed to visualize initialize");
        }
    }

    #[test]
    fn test_visualize_empty_input() {
        let visualizer = V4PoolManagerVisualizer;
        assert_eq!(visualizer.visualize_tx_commands(&[], 1, None), None);
    }

    #[test]
    fn test_visualize_too_short() {
        let visualizer = V4PoolManagerVisualizer;
        assert_eq!(visualizer.visualize_tx_commands(&[0x01, 0x02], 1, None), None);
    }
    
    #[test]
    fn test_visualize_initialize() {
        let visualizer = V4PoolManagerVisualizer;
        
        let calldata_str = concat!(
            "695c5bf5",
            "0000000000000000000000000000000000000001000000000000000000000000",
            "0000000000000000000000000000000000000002000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000bb8",
            "000000000000000000000000000000000000000000000000000000000000003c",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000100000000000000000000000000000000000000",
            "00000000000000000000000000000000000000000000000000000000000000e0",
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
        let calldata = hex::decode(calldata_str).unwrap();

        let result = visualizer.visualize_tx_commands(&calldata, 1, None);
        
        assert!(result.is_some());
        let field = result.unwrap();
        
        if let SignablePayloadField::PreviewLayout { common, preview_layout } = field {
             assert_eq!(common.label, "Initialize Pool");
             assert!(common.fallback_text.contains("Uniswap V4: Initialize Pool"));
             
             let subtitle = preview_layout.subtitle.unwrap();
             assert_eq!(subtitle.text, "Fee: 0.3%"); 
             
             let expanded = preview_layout.expanded.unwrap();
             assert_eq!(expanded.fields.len(), 5); 
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_visualize_swap() {
        let visualizer = V4PoolManagerVisualizer;

        let calldata = hex::decode("f3cd914c000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000bb8000000000000000000000000000000000000000000000000000000000000003c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000de0b6b3a7640000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000000").unwrap();

        let result = visualizer.visualize_tx_commands(&calldata, 1, None);

        assert!(result.is_some());
        let field = result.unwrap();
        
        if let SignablePayloadField::PreviewLayout { common, preview_layout } = field {
             assert_eq!(common.label, "Swap");
             println!("Swap subtitle: {}", preview_layout.subtitle.as_ref().unwrap().text);
             assert!(preview_layout.subtitle.as_ref().unwrap().text.contains("0.3% Fee"));
             
             let expanded = preview_layout.expanded.unwrap();
             assert!(expanded.fields.len() >= 3);
        } else {
            panic!("Expected PreviewLayout");
        }
    }
}
