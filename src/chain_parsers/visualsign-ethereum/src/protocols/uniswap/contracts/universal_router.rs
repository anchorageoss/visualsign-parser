use alloy_sol_types::{SolCall as _, sol};
use chrono::{TimeZone, Utc};
use num_enum::TryFromPrimitive;
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

use crate::registry::ContractRegistry;

// From: https://github.com/Uniswap/universal-router/blob/main/contracts/interfaces/IUniversalRouter.sol
sol! {
    interface IUniversalRouter {
        /// @notice Executes encoded commands along with provided inputs. Reverts if deadline has expired.
        /// @param commands A set of concatenated commands, each 1 byte in length
        /// @param inputs An array of byte strings containing abi encoded inputs for each command
        /// @param deadline The deadline by which the transaction must be executed
        function execute(bytes calldata commands, bytes[] calldata inputs, uint256 deadline) external payable;
    }
}

// Command parameter structures
// From: https://github.com/Uniswap/universal-router/blob/main/contracts/modules/uniswap/v3/V3SwapRouter.sol
sol! {
    /// Parameters for V3_SWAP_EXACT_IN command
    struct V3SwapExactInputParams {
        address recipient;
        uint256 amountIn;
        uint256 amountOutMinimum;
        bytes path;
        bool payerIsUser;
    }

    /// Parameters for V3_SWAP_EXACT_OUT command
    struct V3SwapExactOutputParams {
        address recipient;
        uint256 amountOut;
        uint256 amountInMaximum;
        bytes path;
        bool payerIsUser;
    }

    /// Parameters for PAY_PORTION command
    struct PayPortionParams {
        address token;
        address recipient;
        uint256 bips;
    }

    /// Parameters for UNWRAP_WETH command
    struct UnwrapWethParams {
        address recipient;
        uint256 amountMinimum;
    }
}

// From: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Commands.sol
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum Command {
    V3SwapExactIn = 0x00,
    V3SwapExactOut = 0x01,
    Permit2TransferFrom = 0x02,
    Permit2PermitBatch = 0x03,
    Sweep = 0x04,
    Transfer = 0x05,
    PayPortion = 0x06,

    V2SwapExactIn = 0x08,
    V2SwapExactOut = 0x09,
    Permit2Permit = 0x0a,
    WrapEth = 0x0b,
    UnwrapWeth = 0x0c,
    Permit2TransferFromBatch = 0x0d,
    BalanceCheckErc20 = 0x0e,

    V4Swap = 0x10,
    V3PositionManagerPermit = 0x11,
    V3PositionManagerCall = 0x12,
    V4InitializePool = 0x13,
    V4PositionManagerCall = 0x14,

    ExecuteSubPlan = 0x21,
}

fn map_commands(raw: &[u8]) -> Vec<Command> {
    let mut out = Vec::with_capacity(raw.len());
    for &b in raw {
        if let Ok(cmd) = Command::try_from(b) {
            out.push(cmd);
        }
    }
    out
}

/// Visualizer for Uniswap Universal Router
///
/// Handles the `execute` function from IUniversalRouter interface:
/// <https://github.com/Uniswap/universal-router/blob/dev/contracts/interfaces/IUniversalRouter.sol>
pub struct UniversalRouterVisualizer {}

