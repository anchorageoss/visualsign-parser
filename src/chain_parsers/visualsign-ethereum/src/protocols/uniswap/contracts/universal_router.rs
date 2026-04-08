use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall as _, SolType, SolValue, sol, sol_data};
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

/// Truncates a byte slice to hex, capping at `limit` bytes to avoid unbounded
/// memory allocation from untrusted calldata.
fn truncated_hex(bytes: &[u8], limit: usize) -> String {
    if bytes.len() > limit {
        format!("0x{}...", hex::encode(&bytes[..limit]))
    } else {
        format!("0x{}", hex::encode(bytes))
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
                        let input_hex = truncated_hex(bytes, 32);
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
                // For recognized but unimplemented commands, show truncated hex
                let input_hex = truncated_hex(bytes, 32);
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
            let input_hex = truncated_hex(bytes, 32);
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
        type SubPlanParams = (sol_data::Bytes, sol_data::Array<sol_data::Bytes>);

        let params = match SubPlanParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                let input_hex = truncated_hex(bytes, 32);
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
            // Re-label nested field to distinguish from the top-level execute
            match nested_field {
                SignablePayloadField::PreviewLayout {
                    common,
                    preview_layout,
                } => SignablePayloadField::PreviewLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: common
                            .fallback_text
                            .replace("Uniswap Universal Router Execute", "Sub-Plan"),
                        label: "Sub-Plan".to_string(),
                    },
                    preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                        title: Some(visualsign::SignablePayloadFieldTextV2 {
                            text: format!("Sub-Plan ({} commands)", sub_commands.len()),
                        }),
                        ..preview_layout
                    },
                },
                other => other,
            }
        } else {
            let input_hex = truncated_hex(bytes, 32);
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
                        fallback_text: format!("V3 Swap Exact In: {}", truncated_hex(bytes, 32)),
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

        // Format amounts via registry (overflow-safe)
        let (amount_in_str, token_in_symbol) =
            format_amount_with_registry(&amount_in, chain_id, token_in, registry);
        let (amount_out_min_str, token_out_symbol) =
            format_amount_with_registry(&amount_out_min, chain_id, token_out, registry);

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
                        fallback_text: format!("Pay Portion: {}", truncated_hex(bytes, 32)),
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
                        fallback_text: format!("Unwrap WETH: {}", truncated_hex(bytes, 32)),
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
        // Fall back to format_ether if registry unavailable
        let amount_min_str =
            crate::protocols::uniswap::config::UniswapConfig::weth_address(chain_id)
                .and_then(|weth_addr| {
                    let amount_u128: u128 = params.amountMinimum.to_string().parse().ok()?;
                    registry.and_then(|r| r.format_token_amount(chain_id, weth_addr, amount_u128))
                })
                .map(|(amt, _)| amt)
                .unwrap_or_else(|| crate::fmt::format_ether(params.amountMinimum));

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
                        fallback_text: format!("V3 Swap Exact Out: {}", truncated_hex(bytes, 32)),
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

        // Format amounts via registry (overflow-safe)
        let (amount_out_str, token_out_symbol) =
            format_amount_with_registry(&amount_out, chain_id, token_out, registry);
        let (amount_in_max_str, token_in_symbol) =
            format_amount_with_registry(&amount_in_max, chain_id, token_in, registry);

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
                        fallback_text: format!("V2 Swap Exact In: {}", truncated_hex(bytes, 32)),
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

        // Format amounts via registry (overflow-safe)
        let (amount_in_str, token_in_symbol) =
            format_amount_with_registry(&amount_in, chain_id, token_in, registry);
        let (amount_out_min_str, token_out_symbol) =
            format_amount_with_registry(&amount_out_minimum, chain_id, token_out, registry);

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
                        fallback_text: format!("V2 Swap Exact Out: {}", truncated_hex(bytes, 32)),
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

        // Format amounts via registry (overflow-safe)
        let (amount_out_str, token_out_symbol) =
            format_amount_with_registry(&amount_out, chain_id, token_out, registry);
        let (amount_in_max_str, token_in_symbol) =
            format_amount_with_registry(&amount_in_maximum, chain_id, token_in, registry);

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
                        fallback_text: format!("Wrap ETH: {}", truncated_hex(bytes, 32)),
                        label: "Wrap ETH".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        // Format amount with ETH decimals (18) — overflow-safe
        // Try registry first, fall back to format_ether if registry unavailable
        let amount_min_str =
            crate::protocols::uniswap::config::UniswapConfig::weth_address(chain_id)
                .and_then(|weth_addr| {
                    let amount_u128: u128 = params.amountMin.to_string().parse().ok()?;
                    registry.and_then(|r| r.format_token_amount(chain_id, weth_addr, amount_u128))
                })
                .map(|(amt, _)| amt)
                .unwrap_or_else(|| crate::fmt::format_ether(params.amountMin));

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
                        fallback_text: format!("Sweep: {}", truncated_hex(bytes, 32)),
                        label: "Sweep".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        // Format amount via registry (overflow-safe)
        let (amount_min_str, token_symbol) =
            format_amount_with_registry(&params.amountMinimum, chain_id, params.token, registry);

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
                        fallback_text: format!("Transfer: {}", truncated_hex(bytes, 32)),
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
    /// Universal Router encodes this as (address token, address recipient, uint160 amount)
    /// — raw ABI params without a function selector.
    fn decode_permit2_transfer_from(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Decode directly: (address token, address recipient, uint160 amount)
        type TransferFromParams = (Address, Address, alloy_primitives::U160);

        let params = match TransferFromParams::abi_decode_params(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!(
                            "Permit2 Transfer From: {}",
                            truncated_hex(bytes, 32)
                        ),
                        label: "Permit2 Transfer From".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let (token, recipient, amount) = params;

        let amount_u256 = U256::from(amount);
        let (amount_str, token_symbol) =
            format_amount_with_registry(&amount_u256, chain_id, token, registry);

        let text = format!("Permit2 Transfer {amount_str} {token_symbol} to {recipient:?}");

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.clone(),
                label: "Permit2 Transfer From".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text },
        }
    }

    /// Decodes PERMIT2_PERMIT (0x0a) command by delegating to Permit2Visualizer.
    /// Universal Router encodes this as inline PermitSingle bytes + signature,
    /// without a function selector. Permit2Visualizer.visualize_tx_commands
    /// handles this via decode_custom_permit_params and produces a rich
    /// PreviewLayout with Token, Amount, Spender, Expires, and Sig Deadline.
    fn decode_permit2_permit(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let visualizer = Permit2Visualizer;
        visualizer
            .visualize_tx_commands(bytes, chain_id, registry)
            .unwrap_or_else(|| Self::show_decode_error(bytes, &"Failed to decode parameters"))
    }

    /// Helper function to display decoding error with raw hex slots
    fn show_decode_error(bytes: &[u8], err: &dyn std::fmt::Display) -> SignablePayloadField {
        let hex_data = truncated_hex(bytes, 32);
        let chunk_size = 32;
        let mut fields = vec![];

        // Cap at 16 slots (512 bytes) to prevent unbounded allocation from large payloads
        for (i, chunk) in bytes.chunks(chunk_size).take(16).enumerate() {
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
        // 0xff is not a valid Command — it should be preserved as Unknown to keep indices aligned
        let commands = vec![0xff, Command::Transfer as u8];
        let inputs = vec![vec![0x01], vec![0x02]];
        let deadline = 0u64;
        let input = encode_execute_call(&commands, inputs.clone(), deadline);

        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, None)
            .unwrap();

        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = result
        {
            assert!(
                common
                    .fallback_text
                    .contains("2 commands ([Unknown(0xff), Transfer])"),
                "Expected 2 commands with Unknown(0xff), got: {}",
                common.fallback_text,
            );
            let fields = preview_layout.expanded.unwrap().fields;
            assert_eq!(
                fields.len(),
                2,
                "Expected 2 fields (2 commands, no deadline for epoch 0)"
            );

            // Command 1: Unknown command 0xff with input 0x01
            let unknown_field = &fields[0].signable_payload_field;
            match unknown_field {
                SignablePayloadField::PreviewLayout {
                    preview_layout: pl, ..
                } => {
                    assert_eq!(pl.title.as_ref().unwrap().text, "Unknown(0xff)");
                    assert_eq!(pl.subtitle.as_ref().unwrap().text, "Input: 0x01");
                }
                _ => panic!("Expected PreviewLayout for unknown command"),
            }

            // Command 2: Transfer with input 0x02 (its correct input)
            let transfer_field = &fields[1].signable_payload_field;
            match transfer_field {
                SignablePayloadField::PreviewLayout {
                    preview_layout: pl, ..
                } => {
                    assert_eq!(pl.title.as_ref().unwrap().text, "Transfer");
                    // 0x02 is too short to decode Transfer params
                    assert_eq!(
                        pl.subtitle.as_ref().unwrap().text,
                        "Failed to decode parameters"
                    );
                }
                _ => panic!("Expected PreviewLayout for Transfer command"),
            }
        } else {
            panic!("Expected PreviewLayout");
        }
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

    #[test]
    fn test_bug1_unknown_command_preserves_index_alignment() {
        // Tests that unknown commands keep indices aligned with inputs array
        // Build: [V3SwapExactIn, UNKNOWN(0x07), WrapEth] with valid WrapEth input at position 2
        let recipient: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000_000_000_000u64);
        let wrap_eth_encoded = (recipient, amount).abi_encode();
        let commands = vec![Command::V3SwapExactIn as u8, 0x07, Command::WrapEth as u8];
        let inputs = vec![vec![0xde, 0xad], vec![0xbe, 0xef], wrap_eth_encoded];
        let input = encode_execute_call(&commands, inputs, 1_700_000_000);
        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, None)
            .unwrap();
        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = result
        {
            assert!(
                common.fallback_text.contains("3 commands"),
                "Expected 3 commands, got: {}",
                common.fallback_text,
            );
            assert!(
                common.fallback_text.contains("Unknown(0x07)"),
                "Expected Unknown(0x07), got: {}",
                common.fallback_text,
            );
            let fields = preview_layout.expanded.unwrap().fields;
            assert_eq!(fields.len(), 4, "Expected 4 fields (3 commands + deadline)");
            // WrapEth (Command 3) should decode successfully — NOT "Failed to decode"
            let wrap_field = &fields[2].signable_payload_field;
            let wrap_text = match wrap_field {
                SignablePayloadField::PreviewLayout {
                    preview_layout: pl, ..
                } => pl.subtitle.as_ref().unwrap().text.clone(),
                _ => panic!("Expected PreviewLayout for WrapEth"),
            };
            assert!(
                wrap_text.contains("1.0")
                    || wrap_text.contains("1 ETH")
                    || wrap_text.contains("Wrap"),
                "WrapEth should decode correctly, got: {wrap_text}",
            );
            assert!(
                !wrap_text.contains("Failed to decode"),
                "WrapEth should NOT fail to decode -- index alignment is broken if it does",
            );
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_bug1_multiple_unknown_commands_all_shown() {
        let commands = vec![0xFE, 0xFF, Command::Sweep as u8];
        let inputs = vec![vec![0x01], vec![0x02], vec![0x03]];
        let input = encode_execute_call(&commands, inputs, 0);
        let result = UniversalRouterVisualizer {}
            .visualize_tx_commands(&input, 1, None)
            .unwrap();
        if let SignablePayloadField::PreviewLayout { common, .. } = result {
            assert!(common.fallback_text.contains("3 commands"));
            assert!(common.fallback_text.contains("Unknown(0xfe)"));
            assert!(common.fallback_text.contains("Unknown(0xff)"));
            assert!(common.fallback_text.contains("Sweep"));
        } else {
            panic!("Expected PreviewLayout");
        }
    }
}
