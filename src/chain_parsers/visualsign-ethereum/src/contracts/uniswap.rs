use alloy_sol_types::{SolCall, sol};
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

sol! {
    // This interface is a combination of common elements found in Uniswap V4's
    // IUniversalRouter and related types. Always verify with the latest official sources.

    // --- Enums ---
    // The Command enum defines the various operations the Universal Router can perform.
    // Each command corresponds to a specific action (e.g., swapping, settling, taking assets).
    enum Command {
        // V4_SWAP is for interacting with Uniswap V4 pools
        V4_SWAP,
        // WRAP_ETH allows wrapping Ether into WETH
        WRAP_ETH,
        // UNWRAP_WETH allows unwrapping WETH back to Ether
        UNWRAP_WETH,
        // SWEEP allows sweeping tokens to a recipient
        SWEEP,
        // TRANSFER allows transferring tokens
        TRANSFER,
        // PERMIT2_PERMIT allows setting allowances via Permit2
        PERMIT2_PERMIT,
        // APPROVE allows approving tokens for spending
        APPROVE,
        // TAKE_FROM_ASSETS allows taking assets from the Universal Router's asset buffer
        TAKE_FROM_ASSETS,
        // PAY_TO_WALLET allows paying assets to a wallet
        PAY_TO_WALLET,
        // SETTLE_ALL settles all assets
        SETTLE_ALL,
        // TAKE_ALL takes all assets
        TAKE_ALL,
        // FILL_AND_SETTLE fills an order and settles assets
        FILL_AND_SETTLE,
        // V3_SWAP_EXACT_IN performs an exact input swap on Uniswap V3
        V3_SWAP_EXACT_IN,
        // V3_SWAP_EXACT_OUT performs an exact output swap on Uniswap V3
        V3_SWAP_EXACT_OUT,
        // V3_SWAP_EXACT_IN_SINGLE performs an exact input swap on a single Uniswap V3 pool
        V3_SWAP_EXACT_IN_SINGLE,
        // V3_SWAP_EXACT_OUT_SINGLE performs an exact output swap on a single Uniswap V3 pool
        V3_SWAP_EXACT_OUT_SINGLE,
        // V3_MINT mints liquidity on Uniswap V3
        V3_MINT,
        // V3_BURN burns liquidity on Uniswap V3
        V3_BURN,
        // V3_COLLECT collects fees/liquidity on Uniswap V3
        V3_COLLECT,
        // V3_INCREASE_LIQUIDITY increases liquidity on Uniswap V3
        V3_INCREASE_LIQUIDITY,
        // V3_DECREASE_LIQUIDITY decreases liquidity on Uniswap V3
        V3_DECREASE_LIQUIDITY,
        // ... (add more commands as V4 evolves)
    }

    // --- Structs ---
    // PoolKey uniquely identifies a Uniswap V4 pool.
    struct PoolKey {
        address currency0;
        address currency1;
        uint24 fee;
        int24 tickSpacing;
        address hooks; // Address of the hooks contract, can be zero address
    }

    // V4SwapParams defines parameters for a V4_SWAP command.
    struct V4SwapParams {
        bytes poolKey; // Encoded PoolKey
        address recipient;
        uint128 amount;
        uint128 sqrtPriceLimitX96;
        bool zeroForOne;
    }

    // Permit2PermitParams defines parameters for a PERMIT2_PERMIT command.
    struct Permit2PermitParams {
        address token;
        uint160 amount;
        uint48 expiration;
        uint48 nonce;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    // ExactInputSingleParams for V3 swaps.
    struct ExactInputSingleParams {
        PoolKey poolKey; // Using the V4 PoolKey for V3 interaction
        address recipient;
        uint256 amountIn;
        uint256 minAmountOut;
        uint160 sqrtPriceLimitX96;
    }

    // ExactOutputSingleParams for V3 swaps.
    struct ExactOutputSingleParams {
        PoolKey poolKey;
        address recipient;
        uint256 amountOut;
        uint256 maxAmountIn;
        uint160 sqrtPriceLimitX96;
    }

    // V3MintParams defines parameters for V3_MINT.
    struct V3MintParams {
        PoolKey poolKey;
        address recipient;
        int24 tickLower;
        int24 tickUpper;
        uint128 amount;
        bytes data; // Arbitrary data for hooks/callbacks
    }

    // V3BurnParams defines parameters for V3_BURN.
    struct V3BurnParams {
        PoolKey poolKey;
        int24 tickLower;
        int24 tickUpper;
        uint128 amount;
    }

    // V3CollectParams defines parameters for V3_COLLECT.
    struct V3CollectParams {
        PoolKey poolKey;
        address recipient;
        int24 tickLower;
        int24 tickUpper;
        uint128 amount0Max;
        uint128 amount1Max;
    }

    // --- Interface Definition ---
    // This defines the core functions of the Universal Router.
    #[sol(rpc)]
    interface IUniversalRouter {
        // The primary function for executing a series of commands.
        // `commands`: A byte string where each byte represents a `Command` enum variant.
        // `inputs`: An array of tightly packed bytes, where each element corresponds to
        //           the parameters for a command specified in the `commands` byte string.
        // `deadline`: The timestamp after which the transaction will revert if not executed.
        function execute(
            bytes commands,
            bytes[] inputs,
            uint256 deadline
        ) external payable;
    }
}

#[derive(Debug)]
struct UniversalRouterExecute {
    commands: Vec<u8>,
    deadline: String,
}

fn decode_universal_router_execute(input: &[u8]) -> Option<UniversalRouterExecute> {
    if input.len() < 4 {
        println!("Input too short for Universal Router execute call");
        return None;
    }

    if let Ok(call) = IUniversalRouter::executeCall::abi_decode(input) {
        println!("Decoded Universal Router execute call");
        Some(UniversalRouterExecute {
            commands: call.commands.0.to_vec(),
            deadline: call.deadline.to_string(),
        })
    } else {
        println!("Failed to decode Universal Router execute call");
        None
    }
}

pub fn parse_universal_router_execute(input: &[u8]) -> Vec<SignablePayloadField> {
    let mut fields = Vec::new();
    if let Some(decoded) = decode_universal_router_execute(input) {
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "Universal Router Execute: {} commands, deadline {}",
                    decoded.commands.len(),
                    decoded.deadline
                ),
                label: "Universal Router".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!(
                    "Commands: {:?}\nDeadline: {}",
                    decoded.commands, decoded.deadline
                ),
            },
        });
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Uint;

    #[test]
    fn test_decode_universal_router_execute_invalid_selector() {
        // Wrong selector
        let input_data = vec![0x00, 0x00, 0x00, 0x00];
        assert!(decode_universal_router_execute(&input_data).is_none());
    }

    #[test]
    fn test_decode_universal_router_execute_too_short() {
        // Less than 4 bytes
        let input_data = vec![0x35, 0x93, 0x56];
        assert!(decode_universal_router_execute(&input_data).is_none());
    }

    #[test]
    fn test_parse_universal_router_execute_field() {
        // Use the same valid input as aboves
        let commands = vec![1u8, 2, 3];
        let inputs: Vec<Vec<u8>> = vec![];
        let deadline = Uint::<256, 4>::from(1234567890);
        let call = IUniversalRouter::executeCall {
            commands: commands.clone().into(),
            inputs: inputs.iter().map(|v| v.clone().into()).collect(),
            deadline,
        };

        let input_data = call.abi_encode();
        let fields = parse_universal_router_execute(&input_data);
        assert_eq!(fields.len(), 1);
        if let SignablePayloadField::TextV2 { common, text_v2 } = &fields[0] {
            assert_eq!(
                "Universal Router Execute: 3 commands, deadline 1234567890",
                common.fallback_text
            );
            assert_eq!("Commands: [1, 2, 3]\nDeadline: 1234567890", text_v2.text);
        } else {
            panic!("Expected TextV2 field");
        }
    }
}
