use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall as _, SolType, SolValue, sol};
use chrono::{TimeZone, Utc};
use num_enum::TryFromPrimitive;
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

use crate::registry::{ContractRegistry, ContractType};

// Uniswap Universal Router interface definitions
//
// Official Documentation:
// - Technical Reference: https://docs.uniswap.org/contracts/universal-router/technical-reference
// - Contract Source: https://github.com/Uniswap/universal-router/blob/main/contracts/interfaces/IUniversalRouter.sol
//
// The Universal Router supports function overloading with two execute variants:
// 1. execute(bytes,bytes[],uint256) - with deadline parameter for time-bound execution
// 2. execute(bytes,bytes[]) - without deadline for flexible execution
//
// Each function gets a unique 4-byte selector based on its signature.
sol! {
    interface IUniversalRouter {
        /// @notice Executes encoded commands along with provided inputs. Reverts if deadline has expired.
        /// @param commands A set of concatenated commands, each 1 byte in length
        /// @param inputs An array of byte strings containing abi encoded inputs for each command
        /// @param deadline The deadline by which the transaction must be executed
        function execute(bytes calldata commands, bytes[] calldata inputs, uint256 deadline) external payable;

        /// @notice Executes encoded commands along with provided inputs (no deadline check)
        /// @param commands A set of concatenated commands, each 1 byte in length
        /// @param inputs An array of byte strings containing abi encoded inputs for each command
        function execute(bytes calldata commands, bytes[] calldata inputs) external payable;
    }
}

// Command parameter structures
//
// These structs define the ABI-encoded parameters for each command type.
// Reference: https://docs.uniswap.org/contracts/universal-router/technical-reference
// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/modules/uniswap/v3/V3SwapRouter.sol
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

    /// Parameters for V2_SWAP_EXACT_IN command
    /// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/modules/uniswap/v2/V2SwapRouter.sol
    /// function v2SwapExactInput(address recipient, uint256 amountIn, uint256 amountOutMinimum, address[] calldata path, address payer)
    struct V2SwapExactInputParams {
        address recipient;
        uint256 amountIn;
        uint256 amountOutMinimum;
        address[] path;
        address payer;
    }

    /// Parameters for V2_SWAP_EXACT_OUT command
    struct V2SwapExactOutputParams {
        uint256 amountOut;
        uint256 amountInMaximum;
        address[] path;
        address recipient;
    }

    /// Parameters for WRAP_ETH command
    struct WrapEthParams {
        uint256 amountMin;
    }

    /// Parameters for SWEEP command
    struct SweepParams {
        address token;
        uint256 amountMinimum;
        address recipient;
    }

    /// Parameters for TRANSFER command
    struct TransferParams {
        address from;
        address to;
        uint160 amount;
    }

    /// Parameters for PERMIT2_TRANSFER_FROM command
    struct Permit2TransferFromParams {
        address from;
        address to;
        uint160 amount;
        address token;
    }

    /// Parameters for PERMIT2_PERMIT command
    struct PermitDetails {
        address token;
        uint160 amount;
        uint48 expiration;
        uint48 nonce;
    }

    struct PermitSingle {
        PermitDetails details;
        address spender;
        uint256 sigDeadline;
    }

    struct Permit2PermitParams {
        PermitSingle permitSingle;
        bytes signature;
    }
}

