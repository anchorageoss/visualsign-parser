//! Shared helpers for the Permit2 rendering tests (`permit2_mainnet.rs` and
//! `permit2_proptest.rs`).
//!
//! The ABI definitions here are written from the Permit2 and Universal Router
//! sources, independently of the parser's own `sol!` blocks, so the tests
//! encode and decode calldata without going through the code under test.
//!
//! - Permit2: <https://github.com/Uniswap/permit2/blob/main/src/interfaces/IAllowanceTransfer.sol>
//! - Universal Router: <https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Dispatcher.sol>
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use alloy_primitives::utils::format_units;
use alloy_primitives::{Address, Bytes, U160, U256, address};
use alloy_sol_types::{SolInterface, SolValue};
use serde_json::Value;

use IPermit2Ref::IPermit2RefCalls as P;
use IUniversalRouterRef::IUniversalRouterRefCalls as U;

alloy_sol_types::sol! {
    #[derive(Debug, PartialEq)]
    struct PermitDetails {
        address token;
        uint160 amount;
        uint48 expiration;
        uint48 nonce;
    }

    #[derive(Debug, PartialEq)]
    struct PermitSingle {
        PermitDetails details;
        address spender;
        uint256 sigDeadline;
    }

    #[derive(Debug, PartialEq)]
    struct PermitBatch {
        PermitDetails[] details;
        address spender;
        uint256 sigDeadline;
    }

    #[derive(Debug, PartialEq)]
    struct AllowanceTransferDetails {
        address from;
        address to;
        uint160 amount;
        address token;
    }

    #[derive(Debug, PartialEq)]
    struct TokenSpenderPair {
        address token;
        address spender;
    }

    #[derive(Debug, PartialEq)]
    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    #[derive(Debug, PartialEq)]
    struct PermitTransferFrom {
        TokenPermissions permitted;
        uint256 nonce;
        uint256 deadline;
    }

    #[derive(Debug, PartialEq)]
    struct SignatureTransferDetails {
        address to;
        uint256 requestedAmount;
    }

    interface IPermit2Ref {
        function approve(address token, address spender, uint160 amount, uint48 expiration) external;
        function permit(address owner, PermitSingle permitSingle, bytes signature) external;
        function permit(address owner, PermitBatch permitBatch, bytes signature) external;
        function transferFrom(address from, address to, uint160 amount, address token) external;
        function transferFrom(AllowanceTransferDetails[] transferDetails) external;
        function lockdown(TokenSpenderPair[] approvals) external;
        function invalidateNonces(address token, address spender, uint48 newNonce) external;
        // SignatureTransfer half of Permit2.
        function permitTransferFrom(PermitTransferFrom permit, SignatureTransferDetails transferDetails, address owner, bytes signature) external;
        function invalidateUnorderedNonces(uint256 wordPos, uint256 mask) external;
    }

    interface IUniversalRouterRef {
        function execute(bytes commands, bytes[] inputs, uint256 deadline) external payable;
        function execute(bytes commands, bytes[] inputs) external payable;
    }
}

/// Permit2, at the same address on every chain.
pub const PERMIT2: Address = address!("0x000000000022D473030F116dDEE9F6B43aC78BA3");
/// Universal Router V2.0 on Ethereum mainnet (registered in the parser's config).
pub const UR_V2_0: Address = address!("0x66a9893cc07d91d95644aedd05d03f95e1dba8af");
/// WETH on Ethereum mainnet.
pub const WETH: Address = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

/// The Ethereum mainnet tokens in the parser's built-in registry
/// (`UniswapConfig::register_common_tokens`), with their symbols and decimals.
/// Any other token renders as its raw integer amount.
pub const BUILTIN_MAINNET_TOKENS: &[(Address, &str, u8)] = &[
    (WETH, "WETH", 18),
    (
        address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        "USDC",
        6,
    ),
    (
        address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
        "USDT",
        6,
    ),
    (
        address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
        "DAI",
        18,
    ),
    (
        address!("0xe71bdfe1df69284f00ee185cf0d95d0c7680c0d4"),
        "SETH",
        18,
    ),
];

/// `(symbol, decimals)` of a built-in mainnet token.
pub fn builtin_token(token: Address) -> Option<(&'static str, u8)> {
    BUILTIN_MAINNET_TOKENS
        .iter()
        .find(|(a, _, _)| *a == token)
        .map(|(_, s, d)| (*s, *d))
}

/// Universal Router command bytes (Commands.sol). The dispatcher masks each
/// byte with `COMMAND_TYPE_MASK` (0x3f); bit 7 is `FLAG_ALLOW_REVERT`.
pub const CMD_PERMIT2_TRANSFER_FROM: u8 = 0x02;
pub const CMD_PERMIT2_PERMIT_BATCH: u8 = 0x03;
pub const CMD_PERMIT2_PERMIT: u8 = 0x0a;
pub const CMD_PERMIT2_TRANSFER_FROM_BATCH: u8 = 0x0d;
pub const COMMAND_TYPE_MASK: u8 = 0x3f;

/// The string the parser should show for `amount` of a token with `decimals`
/// (`None` = not in the registry, so the raw integer is shown).
///
/// This is the same `format_units` call `format_token_amount_u256` makes, on the
/// full-width value. The pre-#346 code narrowed to `u128` first and showed "0"
/// for anything wider.
pub fn expected_amount(amount: U160, decimals: Option<u8>) -> String {
    match decimals {
        Some(d) => format_units(U256::from(amount), d).expect("decimals <= 77"),
        None => amount.to_string(),
    }
}

