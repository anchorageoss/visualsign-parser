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

        function initialize(PoolKey memory key, uint160 sqrtPriceX96, bytes calldata hooksData) external returns (int24 tick);
        function unlock(bytes calldata data) external returns (bytes memory);
        function swap(PoolKey memory key, SwapParams memory params, bytes calldata hookData) external returns (BalanceDelta memory delta);
        function modifyLiquidity(PoolKey memory key, ModifyLiquidityParams memory params, bytes calldata hookData) external returns (BalanceDelta memory delta);
        function donate(PoolKey memory key, uint256 amount0, uint256 amount1, bytes calldata hookData) external returns (BalanceDelta memory delta);
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

        if let Ok(call) = IPoolManager::swapCall::abi_decode(input) {
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
                    fallback_text: format!("{}", call.sqrtPriceX96),
                    label: "Sqrt Price X96".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("{}", call.sqrtPriceX96),
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
    call: IPoolManager::swapCall,
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    let key = call.key;
    let params = call.params;
    let hook_data = call.hookData;

    // 1. Determine tokens
    let (token_in, token_out) = if params.zeroForOne {
        (key.currency0, key.currency1)
    } else {
        (key.currency1, key.currency0)
    };

    // 2. Resolve symbols and metadata
    let (symbol_in, name_in, verified_in) = registry
        .and_then(|r| r.get_token_metadata(chain_id, token_in))
        .map(|m| (m.symbol.clone(), m.name.clone(), true))
        .unwrap_or_else(|| (format!("{:?}", token_in), format!("{:?}", token_in), false));
    
    let (symbol_out, name_out, verified_out) = registry
        .and_then(|r| r.get_token_metadata(chain_id, token_out))
        .map(|m| (m.symbol.clone(), m.name.clone(), true))
        .unwrap_or_else(|| (format!("{:?}", token_out), format!("{:?}", token_out), false));

    // 3. Determine amount and direction
    // amountSpecified: positive = exact input, negative = exact output
    let is_exact_input = !params.amountSpecified.is_negative();
    
    let amount_abs_u256: U256 = if params.amountSpecified.is_negative() {
        let negated = params.amountSpecified.checked_neg().unwrap_or(I256::ZERO);
        negated.into_raw()
    } else {
        params.amountSpecified.into_raw()
    };

    // 4. Format amount
    let (amount_fmt, _) = registry
        .and_then(|r| {
            let token = if is_exact_input { token_in } else { token_out };
            let amount_u128: u128 = amount_abs_u256.try_into().ok()?;
            r.format_token_amount(chain_id, token, amount_u128)
        })
        .unwrap_or_else(|| (amount_abs_u256.to_string(), if is_exact_input { symbol_in.clone() } else { symbol_out.clone() }));

    // 5. Construct Fields for Preview Layout
    let mut fields = Vec::new();

    // Input Token Field
    if is_exact_input {
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{} {}", amount_fmt, symbol_in),
                    label: "Selling (Exact)".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: amount_fmt.clone(),
                    abbreviation: Some(symbol_in.clone()),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    } else {
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Unknown {}", symbol_in),
                    label: "Selling (Est.)".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("Unknown Amount of {}", symbol_in),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }

    // Add expanded field for Input Token Address with Name and Badge
    fields.push(AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} ({})", symbol_in, token_in),
                label: "Input Token".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: token_in.to_string(),
                name: name_in,
                memo: None,
                asset_label: symbol_in.clone(),
                badge_text: if verified_in { Some("Verified".to_string()) } else { None },
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    });

    // Output Token Field
    if !is_exact_input {
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AmountV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("{} {}", amount_fmt, symbol_out),
                    label: "Buying (Exact)".to_string(),
                },
                amount_v2: SignablePayloadFieldAmountV2 {
                    amount: amount_fmt.clone(),
                    abbreviation: Some(symbol_out.clone()),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    } else {
        // For exact input swaps, output amount is determined at execution time
        // based on current pool state and price impact
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Estimated {} (determined at execution)", symbol_out),
                    label: "Buying (Est.)".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("Estimated {} (amount determined at execution)", symbol_out),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }

    // Add expanded field for Output Token Address with Name and Badge
    fields.push(AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{} ({})", symbol_out, token_out),
                label: "Output Token".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: token_out.to_string(),
                name: name_out,
                memo: None,
                asset_label: symbol_out.clone(),
                badge_text: if verified_out { Some("Verified".to_string()) } else { None },
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    });

    // Fee Field
    let fee_pct = (key.fee.to::<u32>() as f64) / 10000.0;
    fields.push(AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{}%", fee_pct),
                label: "Pool Fee".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{}%", fee_pct),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    });

    // Price Limit Field (slippage protection) - only show if set (non-zero)
    // sqrtPriceLimitX96 = 0 means no price limit
    let price_limit_str = params.sqrtPriceLimitX96.to_string();
    if price_limit_str != "0" {
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Price limit: {}", price_limit_str),
                    label: "Price Limit (Slippage Protection)".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("Price limit: {} (sqrtPriceX96)", price_limit_str),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }

    // Hook Address Field (if present)
    if key.hooks != Address::ZERO {
        fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::AddressV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: key.hooks.to_string(),
                    label: "Hook Address".to_string(),
                },
                address_v2: SignablePayloadFieldAddressV2 {
                    address: key.hooks.to_string(),
                    name: "Hook".to_string(),
                    memo: None,
                    asset_label: "".to_string(),
                    badge_text: None,
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }
    
    // Hook Data Field (if present)
    if !hook_data.is_empty() {
         fields.push(AnnotatedPayloadField {
            signable_payload_field: SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: hex::encode(&hook_data),
                    label: "Hook Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("0x{}", hex::encode(&hook_data)),
                },
            },
            static_annotation: None,
            dynamic_annotation: None,
        });
    }

    // Construct Header Text
    let header_text = if is_exact_input {
        format!("Swap {} {} for {}", amount_fmt, symbol_in, symbol_out)
    } else {
        format!("Swap {} for {} {}", symbol_in, amount_fmt, symbol_out)
    };
    
    let subtitle_text = format!("V4 Pool ({}% Fee)", fee_pct);

    SignablePayloadField::PreviewLayout {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{} via V4", header_text),
            label: "Swap".to_string(),
        },
        preview_layout: SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 {
                text: "Swap".to_string(),
            }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: subtitle_text,
            }),
            condensed: Some(SignablePayloadFieldListLayout {
                 // Show minimal info in condensed view: Just input and output
                 fields: fields.iter().take(2).cloned().collect(),
            }),
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
        
        // Initialize calldata - This string was previously problematic due to formatting
        // Reconstructing carefully from known good output
        let calldata_str = concat!(
            "695c5bf5",
            "0000000000000000000000000000000000000001000000000000000000000000", // Currency0
            "0000000000000000000000000000000000000002000000000000000000000000", // Currency1
            "0000000000000000000000000000000000000000000000000000000000000bb8", // Fee
            "000000000000000000000000000000000000000000000000000000000000003c", // TickSpacing
            "0000000000000000000000000000000000000000000000000000000000000000", // Hooks
            "0000000000000000000000000100000000000000000000000000000000000000", // SqrtPriceX96
            "00000000000000000000000000000000000000000000000000000000000000e0", // Offset
            "0000000000000000000000000000000000000000000000000000000000000000"  // Length
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
        
        // Reusing the constructed string
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
        
        // Verify it's now a PreviewLayout with rich details
        if let SignablePayloadField::PreviewLayout { common, preview_layout } = field {
             assert_eq!(common.label, "Initialize Pool");
             assert!(common.fallback_text.contains("Uniswap V4: Initialize Pool"));
             
             // Check subtitle
             let subtitle = preview_layout.subtitle.unwrap();
             assert_eq!(subtitle.text, "Fee: 0.3%"); // 3000 fee = 0.3%
             
             // Check expanded fields
             let expanded = preview_layout.expanded.unwrap();
             assert_eq!(expanded.fields.len(), 5); // currency0, currency1, fee, tickSpacing, sqrtPriceX96
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_visualize_swap() {
        let visualizer = V4PoolManagerVisualizer;

        // Generated with:
        // cast calldata "swap((address,address,uint24,int24,address),(bool,int256,uint160),bytes)" \
        //   "(0x0000000000000000000000000000000000000001,0x0000000000000000000000000000000000000002,3000,60,0x0000000000000000000000000000000000000000)" \
        //   "(true,1000000000000000000,0)" 0x
        let calldata = hex::decode("f3cd914c000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000bb8000000000000000000000000000000000000000000000000000000000000003c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000de0b6b3a7640000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001200000000000000000000000000000000000000000000000000000000000000000").unwrap();

        let result = visualizer.visualize_tx_commands(&calldata, 1, None);

        assert!(result.is_some());
        let field = result.unwrap();
        
        // Updated test for PreviewLayout
        if let SignablePayloadField::PreviewLayout { common, preview_layout } = field {
             assert_eq!(common.label, "Swap");
             println!("Swap subtitle: {}", preview_layout.subtitle.as_ref().unwrap().text);
             assert!(preview_layout.subtitle.as_ref().unwrap().text.contains("0.3% Fee"));
             
             let expanded = preview_layout.expanded.unwrap();
             // We expect at least 3 fields: Sell, Buy, Fee
             assert!(expanded.fields.len() >= 3);
        } else {
            panic!("Expected PreviewLayout");
        }
    }
}