// Command IDs for Universal Router
//
// Reference: https://docs.uniswap.org/contracts/universal-router/technical-reference
// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Commands.sol
//
// Commands are encoded as single bytes and define the operation to execute.
// The Universal Router processes these commands sequentially.
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
    /// Visualizes Uniswap Universal Router Execute commands
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

        // Try decoding with deadline first (3-parameter version)
        if let Ok(call) = IUniversalRouter::execute_0Call::abi_decode(input) {
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
            return Self::visualize_commands(
                &call.commands.0,
                &call.inputs,
                deadline,
                chain_id,
                registry,
            );
        }

        // Try decoding without deadline (2-parameter version)
        if let Ok(call) = IUniversalRouter::execute_1Call::abi_decode(input) {
            return Self::visualize_commands(
                &call.commands.0,
                &call.inputs,
                None,
                chain_id,
                registry,
            );
        }

        None
    }

    /// Helper function to visualize commands (shared by both execute variants)
    fn visualize_commands(
        commands: &[u8],
        inputs: &[alloy_primitives::Bytes],
        deadline: Option<String>,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let mapped = map_commands(commands);
        let mut detail_fields = Vec::new();

        for (i, cmd) in mapped.iter().enumerate() {
            let input_bytes = inputs.get(i).map(|b| &b.0[..]);

            // Decode command-specific parameters
            let field = if let Some(bytes) = input_bytes {
                match cmd {
                    Command::V3SwapExactIn => {
                        Self::decode_v3_swap_exact_in(bytes, chain_id, registry)
                    }
                    Command::V3SwapExactOut => {
                        Self::decode_v3_swap_exact_out(bytes, chain_id, registry)
                    }
                    Command::V2SwapExactIn => {
                        Self::decode_v2_swap_exact_in(bytes, chain_id, registry)
                    }
                    Command::V2SwapExactOut => {
                        Self::decode_v2_swap_exact_out(bytes, chain_id, registry)
                    }
                    Command::PayPortion => Self::decode_pay_portion(bytes, chain_id, registry),
                    Command::WrapEth => Self::decode_wrap_eth(bytes, chain_id, registry),
                    Command::UnwrapWeth => Self::decode_unwrap_weth(bytes, chain_id, registry),
                    Command::Sweep => Self::decode_sweep(bytes, chain_id, registry),
                    Command::Transfer => Self::decode_transfer(bytes, chain_id, registry),
                    Command::Permit2TransferFrom => {
                        Self::decode_permit2_transfer_from(bytes, chain_id, registry)
                    }
                    Command::Permit2Permit => {
                        Self::decode_permit2_permit(bytes, chain_id, registry)
                    }
                    _ => {
                        // For unimplemented commands, show hex
                        let input_hex = format!("0x{}", hex::encode(bytes));
                        SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: format!("{cmd:?} input: {input_hex}"),
                                label: format!("{:?}", cmd),
                            },
                            text_v2: SignablePayloadFieldTextV2 {
                                text: format!("Input: {input_hex}"),
                            },
                        }
                    }
                }
            } else {
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{cmd:?} input: None"),
                        label: format!("{:?}", cmd),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Input: None".to_string(),
                    },
                }
            };

            // Wrap the field in a PreviewLayout for consistency
            let label = format!("Command {}", i + 1);
            let wrapped_field = match field {
                SignablePayloadField::TextV2 { common, text_v2 } => {
                    SignablePayloadField::PreviewLayout {
                        common: SignablePayloadFieldCommon {
                            fallback_text: common.fallback_text,
                            label,
                        },
                        preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                            title: Some(visualsign::SignablePayloadFieldTextV2 {
                                text: common.label,
                            }),
                            subtitle: Some(text_v2),
                            condensed: None,
                            expanded: None,
                        },
                    }
                }
                _ => field,
            };

            detail_fields.push(wrapped_field);
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

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: if let Some(dl) = &deadline {
                    format!(
                        "Uniswap Universal Router Execute: {} commands ({:?}), deadline {}",
                        mapped.len(),
                        mapped,
                        dl
                    )
                } else {
                    format!(
                        "Uniswap Universal Router Execute: {} commands ({:?})",
                        mapped.len(),
                        mapped
                    )
                },
                label: "Universal Router".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Uniswap Universal Router Execute".to_string(),
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
        })
    }

    /// Decodes V3_SWAP_EXACT_IN command parameters
    /// Uses abi_decode_params for proper ABI decoding of raw calldata bytes
    fn decode_v3_swap_exact_in(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Define the parameter types for V3SwapExactIn
        // (address recipient, uint256 amountIn, uint256 amountOutMinimum, bytes path, bool payerIsUser)
        type V3SwapParams = (Address, U256, U256, Bytes, bool);

        // Decode the ABI-encoded parameters
        let params = match V3SwapParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("V3 Swap Exact In: 0x{}", hex::encode(bytes)),
                        label: "V3 Swap Exact In".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let (_recipient, amount_in, amount_out_min, path, _payer_is_user) = params;

        // Validate path length (minimum 43 bytes for single hop: token + fee + token)
        if path.len() < 43 {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "V3 Swap Exact In: Invalid path".to_string(),
                    label: "V3 Swap Exact In".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("Path length: {} bytes (expected >=43)", path.len()),
                },
            };
        }

        // Extract token addresses and fee from path
        let token_in = Address::from_slice(&path[0..20]);
        let fee = u32::from_be_bytes([0, path[20], path[21], path[22]]);
        let token_out = Address::from_slice(&path[23..43]);

        // Resolve token symbols
        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{:?}", token_in));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{:?}", token_out));

        // Format amounts
        let amount_in_u128: u128 = amount_in.to_string().parse().unwrap_or(0);
        let amount_out_min_u128: u128 = amount_out_min.to_string().parse().unwrap_or(0);

        let (amount_in_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_in, amount_in_u128))
            .unwrap_or_else(|| (amount_in.to_string(), token_in_symbol.clone()));

        let (amount_out_min_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_out, amount_out_min_u128))
            .unwrap_or_else(|| (amount_out_min.to_string(), token_out_symbol.clone()));

        // Calculate fee percentage
        let fee_pct = fee as f64 / 10000.0;
        let text = format!(
            "Swap {} {} for >={} {} via V3 ({}% fee)",
            amount_in_str, token_in_symbol, amount_out_min_str, token_out_symbol, fee_pct
        );

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_in_symbol.clone(),
                        label: "Input Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_in_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_in_str.clone(),
                        label: "Input Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_in_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_out_symbol.clone(),
                        label: "Output Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_out_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(">={}", amount_out_min_str),
                        label: "Minimum Output".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(">={}", amount_out_min_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{}%", fee_pct),
                        label: "Fee Tier".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{}%", fee_pct),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "V3 Swap Exact In".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "V3 Swap Exact In".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes PAY_PORTION command parameters
    fn decode_pay_portion(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <PayPortionParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Pay Portion: 0x{}", hex::encode(bytes)),
                        label: "Pay Portion".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, params.token))
            .unwrap_or_else(|| format!("{:?}", params.token));

        // Convert bips to percentage (10000 bips = 100%)
        let bips_value: u128 = params.bips.to_string().parse().unwrap_or(0);
        let bips_pct = (bips_value as f64) / 100.0;
        let percentage_str = if bips_pct >= 1.0 {
            format!("{:.2}%", bips_pct)
        } else {
            format!("{:.4}%", bips_pct)
        };

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_symbol.clone(),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: percentage_str.clone(),
                        label: "Percentage".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: percentage_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.recipient),
                        label: "Recipient".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.recipient),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        let text = format!(
            "Pay {} of {} to {}",
            percentage_str, token_symbol, params.recipient
        );

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Pay Portion".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Pay Portion".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes UNWRAP_WETH command parameters
    fn decode_unwrap_weth(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <UnwrapWethParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Unwrap WETH: 0x{}", hex::encode(bytes)),
                        label: "Unwrap WETH".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        // Get WETH address for this chain and format the amount
        // WETH is registered in the token registry via UniswapConfig::register_common_tokens
        let amount_min_str =
            crate::protocols::uniswap::config::UniswapConfig::weth_address(chain_id)
                .and_then(|weth_addr| {
                    let amount_min_u128: u128 =
                        params.amountMinimum.to_string().parse().unwrap_or(0);
                    registry
                        .and_then(|r| r.format_token_amount(chain_id, weth_addr, amount_min_u128))
                })
                .map(|(amt, _)| amt)
                .unwrap_or_else(|| params.amountMinimum.to_string());

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_min_str.clone(),
                        label: "Minimum Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(">={} WETH", amount_min_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.recipient),
                        label: "Recipient".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.recipient),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        let text = format!(
            "Unwrap >={} WETH to ETH for {}",
            amount_min_str, params.recipient
        );

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Unwrap WETH".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Unwrap WETH".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes V3_SWAP_EXACT_OUT command parameters
    /// Uses abi_decode_params for proper ABI decoding of raw calldata bytes
    fn decode_v3_swap_exact_out(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Define the parameter types for V3SwapExactOut
        // (address recipient, uint256 amountOut, uint256 amountInMaximum, bytes path, bool payerIsUser)
        type V3SwapOutParams = (Address, U256, U256, Bytes, bool);

        // Decode the ABI-encoded parameters
        let params = match V3SwapOutParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("V3 Swap Exact Out: 0x{}", hex::encode(bytes)),
                        label: "V3 Swap Exact Out".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let (_recipient, amount_out, amount_in_max, path, _payer_is_user) = params;

        // Validate path length (minimum 43 bytes for single hop: token + fee + token)
        if path.len() < 43 {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "V3 Swap Exact Out: Invalid path".to_string(),
                    label: "V3 Swap Exact Out".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!("Path length: {} bytes (expected >=43)", path.len()),
                },
            };
        }

        // Extract token addresses and fee from path
        let token_in = Address::from_slice(&path[0..20]);
        let fee = u32::from_be_bytes([0, path[20], path[21], path[22]]);
        let token_out = Address::from_slice(&path[23..43]);

        // Resolve token symbols
        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{:?}", token_in));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{:?}", token_out));

        // Convert amounts to u128 for formatting
        let amount_out_u128: u128 = amount_out.to_string().parse().unwrap_or(0);
        let amount_in_max_u128: u128 = amount_in_max.to_string().parse().unwrap_or(0);

        // Format amounts with token decimals
        let (amount_out_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_out, amount_out_u128))
            .unwrap_or_else(|| (amount_out.to_string(), token_out_symbol.clone()));

        let (amount_in_max_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_in, amount_in_max_u128))
            .unwrap_or_else(|| (amount_in_max.to_string(), token_in_symbol.clone()));

        // Calculate fee percentage
        let fee_pct = fee as f64 / 10000.0;
        let text = format!(
            "Swap <={} {} for {} {} via V3 ({}% fee)",
            amount_in_max_str, token_in_symbol, amount_out_str, token_out_symbol, fee_pct
        );

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_in_symbol.clone(),
                        label: "Input Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_in_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("<={}", amount_in_max_str),
                        label: "Maximum Input".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("<={}", amount_in_max_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_out_symbol.clone(),
                        label: "Output Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_out_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_out_str.clone(),
                        label: "Output Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_out_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{}%", fee_pct),
                        label: "Fee Tier".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{}%", fee_pct),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "V3 Swap Exact Out".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "V3 Swap Exact Out".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes V2_SWAP_EXACT_IN command parameters
    /// (address recipient, uint256 amountIn, uint256 amountOutMinimum, address[] path, address payerIsUser)
    fn decode_v2_swap_exact_in(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        use alloy_sol_types::sol_data;

        type V2SwapParams = (
            sol_data::Address,
            sol_data::Uint<256>,
            sol_data::Uint<256>,
            sol_data::Array<sol_data::Address>,
            sol_data::Address,
        );

        let params = match V2SwapParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("V2 Swap Exact In: 0x{}", hex::encode(bytes)),
                        label: "V2 Swap Exact In".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let (_recipient, amount_in, amount_out_minimum, path_array, _payer) = params;
        let path = path_array.as_slice();

        if path.is_empty() {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "V2 Swap Exact In: Empty path".to_string(),
                    label: "V2 Swap Exact In".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Swap path is empty".to_string(),
                },
            };
        }

        let token_in = path[0];
        let token_out = path[path.len() - 1];

        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{:?}", token_in));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{:?}", token_out));

        let amount_in_u128: u128 = amount_in.to_string().parse().unwrap_or(0);
        let amount_out_min_u128: u128 = amount_out_minimum.to_string().parse().unwrap_or(0);

        let (amount_in_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_in, amount_in_u128))
            .unwrap_or_else(|| (amount_in.to_string(), token_in_symbol.clone()));

        let (amount_out_min_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_out, amount_out_min_u128))
            .unwrap_or_else(|| (amount_out_minimum.to_string(), token_out_symbol.clone()));

        let hops = path.len() - 1;
        let text = format!(
            "Swap {} {} for >={} {} via V2 ({} hops)",
            amount_in_str, token_in_symbol, amount_out_min_str, token_out_symbol, hops
        );

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_in_symbol.clone(),
                        label: "Input Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_in_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_in_str.clone(),
                        label: "Input Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_in_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_out_symbol.clone(),
                        label: "Output Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_out_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(">={}", amount_out_min_str),
                        label: "Minimum Output".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(">={}", amount_out_min_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: hops.to_string(),
                        label: "Hops".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: hops.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "V2 Swap Exact In".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "V2 Swap Exact In".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes V2_SWAP_EXACT_OUT command parameters
    /// (uint256 amountOut, uint256 amountInMaximum, address[] path, address recipient)
    fn decode_v2_swap_exact_out(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        use alloy_sol_types::sol_data;

        type V2SwapOutParams = (
            sol_data::Uint<256>,
            sol_data::Uint<256>,
            sol_data::Array<sol_data::Address>,
            sol_data::Address,
        );

        let params = match V2SwapOutParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("V2 Swap Exact Out: 0x{}", hex::encode(bytes)),
                        label: "V2 Swap Exact Out".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let (amount_out, amount_in_maximum, path_array, _recipient) = params;
        let path = path_array.as_slice();

        if path.is_empty() {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "V2 Swap Exact Out: Empty path".to_string(),
                    label: "V2 Swap Exact Out".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Swap path is empty".to_string(),
                },
            };
        }

        let token_in = path[0];
        let token_out = path[path.len() - 1];

        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{:?}", token_in));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{:?}", token_out));

        let amount_out_u128: u128 = amount_out.to_string().parse().unwrap_or(0);
        let amount_in_max_u128: u128 = amount_in_maximum.to_string().parse().unwrap_or(0);

        let (amount_out_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_out, amount_out_u128))
            .unwrap_or_else(|| (amount_out.to_string(), token_out_symbol.clone()));

        let (amount_in_max_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_in, amount_in_max_u128))
            .unwrap_or_else(|| (amount_in_maximum.to_string(), token_in_symbol.clone()));

        let hops = path.len() - 1;
        let text = format!(
            "Swap <={} {} for {} {} via V2 ({} hops)",
            amount_in_max_str, token_in_symbol, amount_out_str, token_out_symbol, hops
        );

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_in_symbol.clone(),
                        label: "Input Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_in_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("<={}", amount_in_max_str),
                        label: "Maximum Input".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("<={}", amount_in_max_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_out_symbol.clone(),
                        label: "Output Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_out_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_out_str.clone(),
                        label: "Output Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_out_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: hops.to_string(),
                        label: "Hops".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: hops.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "V2 Swap Exact Out".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "V2 Swap Exact Out".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes WRAP_ETH command parameters
    fn decode_wrap_eth(
        bytes: &[u8],
        _chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <WrapEthParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Wrap ETH: 0x{}", hex::encode(bytes)),
                        label: "Wrap ETH".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let amount_min_str = params.amountMin.to_string();
        let text = format!("Wrap {} ETH to WETH", amount_min_str);

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Wrap ETH".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes SWEEP command parameters
    fn decode_sweep(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <SweepParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Sweep: 0x{}", hex::encode(bytes)),
                        label: "Sweep".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, params.token))
            .unwrap_or_else(|| format!("{:?}", params.token));

        let text = format!(
            "Sweep >={} {} to {:?}",
            params.amountMinimum, token_symbol, params.recipient
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Sweep".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes TRANSFER command parameters
    fn decode_transfer(
        bytes: &[u8],
        _chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <TransferParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Transfer: 0x{}", hex::encode(bytes)),
                        label: "Transfer".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let text = format!(
            "Transfer {} tokens from {:?} to {:?}",
            params.amount, params.from, params.to
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Transfer".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes PERMIT2_TRANSFER_FROM command parameters
    fn decode_permit2_transfer_from(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match <Permit2TransferFromParams as SolValue>::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Permit2 Transfer From: 0x{}", hex::encode(bytes)),
                        label: "Permit2 Transfer From".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, params.token))
            .unwrap_or_else(|| format!("{:?}", params.token));

        // Format amount with proper decimals
        let amount_u128: u128 = params.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, params.token, amount_u128))
            .unwrap_or_else(|| (params.amount.to_string(), token_symbol.clone()));

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_symbol.clone(),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_str.clone(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.from),
                        label: "From".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.from),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.to),
                        label: "To".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.to),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        let summary = format!(
            "Transfer {} {} from {} to {}",
            amount_str, token_symbol, params.from, params.to
        );

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Permit2 Transfer From".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Permit2 Transfer From".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes PERMIT2_PERMIT (0x0a) command parameters
    /// The Uniswap Universal Router uses custom encoding (not standard ABI) for Permit2 commands:
    /// - Slots 0-5 (192 bytes): Raw PermitSingle struct data (inline, no ABI offsets)
    /// - Slots 6+: ABI-encoded bytes signature
    fn decode_permit2_permit(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Try standard ABI decoding first
        let decode_result = <Permit2PermitParams as SolValue>::abi_decode(bytes);

        let params = match decode_result {
            Ok(p) => p,
            Err(err) => {
                // Try custom encoding layout
                match Self::decode_custom_permit2_params(bytes) {
                    Ok(p) => p,
                    Err(_) => {
                        // Both attempts failed, show diagnostic info
                        return Self::show_decode_error(bytes, &err);
                    }
                }
            }
        };

        let token = params.permitSingle.details.token;
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token))
            .unwrap_or_else(|| format!("{:?}", token));

        // Format amount with proper decimals
        // Check if amount is unlimited (all 0xfff... = max uint160 or max uint256)
        let amount_str_val = params.permitSingle.details.amount.to_string();
        let is_unlimited = amount_str_val == "1461501637330902918203684832716283019655932542975" || // MAX_UINT160
            amount_str_val == "115792089237316195423570985008687907853269984665640564039457584007913129639935"; // MAX_UINT256

        let amount_u128: u128 = amount_str_val.parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token, amount_u128))
            .unwrap_or_else(|| (amount_str_val.clone(), token_symbol.clone()));

        // For condensed display, use "Unlimited Amount" if max value
        let display_amount_str = if is_unlimited {
            "Unlimited Amount".to_string()
        } else {
            amount_str.clone()
        };

        // Format expiration timestamp
        let expiration_u64: u64 = params
            .permitSingle
            .details
            .expiration
            .to_string()
            .parse()
            .unwrap_or(0);
        let expiration_str = if expiration_u64 == u64::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(expiration_u64 as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        // Format sig deadline timestamp
        let sig_deadline_u64: u64 = params
            .permitSingle
            .sigDeadline
            .to_string()
            .parse()
            .unwrap_or(0);
        let sig_deadline_str = if sig_deadline_u64 == u64::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(sig_deadline_u64 as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        // Create individual parameter fields
        let fields = vec![
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: token_symbol.clone(),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: token_symbol.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: amount_str.clone(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: amount_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.permitSingle.spender),
                        label: "Spender".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.permitSingle.spender),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: expiration_str.clone(),
                        label: "Expires".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: expiration_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: sig_deadline_str.clone(),
                        label: "Sig Deadline".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: sig_deadline_str.clone(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        let summary = format!(
            "Permit {} to spend {} of {}",
            params.permitSingle.spender, display_amount_str, token_symbol
        );

        // NOTE: The parameter encoding for PERMIT2_PERMIT command in Universal Router needs verification
        // The current decoding may not match the actual encoding used by the router
        // Values should be compared against Tenderly/Etherscan traces for accuracy

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Permit2 Permit".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Permit2 Permit".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }

    /// Decodes custom Permit2 parameter layout used by Uniswap router
    /// The Universal Router uses a custom encoding for Permit2 commands:
    /// Slots 0-5 (192 bytes): Raw PermitSingle structure (inline, no ABI offsets)
    /// Slots 6+: ABI-encoded bytes signature
    ///
    /// Byte Layout (discovered through transaction analysis):
    /// Slot 0 (0-31):    token (address, left-padded with 12 bytes zero padding)
    /// Slot 1 (32-63):   amount (uint160, left-padded with 12 bytes zero padding)
    /// Slot 2 (64-95):   padding (28 bytes) + expiration (6 bytes, right-aligned)
    /// Slot 3 (96-127):  nonce/reserved (all zeros in observed transaction)
    /// Slot 4 (128-159): spender (address, left-padded with 12 bytes zero padding)
    /// Slot 5 (160-191): sigDeadline (uint256, left-padded, value in last bytes)
    fn decode_custom_permit2_params(
        bytes: &[u8],
    ) -> Result<Permit2PermitParams, Box<dyn std::error::Error>> {
        if bytes.len() < 192 {
            return Err("bytes too short for PermitSingle (need 192 bytes minimum)".into());
        }

        let permit_single_bytes = &bytes[0..192];

        // Extract token (address) from bytes 12-31 (left-padded in Slot 0)
        let token = Address::from_slice(&permit_single_bytes[12..32]);

        // Extract amount (uint160) from bytes 44-63 (left-padded in Slot 1)
        let amount_hex = hex::encode(&permit_single_bytes[44..64]);
        let amount = alloy_primitives::Uint::<160, 3>::from_str_radix(&amount_hex, 16)
            .map_err(|_| "Failed to parse amount")?;

        // Extract expiration (uint48) from bytes 90-95 (right-aligned in Slot 2)
        let expiration_hex = hex::encode(&permit_single_bytes[90..96]);
        let expiration = alloy_primitives::Uint::<48, 1>::from_str_radix(&expiration_hex, 16)
            .map_err(|_| "Failed to parse expiration")?;

        // Extract nonce (uint48) from bytes 96-101 (Slot 3, appears to be unused/zero)
        let nonce_hex = hex::encode(&permit_single_bytes[96..102]);
        let nonce = alloy_primitives::Uint::<48, 1>::from_str_radix(&nonce_hex, 16)
            .map_err(|_| "Failed to parse nonce")?;

        // Extract spender (address) from bytes 140-159 (left-padded in Slot 4)
        let spender = Address::from_slice(&permit_single_bytes[140..160]);

        // Extract sigDeadline (uint256) from bytes 160-191 (all of Slot 5)
        let sig_deadline_hex = hex::encode(&permit_single_bytes[160..192]);
        let sig_deadline = alloy_primitives::U256::from_str_radix(&sig_deadline_hex, 16)
            .map_err(|_| "Failed to parse sigDeadline")?;

        // Extract signature bytes starting at offset 192 (slot 6+)
        // These should be ABI-encoded as bytes: offset (32) | length (32) | data (variable)
        let signature = alloy_primitives::Bytes::default(); // Placeholder

        Ok(Permit2PermitParams {
            permitSingle: PermitSingle {
                details: PermitDetails {
                    token,
                    amount,
                    expiration,
                    nonce,
                },
                spender,
                sigDeadline: sig_deadline,
            },
            signature,
        })
    }

    /// Helper function to display decoding error with raw hex slots
    fn show_decode_error(bytes: &[u8], err: &dyn std::fmt::Display) -> SignablePayloadField {
        let hex_data = format!("0x{}", hex::encode(bytes));
        let chunk_size = 32;
        let mut fields = vec![];

        for (i, chunk) in bytes.chunks(chunk_size).enumerate() {
            let chunk_hex = format!("0x{}", hex::encode(chunk));
            fields.push(visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: chunk_hex.clone(),
                        label: format!("Slot {}", i),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: chunk_hex },
                },
                static_annotation: None,
                dynamic_annotation: None,
            });
        }

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: hex_data.clone(),
                label: "Permit2 Permit".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "Permit2 Permit (Failed to Decode)".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: format!("Error: {}, Length: {} bytes", err, bytes.len()),
                }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout { fields }),
            },
        }
    }
}