/// Undo `expected_amount`: parse a rendered amount back to the raw integer.
/// Used as an independent round-trip check (a rendering that drops or
/// truncates digits cannot parse back to the original value).
pub fn parse_rendered_amount(rendered: &str, decimals: Option<u8>) -> Option<U256> {
    match decimals {
        None => rendered.parse::<U256>().ok(),
        Some(d) => {
            let (int, frac) = rendered.split_once('.').unwrap_or((rendered, ""));
            // `format_units` with 0 decimals still prints one fractional
            // digit ("5.0"); anything past `d` must be zero padding.
            let (frac, pad) = frac.split_at(frac.len().min(usize::from(d)));
            if pad.bytes().any(|b| b != b'0') {
                return None;
            }
            let digits = format!("{int}{frac:0<width$}", width = usize::from(d));
            digits.parse::<U256>().ok()
        }
    }
}

/// Every display string in a serialized payload or field: labels, titles,
/// subtitles, texts and fallback texts.
pub fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

/// One Permit2 allowance or transfer amount that an independent decode says
/// the calldata carries.
#[derive(Debug, Clone)]
pub struct DecodedAmount {
    /// Which call or Universal Router command carries it.
    pub path: &'static str,
    pub token: Address,
    pub amount: U160,
}

/// Decode Permit2 amounts from calldata sent to Permit2 or to a Universal
/// Router, using only the reference ABI above. Returns the function name and
/// the amounts.
pub fn decode_reference(to: Address, calldata: &[u8]) -> (String, Vec<DecodedAmount>) {
    if to == PERMIT2 {
        let Ok(call) = P::abi_decode(calldata) else {
            let sel = calldata.get(..4).map(hex::encode).unwrap_or_default();
            return (format!("unknown 0x{sel}"), vec![]);
        };
        return match call {
            P::approve(c) => (
                "approve".into(),
                vec![DecodedAmount {
                    path: "approve",
                    token: c.token,
                    amount: c.amount,
                }],
            ),
            P::permit_0(c) => (
                "permit(PermitSingle)".into(),
                vec![DecodedAmount {
                    path: "permit",
                    token: c.permitSingle.details.token,
                    amount: c.permitSingle.details.amount,
                }],
            ),
            P::permit_1(c) => (
                format!("permit(PermitBatch[{}])", c.permitBatch.details.len()),
                c.permitBatch
                    .details
                    .iter()
                    .map(|d| DecodedAmount {
                        path: "permitBatch",
                        token: d.token,
                        amount: d.amount,
                    })
                    .collect(),
            ),
            P::transferFrom_0(c) => (
                "transferFrom".into(),
                vec![DecodedAmount {
                    path: "transferFrom",
                    token: c.token,
                    amount: c.amount,
                }],
            ),
            P::transferFrom_1(c) => (
                format!("transferFrom(batch[{}])", c.transferDetails.len()),
                c.transferDetails
                    .iter()
                    .map(|d| DecodedAmount {
                        path: "transferFromBatch",
                        token: d.token,
                        amount: d.amount,
                    })
                    .collect(),
            ),
            P::lockdown(c) => (format!("lockdown[{}]", c.approvals.len()), vec![]),
            P::invalidateNonces(_) => ("invalidateNonces".into(), vec![]),
            // uint256 amounts, outside the AllowanceTransfer code under test.
            P::permitTransferFrom(_) => ("permitTransferFrom".into(), vec![]),
            P::invalidateUnorderedNonces(_) => ("invalidateUnorderedNonces".into(), vec![]),
        };
    }

    let (commands, inputs) = match U::abi_decode(calldata) {
        Ok(U::execute_0(c)) => (c.commands, c.inputs),
        Ok(U::execute_1(c)) => (c.commands, c.inputs),
        Err(_) => return ("unknown".into(), vec![]),
    };
    let mut out = vec![];
    for (i, raw) in commands.iter().enumerate() {
        let Some(input) = inputs.get(i) else { continue };
        match raw & COMMAND_TYPE_MASK {
            CMD_PERMIT2_PERMIT => {
                if let Ok((p, _sig)) = <(PermitSingle, Bytes)>::abi_decode_params(input) {
                    out.push(DecodedAmount {
                        path: "UR PERMIT2_PERMIT",
                        token: p.details.token,
                        amount: p.details.amount,
                    });
                }
            }
            CMD_PERMIT2_PERMIT_BATCH => {
                if let Ok((p, _sig)) = <(PermitBatch, Bytes)>::abi_decode_params(input) {
                    out.extend(p.details.iter().map(|d| DecodedAmount {
                        path: "UR PERMIT2_PERMIT_BATCH",
                        token: d.token,
                        amount: d.amount,
                    }));
                }
            }
            CMD_PERMIT2_TRANSFER_FROM => {
                if let Ok((token, _recipient, amount)) =
                    <(Address, Address, U160)>::abi_decode_params(input)
                {
                    out.push(DecodedAmount {
                        path: "UR PERMIT2_TRANSFER_FROM",
                        token,
                        amount,
                    });
                }
            }
            CMD_PERMIT2_TRANSFER_FROM_BATCH => {
                if let Ok(details) = <Vec<AllowanceTransferDetails>>::abi_decode(input) {
                    out.extend(details.iter().map(|d| DecodedAmount {
                        path: "UR PERMIT2_TRANSFER_FROM_BATCH",
                        token: d.token,
                        amount: d.amount,
                    }));
                }
            }
            _ => {}
        }
    }
    (
        format!("execute(commands=0x{})", hex::encode(&commands)),
        out,
    )
}
