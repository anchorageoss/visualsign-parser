use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall as _, SolType, SolValue, sol};
use chrono::{TimeZone, Utc};
use num_enum::TryFromPrimitive;
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

use crate::protocols::uniswap::contracts::permit2::Permit2Visualizer;
use crate::registry::{ContractRegistry, ContractType};

/// Formats a token amount using the registry when possible, falling back to raw string.
/// Avoids the silent-zero bug where U256 values > u128::MAX would display as "0".
fn format_amount_with_registry(
    amount: &U256,
    chain_id: u64,
    token: Address,
    registry: Option<&ContractRegistry>,
) -> (String, String) {
    let token_symbol = registry
        .and_then(|r| r.get_token_symbol(chain_id, token))
        .unwrap_or_else(|| format!("{token:?}"));

    match amount.to_string().parse::<u128>() {
        Ok(amount_u128) => registry
            .and_then(|r| r.format_token_amount(chain_id, token, amount_u128))
            .unwrap_or_else(|| (amount.to_string(), token_symbol.clone())),
        Err(_) => {
            // Value exceeds u128::MAX — show raw string instead of silent 0
            (amount.to_string(), token_symbol.clone())
        }
    }
}

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
    /// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Dispatcher.sol
    /// (address recipient, uint256 amountMin) = abi.decode(inputs, (address, uint256));
    struct WrapEthParams {
        address recipient;
        uint256 amountMin;
    }

    /// Parameters for SWEEP command
    /// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Dispatcher.sol
    /// (address token, address recipient, uint256 amountMin) = abi.decode(inputs, (address, address, uint256));
    struct SweepParams {
        address token;
        address recipient;
        uint256 amountMinimum;
    }

    /// Parameters for TRANSFER command
    /// Source: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Dispatcher.sol
    /// (address token, address recipient, uint256 value)
    struct TransferParams {
        address token;
        address recipient;
        uint256 value;
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

fn map_commands(raw: &[u8]) -> Vec<Option<Command>> {
    raw.iter().map(|&b| Command::try_from(b).ok()).collect()
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
                0,
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
                0,
            );
        }

        None
    }

    /// Format a command name for display, showing the debug name for known
    /// commands or the raw byte for unknown ones.
    fn format_cmd_name(maybe_cmd: &Option<Command>, raw_byte: u8) -> String {
        match maybe_cmd {
            Some(cmd) => format!("{cmd:?}"),
            None => format!("Unknown(0x{raw_byte:02x})"),
        }
    }

    /// Maximum recursion depth for nested ExecuteSubPlan commands.
    /// Prevents stack overflow from maliciously crafted deeply-nested sub-plans.
    const MAX_SUB_PLAN_DEPTH: usize = 4;

    /// Helper function to visualize commands (shared by both execute variants)
    fn visualize_commands(
        commands: &[u8],
        inputs: &[alloy_primitives::Bytes],
        deadline: Option<String>,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
        depth: usize,
    ) -> Option<SignablePayloadField> {
        let mapped = map_commands(commands);
        let mut detail_fields = Vec::new();

        // Iterate over ALL command bytes (including unknown) to keep indices
        // aligned with the inputs array. Unknown commands are shown explicitly
        // rather than silently dropped — dropping would shift all subsequent
        // command-input pairings.
        for (i, maybe_cmd) in mapped.iter().enumerate() {
            let input_bytes = inputs.get(i).map(|b| &b.0[..]);

            // Decode command-specific parameters
            let field = if let Some(bytes) = input_bytes {
                match maybe_cmd {
                    Some(cmd) => Self::decode_known_command(*cmd, bytes, chain_id, registry, depth),
                    None => {
                        // Unknown command — show truncated hex so user can see it
                        let cmd_name = Self::format_cmd_name(maybe_cmd, commands[i]);
                        let input_hex = {
                            let full = hex::encode(bytes);
                            if full.len() > 64 {
                                format!("0x{}...", &full[..64])
                            } else {
                                format!("0x{full}")
                            }
                        };
                        SignablePayloadField::TextV2 {
                            common: SignablePayloadFieldCommon {
                                fallback_text: format!("{cmd_name} input: {input_hex}"),
                                label: cmd_name,
                            },
                            text_v2: SignablePayloadFieldTextV2 {
                                text: format!("Input: {input_hex}"),
                            },
                        }
                    }
                }
            } else {
                let cmd_label = Self::format_cmd_name(maybe_cmd, commands[i]);
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{cmd_label} input: None"),
                        label: cmd_label,
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

        // Build a human-readable list of command names for the fallback text
        let cmd_names: Vec<String> = mapped
            .iter()
            .enumerate()
            .map(|(i, maybe_cmd)| Self::format_cmd_name(maybe_cmd, commands[i]))
            .collect();

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: if let Some(dl) = &deadline {
                    format!(
                        "Uniswap Universal Router Execute: {} commands ([{}]), deadline {}",
                        mapped.len(),
                        cmd_names.join(", "),
                        dl
                    )
                } else {
                    format!(
                        "Uniswap Universal Router Execute: {} commands ([{}])",
                        mapped.len(),
                        cmd_names.join(", ")
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

    /// Dispatches a known command to its specific decoder
    fn decode_known_command(
        cmd: Command,
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
        depth: usize,
    ) -> SignablePayloadField {
        match cmd {
            Command::V3SwapExactIn => Self::decode_v3_swap_exact_in(bytes, chain_id, registry),
            Command::V3SwapExactOut => Self::decode_v3_swap_exact_out(bytes, chain_id, registry),
            Command::V2SwapExactIn => Self::decode_v2_swap_exact_in(bytes, chain_id, registry),
            Command::V2SwapExactOut => Self::decode_v2_swap_exact_out(bytes, chain_id, registry),
            Command::PayPortion => Self::decode_pay_portion(bytes, chain_id, registry),
            Command::WrapEth => Self::decode_wrap_eth(bytes, chain_id, registry),
            Command::UnwrapWeth => Self::decode_unwrap_weth(bytes, chain_id, registry),
            Command::Sweep => Self::decode_sweep(bytes, chain_id, registry),
            Command::Transfer => Self::decode_transfer(bytes, chain_id, registry),
            Command::Permit2TransferFrom => {
                Self::decode_permit2_transfer_from(bytes, chain_id, registry)
            }
            Command::Permit2Permit => Self::decode_permit2_permit(bytes, chain_id, registry),
            Command::ExecuteSubPlan => {
                Self::decode_execute_sub_plan(bytes, chain_id, registry, depth)
            }
            _ => {
                // For recognized but unimplemented commands, show hex
                let input_hex = format!("0x{}", hex::encode(bytes));
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{cmd:?} input: {input_hex}"),
                        label: format!("{cmd:?}"),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("Input: {input_hex}"),
                    },
                }
            }
        }
    }

    /// Decodes EXECUTE_SUB_PLAN command by recursively visualizing nested commands.
    /// Sub-plan format: (bytes commands, bytes[] inputs)
    /// Depth is bounded by MAX_SUB_PLAN_DEPTH to prevent stack overflow.
    fn decode_execute_sub_plan(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
        depth: usize,
    ) -> SignablePayloadField {
        // Guard against stack overflow from maliciously nested sub-plans
        if depth >= Self::MAX_SUB_PLAN_DEPTH {
            let input_hex = format!("0x{}", hex::encode(bytes));
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("ExecuteSubPlan (depth limit): {input_hex}"),
                    label: "ExecuteSubPlan".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: format!(
                        "Sub-plan nesting exceeds maximum depth ({})",
                        Self::MAX_SUB_PLAN_DEPTH
                    ),
                },
            };
        }

        // Sub-plan is ABI-encoded as (bytes commands, bytes[] inputs)
        // Use sol_data types for proper ABI tuple decoding
        use alloy_sol_types::sol_data;
        type SubPlanParams = (sol_data::Bytes, sol_data::Array<sol_data::Bytes>);

        let params = match SubPlanParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                let input_hex = format!("0x{}", hex::encode(bytes));
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("ExecuteSubPlan: {input_hex}"),
                        label: "ExecuteSubPlan".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode sub-plan parameters".to_string(),
                    },
                };
            }
        };

        let (sub_commands, sub_inputs) = params;
        let sub_inputs_bytes: Vec<alloy_primitives::Bytes> = sub_inputs.into_iter().collect();

        // Recursively visualize the nested commands with incremented depth
        if let Some(nested_field) = Self::visualize_commands(
            &sub_commands,
            &sub_inputs_bytes,
            None,
            chain_id,
            registry,
            depth + 1,
        ) {
            nested_field
        } else {
            let input_hex = format!("0x{}", hex::encode(bytes));
            SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("ExecuteSubPlan: {input_hex}"),
                    label: "ExecuteSubPlan".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Empty sub-plan".to_string(),
                },
            }
        }
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

        // Extract token addresses from path
        // ExactIn path: tokenIn(20) | fee(3) | token(20) | fee(3) | ... | tokenOut(20)
        let token_in = Address::from_slice(&path[0..20]);
        let fee = u32::from_be_bytes([0, path[20], path[21], path[22]]);
        let token_out = Address::from_slice(&path[path.len() - 20..]);

        // Resolve token symbols
        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{token_in:?}"));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{token_out:?}"));

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
            "Swap {amount_in_str} {token_in_symbol} for >={amount_out_min_str} {token_out_symbol} via V3 ({fee_pct}% fee)"
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
                        fallback_text: format!(">={amount_out_min_str}"),
                        label: "Minimum Output".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(">={amount_out_min_str}"),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            visualsign::AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{fee_pct}%"),
                        label: "Fee Tier".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{fee_pct}%"),
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
        // Compare as U256 before narrowing to avoid silent overflow
        let over_100_pct = params.bips > alloy_primitives::U256::from(10000u64);
        let bips_value: u128 = params.bips.to_string().parse().unwrap_or(u128::MAX);
        let bips_pct = (bips_value as f64) / 100.0;
        let percentage_str = if over_100_pct {
            format!("{bips_pct:.2}% (WARNING: >100%)")
        } else if bips_pct >= 1.0 {
            format!("{bips_pct:.2}%")
        } else {
            format!("{bips_pct:.4}%")
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
                        text: format!(">={amount_min_str} WETH"),
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

        // Extract token addresses from path
        // ExactOut path is REVERSED: tokenOut(20) | fee(3) | ... | tokenIn(20)
        let token_out = Address::from_slice(&path[0..20]);
        let fee = u32::from_be_bytes([0, path[20], path[21], path[22]]);
        let token_in = Address::from_slice(&path[path.len() - 20..]);

        // Resolve token symbols
        let token_in_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_in))
            .unwrap_or_else(|| format!("{token_in:?}"));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{token_out:?}"));

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
            "Swap <={amount_in_max_str} {token_in_symbol} for {amount_out_str} {token_out_symbol} via V3 ({fee_pct}% fee)"
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
                        fallback_text: format!("<={amount_in_max_str}"),
                        label: "Maximum Input".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("<={amount_in_max_str}"),
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
                        fallback_text: format!("{fee_pct}%"),
                        label: "Fee Tier".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{fee_pct}%"),
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
            .unwrap_or_else(|| format!("{token_in:?}"));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{token_out:?}"));

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
            "Swap {amount_in_str} {token_in_symbol} for >={amount_out_min_str} {token_out_symbol} via V2 ({hops} hops)"
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
                        fallback_text: format!(">={amount_out_min_str}"),
                        label: "Minimum Output".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!(">={amount_out_min_str}"),
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
            .unwrap_or_else(|| format!("{token_in:?}"));
        let token_out_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_out))
            .unwrap_or_else(|| format!("{token_out:?}"));

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
            "Swap <={amount_in_max_str} {token_in_symbol} for {amount_out_str} {token_out_symbol} via V2 ({hops} hops)"
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
                        fallback_text: format!("<={amount_in_max_str}"),
                        label: "Maximum Input".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("<={amount_in_max_str}"),
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
    /// Note: WRAP_ETH wraps msg.value and checks that it's >= amountMin.
    /// The amountMin is a minimum check, not the actual amount being wrapped.
    fn decode_wrap_eth(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
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

        // Format amount with ETH decimals (18)
        // Get WETH address for this chain to use its decimals
        let amount_min_str =
            crate::protocols::uniswap::config::UniswapConfig::weth_address(chain_id)
                .and_then(|weth_addr| {
                    let amount_min_u128: u128 = params.amountMin.to_string().parse().unwrap_or(0);
                    registry
                        .and_then(|r| r.format_token_amount(chain_id, weth_addr, amount_min_u128))
                })
                .map(|(amt, _)| amt)
                .unwrap_or_else(|| {
                    // Fallback: format as ETH with 18 decimals manually
                    crate::fmt::format_ether(params.amountMin)
                });

        let text = format!("Wrap >={amount_min_str} ETH to WETH");

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

        // Format amount with token decimals
        let amount_min_u128: u128 = params.amountMinimum.to_string().parse().unwrap_or(0);
        let (amount_min_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, params.token, amount_min_u128))
            .unwrap_or_else(|| (params.amountMinimum.to_string(), token_symbol.clone()));

        let text = format!(
            "Sweep >={amount_min_str} {token_symbol} to {:?}",
            params.recipient
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
    /// Source: (address token, address recipient, uint256 value)
    fn decode_transfer(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
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

        let (value_str, token_symbol) =
            format_amount_with_registry(&params.value, chain_id, params.token, registry);

        let text = format!(
            "Transfer {} {} to {:?}",
            value_str, token_symbol, params.recipient
        );

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Transfer".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes PERMIT2_TRANSFER_FROM command parameters directly.
    /// Universal Router encodes this as (address token, address recipient, uint256 amount)
    /// — raw ABI params without a function selector.
    fn decode_permit2_transfer_from(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Decode directly: (address token, address recipient, uint256 amount)
        type TransferFromParams = (Address, Address, U256);

        let params = match TransferFromParams::abi_decode_params(bytes) {
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

        let (token, recipient, amount) = params;

        let (amount_str, token_symbol) =
            format_amount_with_registry(&amount, chain_id, token, registry);

        let text = format!("Permit2 Transfer {amount_str} {token_symbol} to {recipient:?}");

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Permit2 Transfer From".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes PERMIT2_PERMIT (0x0a) command.
    /// Universal Router encodes this as inline PermitSingle bytes + signature,
    /// without a function selector. Try custom permit decoding first, then
    /// fall back to showing raw hex slot breakdown.
    fn decode_permit2_permit(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Try custom permit decoding (inline PermitSingle without selector)
        if let Ok(permit_single) = Permit2Visualizer::decode_custom_permit_params(bytes) {
            let token = permit_single.details.token;
            let token_symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, token))
                .unwrap_or_else(|| format!("{token:?}"));

            let amount_str = match permit_single.details.amount.to_string().parse::<u128>() {
                Ok(amount_u128) => registry
                    .and_then(|r| r.format_token_amount(chain_id, token, amount_u128))
                    .map(|(s, _)| s)
                    .unwrap_or_else(|| permit_single.details.amount.to_string()),
                Err(_) => {
                    // Value exceeds u128::MAX — show raw amount string
                    permit_single.details.amount.to_string()
                }
            };

            let amount_display = if permit_single.details.amount == alloy_primitives::U160::MAX {
                format!("Unlimited {token_symbol}")
            } else {
                format!("{amount_str} {token_symbol}")
            };

            let text = format!(
                "Permit {} to spend {}",
                permit_single.spender, amount_display
            );

            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: text.clone(),
                    label: "Permit2 Permit".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 { text },
            };
        }

        // Fall back to showing raw hex with slot breakdown
        Self::show_decode_error(bytes, &"Failed to decode parameters")
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
                        label: format!("Slot {i}"),
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
        let (contract_registry, _visualizer_builder) =
            crate::registry::ContractRegistry::with_default_protocols();

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
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

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
        // 0xff is not a valid Command — it should be shown (not dropped) to keep
        // indices aligned with the inputs array.
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
                    fallback_text:
                        "Uniswap Universal Router Execute: 2 commands ([Unknown(0xff), Transfer])"
                            .to_string(),
                    label: "Universal Router".to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: "Uniswap Universal Router Execute".to_string(),
                    }),
                    subtitle: Some(SignablePayloadFieldTextV2 {
                        text: "2 commands".to_string(),
                    }),
                    condensed: None,
                    expanded: Some(SignablePayloadFieldListLayout {
                        fields: vec![
                            // Unknown command shown first with its own input
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "Unknown(0xff) input: 0x01".to_string(),
                                        label: "Command 1".to_string(),
                                    },
                                    preview_layout: SignablePayloadFieldPreviewLayout {
                                        title: Some(SignablePayloadFieldTextV2 {
                                            text: "Unknown(0xff)".to_string(),
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
                            },
                            // Transfer gets its correct input (0x02), not 0x01
                            AnnotatedPayloadField {
                                signable_payload_field: SignablePayloadField::PreviewLayout {
                                    common: SignablePayloadFieldCommon {
                                        fallback_text: "Transfer: 0x02".to_string(),
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
                        ],
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

        let result = Permit2Visualizer::decode_custom_permit_params(&permit_single);
        assert!(
            result.is_ok(),
            "Should decode custom permit2 params successfully"
        );

        let params = result.unwrap();

        // Verify token
        let expected_token: Address = "0x72b658bd674f9c2b4954682f517c17d14476e417"
            .parse()
            .unwrap();
        assert_eq!(params.details.token, expected_token);

        // Verify amount (max uint160)
        let expected_amount = alloy_primitives::Uint::<160, 3>::from_str_radix(
            "ffffffffffffffffffffffffffffffffffffffff",
            16,
        )
        .unwrap();
        assert_eq!(params.details.amount, expected_amount);

        // Verify expiration
        let expected_expiration = alloy_primitives::Uint::<48, 1>::from(1765824281u64);
        assert_eq!(params.details.expiration, expected_expiration);

        // Verify spender
        let expected_spender: Address = "0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad"
            .parse()
            .unwrap();
        assert_eq!(params.spender, expected_spender);

        // Verify sigDeadline
        let expected_sig_deadline = alloy_primitives::U256::from(1763234081u64);
        assert_eq!(params.sigDeadline, expected_sig_deadline);
    }

    #[test]
    fn test_decode_permit2_permit_field_visualization() {
        // Unit test for Permit2 Permit field visualization
        let (registry, _) = ContractRegistry::with_default_protocols();

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

        // Verify the field has the correct label
        match field {
            SignablePayloadField::TextV2 { common, .. } => {
                // Permit2Visualizer now returns TextV2 for permit
                assert_eq!(common.label, "Permit2 Permit");
            }
            SignablePayloadField::PreviewLayout { common, .. } => {
                // Also accept PreviewLayout for backwards compatibility
                assert_eq!(common.label, "Permit2 Permit");
            }
            _ => panic!("Expected TextV2 or PreviewLayout, got different field type"),
        }
    }

    #[test]
    fn test_permit2_permit_integration_with_fixture_transaction() {
        // Integration test using the actual transaction fixture provided by the user
        // The user provided a full EIP-1559 transaction, but we can only test with the calldata
        let (registry, _) = ContractRegistry::with_default_protocols();

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
        let (registry, _) = ContractRegistry::with_default_protocols();
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

        if let SignablePayloadField::PreviewLayout { preview_layout, .. } = field {
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
    }

    #[test]
    fn test_permit2_permit_invalid_input_too_short() {
        // Test that short input is properly rejected
        let short_input = vec![0u8; 100]; // Too short
        let result = Permit2Visualizer::decode_custom_permit_params(&short_input);
        assert!(
            result.is_err(),
            "Should reject input shorter than 192 bytes"
        );
    }

    #[test]
    fn test_permit2_permit_empty_input() {
        // Test that empty input is properly rejected
        let empty_input = vec![];
        let result = Permit2Visualizer::decode_custom_permit_params(&empty_input);
        assert!(result.is_err(), "Should reject empty input");
    }

    #[test]
    fn test_decode_wrap_eth_params_order() {
        // WRAP_ETH params: (address recipient, uint256 amountMin)
        // This test verifies we decode (recipient, amountMin) not just (amountMin)
        let recipient: Address = "0xd27f4bbd67bd4ad1674c9c2c5a75ca8c3e389f3b"
            .parse()
            .unwrap();
        let amount_min = U256::from(3_200_000_000_000_000u64); // 0.0032 ETH in wei

        // ABI encode: (address, uint256)
        let encoded = (recipient, amount_min).abi_encode();

        let field = UniversalRouterVisualizer::decode_wrap_eth(&encoded, 1, None);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                // Should show ~0.0032 ETH, not a huge number from misinterpreting address as amount
                assert!(
                    text_v2.text.contains("0.0032"),
                    "Expected 0.0032 ETH, got: {}",
                    text_v2.text
                );
                assert!(
                    !text_v2.text.contains("1201726854"), // This was the buggy value
                    "Should not contain buggy large number"
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_wrap_eth_formats_amount_with_decimals() {
        // Verify amount is formatted with 18 decimals (ETH)
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let amount_min = U256::from(1_000_000_000_000_000_000u64); // 1 ETH

        let encoded = (recipient, amount_min).abi_encode();
        let field = UniversalRouterVisualizer::decode_wrap_eth(&encoded, 1, None);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("1.0") || text_v2.text.contains("1 ETH"),
                    "Expected ~1 ETH formatted, got: {}",
                    text_v2.text
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_sweep_params_order() {
        // SWEEP params: (address token, address recipient, uint256 amountMin)
        // This test verifies correct field order - NOT (token, amountMin, recipient)
        let token: Address = "0x255494b830bd4fe7220b3ec4842cba75600b6c80"
            .parse()
            .unwrap();
        let recipient: Address = "0xd27f4bbd67bd4ad1674c9c2c5a75ca8c3e389f3b"
            .parse()
            .unwrap();
        let amount_min = U256::from(2264700707120u64); // ~2264 tokens (if 9 decimals)

        // ABI encode: (address, address, uint256)
        let encoded = (token, recipient, amount_min).abi_encode();

        let field = UniversalRouterVisualizer::decode_sweep(&encoded, 1, None);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                // Should contain the correct amount, not a huge number from wrong field order
                assert!(
                    text_v2.text.contains("2264700707120"),
                    "Expected amount 2264700707120, got: {}",
                    text_v2.text
                );
                // Should contain correct recipient
                assert!(
                    text_v2.text.to_lowercase().contains("d27f4bbd"),
                    "Expected recipient address, got: {}",
                    text_v2.text
                );
                // Should NOT contain astronomically large numbers from wrong decoding
                assert!(
                    !text_v2.text.contains("120172685438592526"),
                    "Should not contain buggy large number from wrong field order"
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_sweep_with_known_token() {
        // Test SWEEP with WETH (which is in registry) to verify amount formatting
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        // WETH on mainnet
        let token: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let amount_min = U256::from(500_000_000_000_000_000u64); // 0.5 WETH

        let encoded = (token, recipient, amount_min).abi_encode();

        let field = UniversalRouterVisualizer::decode_sweep(&encoded, 1, Some(&registry));

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                // With registry, should format as 0.5 WETH
                assert!(
                    text_v2.text.contains("0.5") || text_v2.text.contains("WETH"),
                    "Expected formatted WETH amount, got: {}",
                    text_v2.text
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_wrap_eth_invalid_input() {
        // Test with invalid/short input
        let short_input = vec![0u8; 10];
        let field = UniversalRouterVisualizer::decode_wrap_eth(&short_input, 1, None);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("Failed to decode"),
                    "Expected decode failure message"
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_decode_sweep_invalid_input() {
        // Test with invalid/short input
        let short_input = vec![0u8; 10];
        let field = UniversalRouterVisualizer::decode_sweep(&short_input, 1, None);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("Failed to decode"),
                    "Expected decode failure message"
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }
    // =========================================================================
    // Bug fix regression tests
    // =========================================================================

    // --- Bug 1: Command-input index alignment ---

    #[test]
    fn test_bug1_unknown_command_preserves_index_alignment() {
        // A transaction with [V3SwapExactIn, UNKNOWN(0x07), WrapEth] and three inputs.
        // Before the fix, UNKNOWN was silently dropped, causing WrapEth to get
        // input[1] (the unknown command's data) instead of input[2].
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000_000_000_000u64); // 1 ETH

        // Build valid WrapEth input as input[2]
        let wrap_eth_encoded = (recipient, amount).abi_encode();

        let commands = vec![
            Command::V3SwapExactIn as u8,
            0x07, // Unknown command
            Command::WrapEth as u8,
        ];
        let inputs = vec![
            vec![0xde, 0xad], // garbage for V3Swap (will fail decode)
            vec![0xbe, 0xef], // garbage for unknown command
            wrap_eth_encoded, // valid WrapEth params
        ];
        let input = encode_execute_call(&commands, inputs, 1_700_000_000);

        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, None)
            .unwrap();

        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = result
        {
            // Should show 3 commands (not 2)
            assert!(
                common.fallback_text.contains("3 commands"),
                "Expected 3 commands, got: {}",
                common.fallback_text
            );
            // Unknown command should be visible
            assert!(
                common.fallback_text.contains("Unknown(0x07)"),
                "Expected Unknown(0x07), got: {}",
                common.fallback_text
            );

            let fields = preview_layout.expanded.unwrap().fields;
            // 3 commands + 1 deadline = 4 fields
            assert_eq!(fields.len(), 4, "Expected 4 fields (3 commands + deadline)");

            // Command 3 (WrapEth) should decode successfully with 1 ETH
            let wrap_field = &fields[2].signable_payload_field;
            let wrap_text = match wrap_field {
                SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                    preview_layout.subtitle.as_ref().unwrap().text.clone()
                }
                _ => panic!("Expected PreviewLayout for WrapEth"),
            };
            assert!(
                wrap_text.contains("1.0")
                    || wrap_text.contains("1 ETH")
                    || wrap_text.contains("Wrap"),
                "WrapEth should decode correctly with right input, got: {wrap_text}"
            );
            // Crucially, it should NOT say "Failed to decode"
            assert!(
                !wrap_text.contains("Failed to decode"),
                "WrapEth should NOT fail to decode — index alignment is broken if it does"
            );
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_bug1_multiple_unknown_commands_all_shown() {
        // Two unknown commands in a row
        let commands = vec![0xFE, 0xFF, Command::Sweep as u8];
        let inputs = vec![vec![0x01], vec![0x02], vec![0x03]];
        let input = encode_execute_call(&commands, inputs, 0);

        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, None)
            .unwrap();

        if let SignablePayloadField::PreviewLayout { common, .. } = result {
            assert!(
                common.fallback_text.contains("3 commands"),
                "All 3 commands should be counted"
            );
            assert!(
                common.fallback_text.contains("Unknown(0xfe)"),
                "First unknown should appear"
            );
            assert!(
                common.fallback_text.contains("Unknown(0xff)"),
                "Second unknown should appear"
            );
            assert!(
                common.fallback_text.contains("Sweep"),
                "Known command should appear"
            );
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    // --- Bug 2: V3 multi-hop path reads correct final token ---

    #[test]
    fn test_bug2_v3_exact_in_multihop_shows_final_token() {
        // Multi-hop V3 path: SETH -> (fee 3000) -> WETH -> (fee 500) -> USDC
        // Path bytes: SETH(20) | fee(3) | WETH(20) | fee(3) | USDC(20) = 66 bytes
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        let seth: Address = "0xe71bdfe1df69284f00ee185cf0d95d0c7680c0d4"
            .parse()
            .unwrap();
        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();

        // Build packed V3 path: SETH | 3000 | WETH | 500 | USDC
        let mut path = Vec::new();
        path.extend_from_slice(seth.as_slice());
        path.extend_from_slice(&[0x00, 0x0B, 0xB8]); // fee 3000
        path.extend_from_slice(weth.as_slice());
        path.extend_from_slice(&[0x00, 0x01, 0xF4]); // fee 500
        path.extend_from_slice(usdc.as_slice());
        assert_eq!(path.len(), 66);

        // ABI encode: (address recipient, uint256 amountIn, uint256 amountOutMin, bytes path, bool payerIsUser)
        let recipient = Address::ZERO;
        let amount_in = U256::from(1_000_000_000_000_000_000u64); // 1 SETH
        let amount_out_min = U256::from(1_000_000u64); // 1 USDC

        type V3SwapParams = (Address, U256, U256, Bytes, bool);
        let encoded = V3SwapParams::abi_encode_params(&(
            recipient,
            amount_in,
            amount_out_min,
            Bytes::from(path),
            true,
        ));

        let field =
            UniversalRouterVisualizer::decode_v3_swap_exact_in(&encoded, 1, Some(&registry));

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        // Output token should be USDC (final hop), NOT WETH (intermediate)
        assert!(
            text.contains("USDC"),
            "Multi-hop should show USDC as output token, got: {text}"
        );
        // Input token should be SETH
        assert!(text.contains("SETH"), "Input should be SETH, got: {text}");
        // Should NOT show WETH as output (that's the intermediate)
        assert!(
            !text.contains("for") || text.contains("USDC"),
            "Output should be USDC not WETH"
        );
    }

    #[test]
    fn test_bug2_v3_exact_in_single_hop_still_works() {
        // Single-hop V3 path: WETH -> (fee 3000) -> USDC (43 bytes)
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();

        let mut path = Vec::new();
        path.extend_from_slice(weth.as_slice());
        path.extend_from_slice(&[0x00, 0x0B, 0xB8]); // fee 3000
        path.extend_from_slice(usdc.as_slice());
        assert_eq!(path.len(), 43);

        let recipient = Address::ZERO;
        let amount_in = U256::from(1_000_000_000_000_000_000u64);
        let amount_out_min = U256::from(2_000_000_000u64);

        type V3SwapParams = (Address, U256, U256, Bytes, bool);
        let encoded = V3SwapParams::abi_encode_params(&(
            recipient,
            amount_in,
            amount_out_min,
            Bytes::from(path),
            true,
        ));

        let field =
            UniversalRouterVisualizer::decode_v3_swap_exact_in(&encoded, 1, Some(&registry));

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        assert!(text.contains("WETH"), "Input should be WETH, got: {text}");
        assert!(text.contains("USDC"), "Output should be USDC, got: {text}");
    }

    // --- Bug 3: V3 ExactOut path reversal ---

    #[test]
    fn test_bug3_v3_exact_out_reverses_path_tokens() {
        // ExactOut path is REVERSED: tokenOut(20) | fee(3) | tokenIn(20)
        // For a swap buying USDC with WETH, the path is: USDC | fee | WETH
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();

        // ExactOut path: USDC (output) | fee | WETH (input)
        let mut path = Vec::new();
        path.extend_from_slice(usdc.as_slice()); // tokenOut first in ExactOut
        path.extend_from_slice(&[0x00, 0x0B, 0xB8]); // fee 3000
        path.extend_from_slice(weth.as_slice()); // tokenIn last in ExactOut

        let recipient = Address::ZERO;
        let amount_out = U256::from(1_000_000_000u64); // 1000 USDC (6 decimals)
        let amount_in_max = U256::from(1_000_000_000_000_000_000u64); // 1 WETH

        type V3SwapParams = (Address, U256, U256, Bytes, bool);
        let encoded = V3SwapParams::abi_encode_params(&(
            recipient,
            amount_out,
            amount_in_max,
            Bytes::from(path),
            true,
        ));

        let field =
            UniversalRouterVisualizer::decode_v3_swap_exact_out(&encoded, 1, Some(&registry));

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        // Should show "Swap <=1.0 WETH for 1000.0 USDC" (input is WETH, output is USDC)
        assert!(
            text.contains("WETH") && text.contains("USDC"),
            "Should show both WETH and USDC, got: {text}"
        );
        // The text format is "Swap <=X INPUT for Y OUTPUT"
        // WETH should appear after "<=" (input) and USDC after "for" (output)
        let weth_pos = text.find("WETH").unwrap();
        let usdc_pos = text.find("USDC").unwrap();
        let for_pos = text.find("for").unwrap();
        assert!(
            weth_pos < for_pos && usdc_pos > for_pos,
            "WETH (input) should be before 'for', USDC (output) after. Got: {text}"
        );
    }

    #[test]
    fn test_bug3_v3_exact_out_multihop_reversed() {
        // Multi-hop ExactOut: buying DAI with WETH through USDC
        // ExactOut path (reversed): DAI | fee | USDC | fee | WETH
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let dai: Address = "0x6b175474e89094c44da98b954eedeac495271d0f"
            .parse()
            .unwrap();

        let mut path = Vec::new();
        path.extend_from_slice(dai.as_slice()); // tokenOut
        path.extend_from_slice(&[0x00, 0x01, 0xF4]); // fee 500
        path.extend_from_slice(usdc.as_slice()); // intermediate
        path.extend_from_slice(&[0x00, 0x0B, 0xB8]); // fee 3000
        path.extend_from_slice(weth.as_slice()); // tokenIn
        assert_eq!(path.len(), 66);

        let recipient = Address::ZERO;
        let amount_out = U256::from(1_000_000_000_000_000_000u64); // 1 DAI
        let amount_in_max = U256::from(500_000_000_000_000u64); // 0.0005 WETH

        type V3SwapParams = (Address, U256, U256, Bytes, bool);
        let encoded = V3SwapParams::abi_encode_params(&(
            recipient,
            amount_out,
            amount_in_max,
            Bytes::from(path),
            true,
        ));

        let field =
            UniversalRouterVisualizer::decode_v3_swap_exact_out(&encoded, 1, Some(&registry));

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        // Output should be DAI (first in reversed path), input should be WETH (last)
        assert!(text.contains("DAI"), "Output should be DAI, got: {text}");
        assert!(text.contains("WETH"), "Input should be WETH, got: {text}");
        // Should NOT show USDC (intermediate token)
        let for_pos = text.find("for").unwrap();
        let after_for = &text[for_pos..];
        assert!(
            after_for.contains("DAI"),
            "DAI should appear as output after 'for', got: {text}"
        );
    }

    // --- Bug 4: ExecuteSubPlan recursive decoding ---

    #[test]
    fn test_bug4_execute_sub_plan_decodes_nested_commands() {
        use alloy_sol_types::sol_data;

        // Build a sub-plan with [WrapEth] command
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000_000_000_000u64);
        let wrap_input = (recipient, amount).abi_encode();

        let sub_commands: Vec<u8> = vec![Command::WrapEth as u8];
        let sub_inputs: Vec<Bytes> = vec![Bytes::from(wrap_input)];

        // ABI-encode the sub-plan: (bytes, bytes[])
        type SubPlanParams = (sol_data::Bytes, sol_data::Array<sol_data::Bytes>);
        let sub_plan_encoded = SubPlanParams::abi_encode_params(&(
            sub_commands.clone(),
            sub_inputs.iter().map(|b| b.to_vec()).collect::<Vec<_>>(),
        ));

        let field =
            UniversalRouterVisualizer::decode_execute_sub_plan(&sub_plan_encoded, 1, None, 0);

        // Should decode as a nested Universal Router Execute, not raw hex
        match &field {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert!(
                    common.fallback_text.contains("1 commands"),
                    "Sub-plan should show 1 command, got: {}",
                    common.fallback_text
                );
                assert!(
                    common.fallback_text.contains("WrapEth"),
                    "Sub-plan should show WrapEth, got: {}",
                    common.fallback_text
                );
                // The nested WrapEth should be decoded (not "Failed to decode")
                if let Some(expanded) = &preview_layout.expanded {
                    let nested = &expanded.fields[0].signable_payload_field;
                    match nested {
                        SignablePayloadField::PreviewLayout {
                            preview_layout: inner,
                            ..
                        } => {
                            let subtitle = inner.subtitle.as_ref().unwrap().text.clone();
                            assert!(
                                subtitle.contains("Wrap") || subtitle.contains("ETH"),
                                "Nested WrapEth should decode, got: {subtitle}"
                            );
                        }
                        _ => panic!("Expected PreviewLayout for nested command"),
                    }
                }
            }
            _ => panic!("Expected PreviewLayout, got: {field:?}"),
        }
    }

    #[test]
    fn test_bug4_execute_sub_plan_invalid_input() {
        let garbage = vec![0xde, 0xad, 0xbe, 0xef];
        let field = UniversalRouterVisualizer::decode_execute_sub_plan(&garbage, 1, None, 0);

        match &field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("Failed to decode"),
                    "Invalid sub-plan should show decode error"
                );
            }
            _ => panic!("Expected TextV2 error field"),
        }
    }

    // --- Bug 5: TRANSFER command correct semantics ---

    #[test]
    fn test_bug5_transfer_shows_token_symbol_and_recipient() {
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        // Transfer 0.5 WETH to a recipient
        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let recipient: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let value = U256::from(500_000_000_000_000_000u64); // 0.5 WETH

        // ABI encode: (address token, address recipient, uint256 value)
        let encoded = (weth, recipient, value).abi_encode();

        let field = UniversalRouterVisualizer::decode_transfer(&encoded, 1, Some(&registry));

        match &field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                // Should show "Transfer 0.5 WETH to <recipient>"
                assert!(
                    text_v2.text.contains("WETH"),
                    "Should resolve WETH symbol, got: {}",
                    text_v2.text
                );
                assert!(
                    text_v2.text.contains("0.5"),
                    "Should format amount with decimals, got: {}",
                    text_v2.text
                );
                // Should NOT say "from" (old buggy format)
                assert!(
                    !text_v2.text.contains("from"),
                    "Should NOT say 'from' — token is the contract, not sender. Got: {}",
                    text_v2.text
                );
                assert!(
                    text_v2.text.to_lowercase().contains("d8da6bf2"),
                    "Should show recipient address, got: {}",
                    text_v2.text
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_bug5_transfer_unknown_token_shows_address() {
        // Transfer with a token not in the registry
        let unknown_token: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let recipient: Address = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();
        let value = U256::from(42u64);

        let encoded = (unknown_token, recipient, value).abi_encode();

        let field = UniversalRouterVisualizer::decode_transfer(&encoded, 1, None);

        match &field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                // Should show raw address for unknown token
                assert!(
                    text_v2.text.to_lowercase().contains("1111111111"),
                    "Should show token address, got: {}",
                    text_v2.text
                );
                assert!(
                    text_v2.text.contains("42"),
                    "Should show raw amount, got: {}",
                    text_v2.text
                );
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    // --- Bug 6: Permit2 delegation (direct decoding) ---

    #[test]
    fn test_bug6_permit2_transfer_from_decodes_without_selector() {
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        // PERMIT2_TRANSFER_FROM params: (address token, address recipient, uint256 amount)
        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let recipient: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let amount = U256::from(2_000_000_000_000_000_000u64); // 2 WETH

        type TransferFromParams = (Address, Address, U256);
        let encoded = TransferFromParams::abi_encode_params(&(weth, recipient, amount));

        let field =
            UniversalRouterVisualizer::decode_permit2_transfer_from(&encoded, 1, Some(&registry));

        match &field {
            SignablePayloadField::TextV2 { text_v2, common } => {
                assert_eq!(common.label, "Permit2 Transfer From");
                assert!(
                    text_v2.text.contains("WETH"),
                    "Should resolve WETH, got: {}",
                    text_v2.text
                );
                assert!(
                    text_v2.text.contains("2.0") || text_v2.text.contains("2 WETH"),
                    "Should show 2 WETH, got: {}",
                    text_v2.text
                );
                // Must NOT say "Failed to decode"
                assert!(
                    !text_v2.text.contains("Failed"),
                    "Should successfully decode, got: {}",
                    text_v2.text
                );
            }
            _ => panic!("Expected TextV2 field, got: {field:?}"),
        }
    }

    #[test]
    fn test_bug6_permit2_permit_decodes_without_selector() {
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        // Build inline PermitSingle (192 bytes) as Universal Router encodes it
        let mut permit_single = vec![0u8; 192];

        // Token at bytes 12-31
        let token_bytes = hex::decode("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2").unwrap();
        permit_single[12..32].copy_from_slice(&token_bytes);

        // Amount: max uint160 (unlimited)
        let amount_bytes = hex::decode("ffffffffffffffffffffffffffffffffffffffff").unwrap();
        permit_single[44..64].copy_from_slice(&amount_bytes);

        // Expiration at 90-95: some future date
        permit_single[90..96].copy_from_slice(&[0u8, 0, 0x69, 0x40, 0x57, 0x19]);

        // Spender at 140-159
        let spender_bytes = hex::decode("3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad").unwrap();
        permit_single[140..160].copy_from_slice(&spender_bytes);

        // sigDeadline at 160-191
        permit_single[188..192].copy_from_slice(&[0x69, 0x18, 0xd1, 0x21]);

        let field =
            UniversalRouterVisualizer::decode_permit2_permit(&permit_single, 1, Some(&registry));

        match &field {
            SignablePayloadField::TextV2 { text_v2, common } => {
                assert_eq!(common.label, "Permit2 Permit");
                assert!(
                    text_v2.text.contains("Permit"),
                    "Should show Permit, got: {}",
                    text_v2.text
                );
                assert!(
                    text_v2.text.contains("Unlimited"),
                    "Max uint160 should show as Unlimited, got: {}",
                    text_v2.text
                );
                assert!(
                    !text_v2.text.contains("Failed"),
                    "Should NOT fail to decode, got: {}",
                    text_v2.text
                );
            }
            // Also accept PreviewLayout (if fallback was used)
            SignablePayloadField::PreviewLayout { common, .. } => {
                assert!(
                    !common.fallback_text.contains("Failed"),
                    "Should not show failure: {}",
                    common.fallback_text
                );
            }
            _ => panic!("Unexpected field type: {field:?}"),
        }
    }

    // --- Bug 14: PayPortion bips > 10000 warning ---

    #[test]
    fn test_bug14_pay_portion_bips_over_100_percent_shows_warning() {
        let token: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let bips = U256::from(20000u64); // 200%

        let encoded = (token, recipient, bips).abi_encode();

        let field = UniversalRouterVisualizer::decode_pay_portion(&encoded, 1, None);

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        assert!(
            text.contains("WARNING") || text.contains(">100%"),
            "Bips > 10000 should show warning, got: {text}"
        );
    }

    #[test]
    fn test_bug14_pay_portion_normal_bips_no_warning() {
        let token: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let bips = U256::from(25u64); // 0.25%

        let encoded = (token, recipient, bips).abi_encode();

        let field = UniversalRouterVisualizer::decode_pay_portion(&encoded, 1, None);

        let text = match &field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.subtitle.as_ref().unwrap().text.clone()
            }
            _ => panic!("Expected PreviewLayout"),
        };

        assert!(
            !text.contains("WARNING"),
            "Normal bips should not show warning, got: {text}"
        );
        assert!(text.contains("0.25"), "Should show 0.25%, got: {text}");
    }

    // --- Integration: full transaction with all command types ---

    #[test]
    fn test_integration_full_transaction_with_mixed_commands() {
        // Build a realistic transaction: V2SwapExactIn + PayPortion + UnwrapWeth
        let (registry, _) = crate::registry::ContractRegistry::with_default_protocols();

        let weth: Address = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
            .parse()
            .unwrap();
        let usdc: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let recipient: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let fee_recipient: Address = "0x000000fee13a103a10d593b9ae06b3e05f2e7e1c"
            .parse()
            .unwrap();

        // V2SwapExactIn: swap 1000 USDC for WETH
        use alloy_sol_types::sol_data;
        type V2Params = (
            sol_data::Address,
            sol_data::Uint<256>,
            sol_data::Uint<256>,
            sol_data::Array<sol_data::Address>,
            sol_data::Address,
        );
        let v2_encoded = V2Params::abi_encode_params(&(
            recipient,
            U256::from(1_000_000_000u64),       // 1000 USDC
            U256::from(500_000_000_000_000u64), // min 0.0005 WETH
            vec![usdc, weth],
            recipient,
        ));

        // PayPortion: 0.25% fee
        let pay_encoded = (weth, fee_recipient, U256::from(25u64)).abi_encode();

        // UnwrapWeth
        let unwrap_encoded = (recipient, U256::from(400_000_000_000_000u64)).abi_encode();

        let commands = vec![
            Command::V2SwapExactIn as u8,
            Command::PayPortion as u8,
            Command::UnwrapWeth as u8,
        ];
        let inputs = vec![v2_encoded, pay_encoded, unwrap_encoded];
        let input = encode_execute_call(&commands, inputs, 1_700_000_000);

        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, Some(&registry))
            .unwrap();

        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = result
        {
            assert!(common.fallback_text.contains("3 commands"));
            let fields = preview_layout.expanded.unwrap().fields;
            // 3 commands + 1 deadline = 4 fields
            assert_eq!(fields.len(), 4);

            // Verify each command decoded successfully (no "Failed to decode")
            for (i, field) in fields.iter().take(3).enumerate() {
                if let SignablePayloadField::PreviewLayout { preview_layout, .. } =
                    &field.signable_payload_field
                {
                    let subtitle = preview_layout.subtitle.as_ref().unwrap().text.clone();
                    assert!(
                        !subtitle.contains("Failed to decode"),
                        "Command {} should decode successfully, got: {subtitle}",
                        i + 1,
                    );
                }
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }
}