impl UniversalRouterVisualizer {
    /// Visualizes Universal Router execute commands
    ///
    /// # Arguments
    /// * `input` - The calldata bytes
    /// * `chain_id` - The chain ID for registry lookups
    /// * `registry` - Optional registry for resolving token symbols
    pub fn visualize_tx_commands(
        &self,
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }
        if let Ok(call) = IUniversalRouter::executeCall::abi_decode(input) {
            let deadline_val: i64 = match call.deadline.try_into() {
                Ok(val) => val,
                Err(_) => return None,
            };
            let deadline = if deadline_val > 0 {
                Utc.timestamp_opt(deadline_val, 0)
                    .single()
                    .map(|dt| dt.to_string())
            } else {
                None
            };
            let mapped = map_commands(&call.commands.0);
            let mut detail_fields = Vec::new();

            for (i, cmd) in mapped.iter().enumerate() {
                let input_bytes = call.inputs.get(i).map(|b| &b.0[..]);
                let input_hex = input_bytes
                    .map(|b| format!("0x{}", hex::encode(b)))
                    .unwrap_or_else(|| "None".to_string());

                // Decode command-specific parameters (TODO: implement actual decoding)
                // For now, all commands use the same hex format until decoders are implemented
                detail_fields.push(SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{cmd:?} input: {input_hex}"),
                        label: format!("Command {}", i + 1),
                    },
                    preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                        title: Some(visualsign::SignablePayloadFieldTextV2 {
                            text: format!("{cmd:?}"),
                        }),
                        subtitle: Some(visualsign::SignablePayloadFieldTextV2 {
                            text: format!("Input: {input_hex}"),
                        }),
                        condensed: None,
                        expanded: None,
                    },
                });
            }

            // Deadline field (optional)
            if let Some(dl) = &deadline {
                detail_fields.push(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: dl.clone(),
                        label: "Deadline".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: dl.clone() },
                });
            }

            return Some(SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: if let Some(dl) = &deadline {
                        format!(
                            "Universal Router Execute: {} commands ({:?}), deadline {}",
                            mapped.len(),
                            mapped,
                            dl
                        )
                    } else {
                        format!(
                            "Universal Router Execute: {} commands ({:?})",
                            mapped.len(),
                            mapped
                        )
                    },
                    label: "Universal Router".to_string(),
                },
                preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                    title: Some(visualsign::SignablePayloadFieldTextV2 {
                        text: "Universal Router Execute".to_string(),
                    }),
                    subtitle: if let Some(dl) = &deadline {
                        Some(visualsign::SignablePayloadFieldTextV2 {
                            text: format!("{} commands, deadline {}", mapped.len(), dl),
                        })
                    } else {
                        Some(visualsign::SignablePayloadFieldTextV2 {
                            text: format!("{} commands", mapped.len()),
                        })
                    },
                    condensed: None,
                    expanded: Some(visualsign::SignablePayloadFieldListLayout {
                        fields: detail_fields
                            .into_iter()
                            .map(|f| visualsign::AnnotatedPayloadField {
                                signable_payload_field: f,
                                static_annotation: None,
                                dynamic_annotation: None,
                            })
                            .collect(),
                    }),
                },
            });
        }
        None
    }

    // TODO: Implement command decoders
    //
    // /// Decodes V3_SWAP_EXACT_IN command parameters
    // fn decode_v3_swap_exact_in(
    //     bytes: &[u8],
    //     chain_id: u64,
    //     registry: Option<&ContractRegistry>,
    // ) -> SignablePayloadField {
    //     // Decode V3SwapExactInputParams
    //     // Parse path to extract tokens and fees
    //     // Resolve token symbols from registry
    //     // Display: "Swap X TOKEN_A for ≥Y TOKEN_B"
    // }
    //
    // /// Decodes PAY_PORTION command parameters
    // fn decode_pay_portion(
    //     bytes: &[u8],
    //     chain_id: u64,
    //     registry: Option<&ContractRegistry>,
    // ) -> SignablePayloadField {
    //     // Decode PayPortionParams
    //     // Display: "Pay X% of TOKEN to RECIPIENT"
    // }
    //
    // /// Decodes UNWRAP_WETH command parameters
    // fn decode_unwrap_weth(
    //     bytes: &[u8],
    //     chain_id: u64,
    //     registry: Option<&ContractRegistry>,
    // ) -> SignablePayloadField {
    //     // Decode UnwrapWethParams
    //     // Display: "Unwrap ≥X WETH to ETH for RECIPIENT"
    // }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, U256};
    use visualsign::{
        AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
        SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout,
        SignablePayloadFieldTextV2,
    };

    fn encode_execute_call(commands: &[u8], inputs: Vec<Vec<u8>>, deadline: u64) -> Vec<u8> {
        let inputs_bytes = inputs.into_iter().map(Bytes::from).collect::<Vec<_>>();
        IUniversalRouter::executeCall {
            commands: Bytes::from(commands.to_vec()),
            inputs: inputs_bytes,
            deadline: U256::from(deadline),
        }
        .abi_encode()
    }

    #[test]
    fn test_visualize_tx_commands_empty_input() {
        assert_eq!(UniversalRouterVisualizer {}.visualize_tx_commands(&[], 1, None), None);
        assert_eq!(
            UniversalRouterVisualizer {}.visualize_tx_commands(&[0x01, 0x02, 0x03], 1, None),
            None
        );
    }

    #[test]
    fn test_visualize_tx_commands_invalid_deadline() {
        // deadline is not convertible to i64 (u64::MAX)
        let input = encode_execute_call(&[0x00], vec![vec![0x01, 0x02]], u64::MAX);
        assert_eq!(UniversalRouterVisualizer {}.visualize_tx_commands(&input, 1, None), None);
    }

    #[test]
    fn test_visualize_tx_commands_single_command_with_deadline() {
        let commands = vec![Command::V3SwapExactIn as u8];
        let inputs = vec![vec![0xde, 0xad, 0xbe, 0xef]];
        let deadline = 1_700_000_000u64; // 2023-11-13T12:26:40Z
        let input = encode_execute_call(&commands, inputs.clone(), deadline);

        // Build expected field
        let dt = chrono::Utc.timestamp_opt(deadline as i64, 0).unwrap();
        let deadline_str = dt.to_string();

        assert_eq!(
            UniversalRouterVisualizer {}
                .visualize_tx_commands(&input, 1, None)
                .unwrap(),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "Universal Router Execute: 1 commands ([V3SwapExactIn]), deadline {deadline_str}"
                    ),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: format!("1 commands, deadline {deadline_str}"),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "V3SwapExactIn input: 0xdeadbeef"
                                            .to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "V3SwapExactIn".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Input: 0xdeadbeef".to_string(),
                                        }),
                                        condensed: None,
                                        expanded: None,
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::TextV2 {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: deadline_str.clone(),
                                        label: "Deadline".to_string(),
                                    },
                                    text_v2: SignablePayloadFieldTextV2 {
                                        text: deadline_str.clone(),
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                        ],
                    }),
                },
            }
        );
    }

    #[test]
    fn test_visualize_tx_commands_multiple_commands_no_deadline() {
        let commands = vec![
            Command::V3SwapExactIn as u8,
            Command::Transfer as u8,
            Command::WrapEth as u8,
        ];
        let inputs = vec![vec![0x01, 0x02], vec![0x03, 0x04, 0x05], vec![0x06]];
        let deadline = 0u64;
        let input = encode_execute_call(&commands, inputs.clone(), deadline);

        assert_eq!(
            UniversalRouterVisualizer {}
                .visualize_tx_commands(&input, 1, None)
                .unwrap(),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text:
                        "Universal Router Execute: 3 commands ([V3SwapExactIn, Transfer, WrapEth])"
                            .to_string(),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "3 commands".to_string(),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "V3SwapExactIn input: 0x0102".to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "V3SwapExactIn".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Input: 0x0102".to_string(),
                                        }),
                                        condensed: None,
                                        expanded: None,
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "Transfer input: 0x030405".to_string(),
                                        label: "Command 2".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "Transfer".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Input: 0x030405".to_string(),
                                        }),
                                        condensed: None,
                                        expanded: None,
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "WrapEth input: 0x06".to_string(),
                                        label: "Command 3".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "WrapEth".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Input: 0x06".to_string(),
                                        }),
                                        condensed: None,
                                        expanded: None,
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                        ],
                    }),
                },
            }
        );
    }

    #[test]
    fn test_visualize_tx_commands_command_without_input() {
        // Only one command, but no input for it
        let commands = vec![Command::Sweep as u8];
        let inputs = vec![]; // No input
        let deadline = 1_700_000_000u64;
        let input = encode_execute_call(&commands, inputs.clone(), deadline);

        let dt = chrono::Utc.timestamp_opt(deadline as i64, 0).unwrap();
        let deadline_str = dt.to_string();

        assert_eq!(
            UniversalRouterVisualizer {}
                .visualize_tx_commands(&input, 1, None)
                .unwrap(),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "Universal Router Execute: 1 commands ([Sweep]), deadline {deadline_str}",
                    ),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: format!("1 commands, deadline {deadline_str}"),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "Sweep input: None".to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "Sweep".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Input: None".to_string(),
                                        }),
                                        condensed: None,
                                        expanded: None,
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::TextV2 {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: deadline_str.clone(),
                                        label: "Deadline".to_string(),
                                    },
                                    text_v2: SignablePayloadFieldTextV2 {
                                        text: deadline_str.clone(),
                                    },
                                },
                                static_annotation: None,
                                dynamic_annotation: None,
                            },
                        ],
                    }),
                },
            }
        );
    }

    #[test]
    fn test_visualize_tx_commands_unrecognized_command() {
        // 0xff is not a valid Command, so it should be skipped
        let commands = vec![0xff, Command::Transfer as u8];
        let inputs = vec![vec![0x01], vec![0x02]];
        let deadline = 0u64;
        let input = encode_execute_call(&commands, inputs.clone(), deadline);

        assert_eq!(
            UniversalRouterVisualizer {}
                .visualize_tx_commands(&input, 1, None)
                .unwrap(),
            SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Universal Router Execute: 1 commands ([Transfer])".to_string(),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "1 commands".to_string(),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![AnnotatedPayloadField {
                            signable_payload_field: SignablePayloadField::PreviewLayout {
                                common: SignablePayloadFieldCommon {
                                    fallback_text: "Transfer input: 0x01".to_string(),
                                    label: "Command 1".to_string(),
                                },
                                preview_layout: SignablePayloadFieldPreviewLayout {
                                    title: Some(SignablePayloadFieldTextV2 {
                                        text: "Transfer".to_string(),
                                    }),
                                    subtitle: Some(SignablePayloadFieldTextV2 {
                                        text: "Input: 0x01".to_string(),
                                    }),
                                    condensed: None,
                                    expanded: None,
                                },
                            },
                            static_annotation: None,
                            dynamic_annotation: None,
                        }],
                    }),
                },
            }
        );
    }
}