/// ContractVisualizer implementation for Uniswap Universal Router
pub struct UniversalRouterContractVisualizer {
    inner: UniversalRouterVisualizer,
}

impl UniversalRouterContractVisualizer {
    pub fn new() -> Self {
        Self {
            inner: UniversalRouterVisualizer {},
        }
    }
}

impl Default for UniversalRouterContractVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::visualizer::ContractVisualizer for UniversalRouterContractVisualizer {
    fn contract_type(&self) -> &str {
        crate::protocols::uniswap::config::UniswapUniversalRouter::short_type_id()
    }

    fn visualize(
        &self,
        context: &crate::context::VisualizerContext,
    ) -> Result<Option<Vec<visualsign::AnnotatedPayloadField>>, visualsign::vsptrait::VisualSignError>
    {
        let contract_registry = crate::registry::ContractRegistry::with_default_protocols();

        if let Some(field) = self.inner.visualize_tx_commands(
            &context.calldata,
            context.chain_id,
            Some(&contract_registry),
        ) {
            let annotated = visualsign::AnnotatedPayloadField {
                signable_payload_field: field,
                static_annotation: None,
                dynamic_annotation: None,
            };

            Ok(Some(vec![annotated]))
        } else {
            Ok(None)
        }
    }
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
        IUniversalRouter::execute_0Call {
            commands: Bytes::from(commands.to_vec()),
            inputs: inputs_bytes,
            deadline: U256::from(deadline),
        }
        .abi_encode()
    }

    #[test]
    fn test_visualize_tx_commands_empty_input() {
        assert_eq!(
            UniversalRouterVisualizer {}.visualize_tx_commands(&[], 1, None),
            None
        );
        assert_eq!(
            UniversalRouterVisualizer {}.visualize_tx_commands(&[0x01, 0x02, 0x03], 1, None),
            None
        );
    }

    #[test]
    fn test_visualize_tx_commands_invalid_deadline() {
        // deadline is not convertible to i64 (u64::MAX)
        let input = encode_execute_call(&[0x00], vec![vec![0x01, 0x02]], u64::MAX);
        assert_eq!(
            UniversalRouterVisualizer {}.visualize_tx_commands(&input, 1, None),
            None
        );
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
                        "Uniswap Universal Router Execute: 1 commands ([V3SwapExactIn]), deadline {deadline_str}"
                    ),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Uniswap Universal Router Execute".to_string(),
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
                                        fallback_text: "V3 Swap Exact In: 0xdeadbeef".to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "V3 Swap Exact In".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Failed to decode parameters".to_string(),
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
                        "Uniswap Universal Router Execute: 3 commands ([V3SwapExactIn, Transfer, WrapEth])"
                            .to_string(),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Uniswap Universal Router Execute".to_string(),
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
                                        fallback_text: "V3 Swap Exact In: 0x0102".to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "V3 Swap Exact In".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Failed to decode parameters".to_string(),
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
                                        fallback_text: "Transfer: 0x030405".to_string(),
                                        label: "Command 2".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "Transfer".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Failed to decode parameters".to_string(),
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
                                        fallback_text: "Wrap ETH: 0x06".to_string(),
                                        label: "Command 3".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "Wrap ETH".to_string(),
                                        }),
                                        subtitle: Some(SignablePayloadFieldTextV2 {
                                            text: "Failed to decode parameters".to_string(),
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
                        "Uniswap Universal Router Execute: 1 commands ([Sweep]), deadline {deadline_str}",
                    ),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Uniswap Universal Router Execute".to_string(),
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
    fn test_visualize_tx_commands_real_transaction() {
        // Real transaction from Etherscan with 4 commands:
        // 1. V3SwapExactIn (0x00)
        // 2. V3SwapExactIn (0x00)
        // 3. PayPortion (0x06)
        // 4. UnwrapWeth (0x0c)
        let registry = crate::registry::ContractRegistry::with_default_protocols();

        // Transaction input data (execute function call)
        let input_hex = "3593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000006918f83f00000000000000000000000000000000000000000000000000000000000000040000060c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000002c000000000000000000000000000000000000000000000000000000000000003400000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000d02ab486cedc00000000000000000000000000000000000000000000000000000000cb274a57755e600000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002be71bdfe1df69284f00ee185cf0d95d0c7680c0d4000bb8c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000340aad21b3b70000000000000000000000000000000000000000000000000000000032e42284d704100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002be71bdfe1df69284f00ee185cf0d95d0c7680c0d4002710c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c000000000000000000000000000000000000000000000000000000000000001900000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000fe0b6cdc4c628c0";
        let input = hex::decode(input_hex).unwrap();

        let result = UniversalRouterVisualizer {}.visualize_tx_commands(&input, 1, Some(&registry));
        assert!(result.is_some(), "Should decode transaction successfully");

        // Verify the result contains decoded information
        let field = result.unwrap();
        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = field
        {
            // Check that the fallback text mentions 4 commands
            assert!(
                common.fallback_text.contains("4 commands"),
                "Expected '4 commands' in: {}",
                common.fallback_text
            );

            // Check that expanded section exists
            assert!(
                preview_layout.expanded.is_some(),
                "Expected expanded section"
            );

            if let Some(list_layout) = preview_layout.expanded {
                // Should have 5 fields: 4 commands + 1 deadline
                assert_eq!(
                    list_layout.fields.len(),
                    5,
                    "Expected 5 fields (4 commands + deadline)"
                );

                // Print decoded commands to verify they're human-readable
                println!("\n=== Decoded Transaction ===");
                println!("Fallback text: {}", common.fallback_text);
                for (i, annotated_field) in list_layout.fields.iter().enumerate() {
                    match &annotated_field.signable_payload_field {
                        SignablePayloadField::PreviewLayout {
                            common: field_common,
                            preview_layout: field_preview,
                        } => {
                            println!("\nCommand {}: {}", i + 1, field_common.label);
                            if let Some(title) = &field_preview.title {
                                println!("  Title: {}", title.text);
                            }
                            if let Some(subtitle) = &field_preview.subtitle {
                                println!("  Detail: {}", subtitle.text);

                                // Verify that decoded commands contain tokens, amounts, or decode failures
                                if i < 2 {
                                    // First two are swaps - should mention WETH, address, or decode failure
                                    assert!(
                                        subtitle.text.contains("WETH")
                                            || subtitle.text.contains("0x")
                                            || subtitle.text.contains("Failed to decode"),
                                        "Swap command should mention WETH, token address, or decode failure"
                                    );
                                }
                            }
                        }
                        SignablePayloadField::TextV2 {
                            common: field_common,
                            text_v2,
                        } => {
                            println!("\n{}: {}", field_common.label, text_v2.text);
                        }
                        _ => {}
                    }
                }
                println!("\n=== End Decoded Transaction ===\n");
            }
        } else {
            panic!("Expected PreviewLayout, got different field type");
        }
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
                    fallback_text: "Uniswap Universal Router Execute: 1 commands ([Transfer])"
                        .to_string(),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Uniswap Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "1 commands".to_string(),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![AnnotatedPayloadField {
                            signable_payload_field: SignablePayloadField::PreviewLayout {
                                common: SignablePayloadFieldCommon {
                                    fallback_text: "Transfer: 0x01".to_string(),
                                    label: "Command 1".to_string(),
                                },
                                preview_layout: SignablePayloadFieldPreviewLayout {
                                    title: Some(SignablePayloadFieldTextV2 {
                                        text: "Transfer".to_string(),
                                    }),
                                    subtitle: Some(SignablePayloadFieldTextV2 {
                                        text: "Failed to decode parameters".to_string(),
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

    #[test]
    fn test_decode_permit2_permit_custom_decoder() {
        // Unit test for the custom Permit2 Permit decoder
        // This tests the byte-level decoding without going through ABI

        // Construct a minimal PermitSingle structure (192 bytes)
        let mut permit_single = vec![0u8; 192];

        // Set token at bytes 12-31 (Slot 0, left-padded address)
        let token_bytes = hex::decode("72b658bd674f9c2b4954682f517c17d14476e417").unwrap();
        permit_single[0..12].fill(0); // Clear padding
        permit_single[12..32].copy_from_slice(&token_bytes);

        // Set amount at bytes 44-63 (Slot 1, max uint160, left-padded)
        let amount_bytes = hex::decode("ffffffffffffffffffffffffffffffffffffffff").unwrap();
        permit_single[32..44].fill(0); // Clear padding for slot 1
        permit_single[44..64].copy_from_slice(&amount_bytes);

        // Set expiration at bytes 90-95 (Slot 2, 1765824281 = 0x69405719)
        permit_single[90..96].copy_from_slice(&[0u8, 0, 0x69, 0x40, 0x57, 0x19]);

        // Set spender at bytes 140-159 (Slot 4, left-padded address)
        let spender_bytes = hex::decode("3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad").unwrap();
        permit_single[128..140].fill(0); // Clear padding for slot 4
        permit_single[140..160].copy_from_slice(&spender_bytes);

        // Set sigDeadline at bytes 160-191 (Slot 5, 1763234081 = 0x6918d121)
        permit_single[160..188].copy_from_slice(&[0u8; 28]);
        permit_single[188..192].copy_from_slice(&[0x69, 0x18, 0xd1, 0x21]);

        let result = UniversalRouterVisualizer::decode_custom_permit2_params(&permit_single);
        assert!(
            result.is_ok(),
            "Should decode custom permit2 params successfully"
        );

        let params = result.unwrap();

        // Verify token
        let expected_token: Address = "0x72b658bd674f9c2b4954682f517c17d14476e417"
            .parse()
            .unwrap();
        assert_eq!(params.permitSingle.details.token, expected_token);

        // Verify amount (max uint160)
        let expected_amount = alloy_primitives::Uint::<160, 3>::from_str_radix(
            "ffffffffffffffffffffffffffffffffffffffff",
            16,
        )
        .unwrap();
        assert_eq!(params.permitSingle.details.amount, expected_amount);

        // Verify expiration
        let expected_expiration = alloy_primitives::Uint::<48, 1>::from(1765824281u64);
        assert_eq!(params.permitSingle.details.expiration, expected_expiration);

        // Verify spender
        let expected_spender: Address = "0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad"
            .parse()
            .unwrap();
        assert_eq!(params.permitSingle.spender, expected_spender);

        // Verify sigDeadline
        let expected_sig_deadline = alloy_primitives::U256::from(1763234081u64);
        assert_eq!(params.permitSingle.sigDeadline, expected_sig_deadline);
    }

    #[test]
    fn test_decode_permit2_permit_field_visualization() {
        // Unit test for Permit2 Permit field visualization
        let registry = ContractRegistry::with_default_protocols();

        // Construct the same PermitSingle structure
        let mut permit_single = vec![0u8; 192];

        let token_bytes = hex::decode("72b658bd674f9c2b4954682f517c17d14476e417").unwrap();
        permit_single[0..12].fill(0);
        permit_single[12..32].copy_from_slice(&token_bytes);

        let amount_bytes = hex::decode("ffffffffffffffffffffffffffffffffffffffff").unwrap();
        permit_single[32..44].fill(0);
        permit_single[44..64].copy_from_slice(&amount_bytes);

        permit_single[90..96].copy_from_slice(&[0u8, 0, 0x69, 0x40, 0x57, 0x19]);

        let spender_bytes = hex::decode("3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad").unwrap();
        permit_single[128..140].fill(0);
        permit_single[140..160].copy_from_slice(&spender_bytes);

        permit_single[160..188].copy_from_slice(&[0u8; 28]);
        permit_single[188..192].copy_from_slice(&[0x69, 0x18, 0xd1, 0x21]);

        let field =
            UniversalRouterVisualizer::decode_permit2_permit(&permit_single, 1, Some(&registry));

        // Verify the field is a PreviewLayout
        match field {
            SignablePayloadField::PreviewLayout { common, .. } => {
                // Check the label
                assert_eq!(common.label, "Permit2 Permit");
            }
            _ => panic!("Expected PreviewLayout, got different field type"),
        }
    }

    #[test]
    fn test_permit2_permit_integration_with_fixture_transaction() {
        // Integration test using the actual transaction fixture provided by the user
        // The user provided a full EIP-1559 transaction, but we can only test with the calldata
        let registry = ContractRegistry::with_default_protocols();

        // Extract just the execute() calldata from the transaction data
        let input_hex = "3593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000006918f83f00000000000000000000000000000000000000000000000000000000000000040a08060c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000032000000000000000000000000000000000000000000000000000000000000003a0000000000000000000000000000000000000000000000000000000000000016000000000000000000000000072b658bd674f9c2b4954682f517c17d14476e417000000000000000000000000ffffffffffffffffffffffffffffffffffffffff000000000000000000000000000000000000000000000000000000006940571900000000000000000000000000000000000000000000000000000000000000000000000000000000000000003fc91a3afd70395cd496c647d5a6cc9d4b2b7fad000000000000000000000000000000000000000000000000000000006918d12100000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000000412eb0933411b0970637515316fb50511bea7908d3f85808074ceed3bf881562bc06da5178104470e54fb5be96075169b30799c30f30975317ae14113ffdb84bc81c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000285aaa58c1a1a183d0000000000000000000000000000000000000000000000000009cf200e607a0800000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000072b658bd674f9c2b4954682f517c17d14476e417000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000000060000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c000000000000000000000000000000000000000000000000000000000000001900000000000000000000000000000000000000000000000000000000000000400000000000000000000000008419e7eda8577dfc49591a49cad965a0fc6716cf0000000000000000000000000000000000000000000000000009c8d8ef9ef49bc0";
        let input = hex::decode(input_hex).unwrap();

        let result = UniversalRouterVisualizer {}.visualize_tx_commands(&input, 1, Some(&registry));
        assert!(result.is_some(), "Should decode transaction successfully");

        let field = result.unwrap();

        // Verify the main transaction field
        match field {
            SignablePayloadField::PreviewLayout { common, .. } => {
                // Check that it mentions commands
                assert!(
                    common.fallback_text.contains("commands"),
                    "Expected 'commands' in fallback text: {}",
                    common.fallback_text
                );
            }
            _ => panic!("Expected PreviewLayout for main field"),
        }
    }

    #[test]
    fn test_permit2_permit_timestamp_boundaries() {
        // Test edge cases for timestamp handling
        let registry = ContractRegistry::with_default_protocols();
        let mut permit_single = vec![0u8; 192];

        let token_bytes = hex::decode("72b658bd674f9c2b4954682f517c17d14476e417").unwrap();
        permit_single[0..12].fill(0);
        permit_single[12..32].copy_from_slice(&token_bytes);

        let amount_bytes = hex::decode("ffffffffffffffffffffffffffffffffffffffff").unwrap();
        permit_single[32..44].fill(0);
        permit_single[44..64].copy_from_slice(&amount_bytes);

        let spender_bytes = hex::decode("3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad").unwrap();
        permit_single[128..140].fill(0);
        permit_single[140..160].copy_from_slice(&spender_bytes);

        // Test with a future timestamp (year 2030)
        // 1893456000 = Friday, January 1, 2030 2:40:00 AM
        permit_single[90..96].copy_from_slice(&[0u8, 0, 0x70, 0x94, 0x4b, 0x80]);
        permit_single[160..192].copy_from_slice(&[0u8; 32]);

        let field =
            UniversalRouterVisualizer::decode_permit2_permit(&permit_single, 1, Some(&registry));

        match field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                if let Some(expanded) = &preview_layout.expanded {
                    for f in &expanded.fields {
                        if let SignablePayloadField::PreviewLayout {
                            common,
                            preview_layout: inner_preview,
                        } = &f.signable_payload_field
                        {
                            if common.label.contains("Expires") {
                                if let Some(subtitle) = &inner_preview.subtitle {
                                    // Should show a valid date in 2030
                                    assert!(subtitle.text.contains("2030"));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[test]
    fn test_permit2_permit_invalid_input_too_short() {
        // Test that short input is properly rejected
        let short_input = vec![0u8; 100]; // Too short
        let result = UniversalRouterVisualizer::decode_custom_permit2_params(&short_input);
        assert!(
            result.is_err(),
            "Should reject input shorter than 192 bytes"
        );
    }

    #[test]
    fn test_permit2_permit_empty_input() {
        // Test that empty input is properly rejected
        let empty_input = vec![];
        let result = UniversalRouterVisualizer::decode_custom_permit2_params(&empty_input);
        assert!(result.is_err(), "Should reject empty input");
    }
}
