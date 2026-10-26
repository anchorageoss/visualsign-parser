#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Property tests for Permit2 amount rendering (#346).
//!
//! Calldata is built with `alloy_sol_types` encoders from the reference ABI in
//! `common`, never by hand, and rendered through the public visualizers:
//!
//! - `Permit2Visualizer` for direct Permit2 calls (`approve`, `permit`,
//!   `transferFrom`, and the batch variants the parser does not decode);
//! - `UniversalRouterVisualizer` for `execute` with a `PERMIT2_PERMIT` (0x0a)
//!   command, the path that reaches `decode_permit` through
//!   `decode_custom_permit_params` and that the unit tests do not pin;
//! - the full `transaction_string_to_visual_sign` pipeline, to cover the
//!   dispatch by `to` address.
//!
//! Amounts span the whole uint160 range, weighted towards the edges that broke
//! before (u128::MAX + 1, 2^159, U160::MAX). Default 256 cases per property;
//! override with `PROPTEST_CASES=N`.

mod common;

use alloy_primitives::aliases::U48;
use alloy_primitives::{Address, Bytes, U160, U256};
use alloy_sol_types::{SolCall, SolValue};
use proptest::prelude::*;
use serde_json::Value;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayloadField, SignablePayloadFieldPreviewLayout};
use visualsign_ethereum::protocols::uniswap::{Permit2Visualizer, UniversalRouterVisualizer};
use visualsign_ethereum::registry::ContractRegistry;
use visualsign_ethereum::token_metadata::{ErcStandard, TokenMetadata};
use visualsign_ethereum::transaction_string_to_visual_sign;

use common::{
    AllowanceTransferDetails, BUILTIN_MAINNET_TOKENS, CMD_PERMIT2_PERMIT, CMD_PERMIT2_PERMIT_BATCH,
    CMD_PERMIT2_TRANSFER_FROM_BATCH, IPermit2Ref, IUniversalRouterRef, MAINNET_UNIVERSAL_ROUTERS,
    PERMIT2, PermitBatch, PermitDetails, PermitSingle, WETH, builtin_token, collect_strings,
    expected_amount, expected_timestamp, parse_rendered_amount,
};

const CHAIN_ID: u64 = 1;

/// Tokens registered in the test registry on top of the built-in ones, chosen
/// for their decimals. The addresses are synthetic.
const TEST_TOKENS: &[(&str, &str, u8)] = &[
    ("0x00000000000000000000000000000000000000a6", "USDX", 6),
    ("0x00000000000000000000000000000000000000a8", "BTCX", 8),
    ("0x00000000000000000000000000000000000000a0", "NODEC", 0),
    ("0x00000000000000000000000000000000000000a2", "WIDE", 24),
    ("0x00000000000000000000000000000000000000b4", "HUGE", 77),
];

fn test_registry() -> ContractRegistry {
    let (mut registry, _) = ContractRegistry::with_default_protocols();
    for (addr, symbol, decimals) in TEST_TOKENS {
        registry
            .register_token(
                CHAIN_ID,
                TokenMetadata {
                    symbol: (*symbol).to_string(),
                    name: (*symbol).to_string(),
                    erc_standard: ErcStandard::Erc20,
                    contract_address: (*addr).to_string(),
                    decimals: *decimals,
                },
            )
            .unwrap();
    }
    registry
}

/// `(symbol, decimals)` the test registry holds for `token`, if any.
fn known(token: Address) -> Option<(String, u8)> {
    if let Some((symbol, decimals)) = builtin_token(token) {
        return Some((symbol.to_string(), decimals));
    }
    TEST_TOKENS
        .iter()
        .find(|(a, _, _)| a.parse::<Address>().unwrap() == token)
        .map(|(_, s, d)| ((*s).to_string(), *d))
}

// --- Strategies ---

fn u160_edges() -> Vec<U160> {
    let u128_max = U160::from(u128::MAX);
    vec![
        U160::ZERO,
        U160::from(1u8),
        U160::from(u64::MAX),
        u128_max,
        u128_max + U160::from(1u8),
        U160::from(1u8) << 159,
        U160::MAX - U160::from(1u8),
        U160::MAX,
    ]
}

/// Full-range uint160 with the historical edge cases weighted in.
fn arb_amount() -> impl Strategy<Value = U160> {
    prop_oneof![
        3 => prop::sample::select(u160_edges()),
        3 => any::<[u8; 20]>().prop_map(U160::from_be_bytes),
        2 => any::<u64>().prop_map(U160::from),
        2 => any::<u128>().prop_map(U160::from),
    ]
}

fn timestamp_edges() -> Vec<u64> {
    vec![
        common::MAX_FOUR_DIGIT_YEAR_TS,
        common::MAX_FOUR_DIGIT_YEAR_TS + 1,
        common::CHRONO_MAX_TS,
        common::CHRONO_MAX_TS + 1,
    ]
}

fn arb_u48() -> impl Strategy<Value = U48> {
    let max = (1u64 << 48) - 1;
    prop_oneof![
        1 => Just(U48::ZERO),
        1 => Just(U48::from(max)),
        // Edges of the parser's timestamp rendering: four-digit years, signed
        // years up to chrono's maximum, and `unix:` beyond it.
        1 => prop::sample::select(timestamp_edges()).prop_map(U48::from),
        3 => (0..=max).prop_map(U48::from),
        3 => (0..=common::MAX_FOUR_DIGIT_YEAR_TS).prop_map(U48::from),
        1 => (common::MAX_FOUR_DIGIT_YEAR_TS..=common::CHRONO_MAX_TS).prop_map(U48::from),
    ]
}

fn arb_address() -> impl Strategy<Value = Address> {
    any::<[u8; 20]>().prop_map(Address::from)
}

/// A registry token (built-in or one of `TEST_TOKENS`) or an arbitrary one.
fn arb_token() -> impl Strategy<Value = Address> {
    let mut known: Vec<Address> = TEST_TOKENS
        .iter()
        .map(|(a, _, _)| a.parse().unwrap())
        .collect();
    known.extend(BUILTIN_MAINNET_TOKENS.iter().map(|(a, _, _)| *a));
    prop_oneof![
        1 => prop::sample::select(known),
        1 => arb_address(),
    ]
}

prop_compose! {
    fn arb_details()(
        token in arb_token(),
        amount in arb_amount(),
        expiration in arb_u48(),
        nonce in arb_u48(),
    ) -> PermitDetails {
        PermitDetails { token, amount, expiration, nonce }
    }
}

/// `sigDeadline` is a uint256: mostly u64 values (including the u64::MAX
/// "never" sentinel), sometimes wider than u64.
fn arb_sig_deadline() -> impl Strategy<Value = U256> {
    prop_oneof![
        1 => Just(U256::from(u64::MAX)),
        6 => any::<u64>().prop_map(U256::from),
        6 => (0..=common::MAX_FOUR_DIGIT_YEAR_TS).prop_map(U256::from),
        1 => prop::sample::select(timestamp_edges()).prop_map(U256::from),
        1 => (common::MAX_FOUR_DIGIT_YEAR_TS..=common::CHRONO_MAX_TS).prop_map(U256::from),
        2 => any::<[u8; 32]>().prop_map(|b| U256::from_be_bytes(b) | (U256::from(1u8) << 64)),
    ]
}

prop_compose! {
    fn arb_permit_single()(
        details in arb_details(),
        spender in arb_address(),
        sig_deadline in arb_sig_deadline(),
    ) -> PermitSingle {
        PermitSingle { details, spender, sigDeadline: sig_deadline }
    }
}

fn arb_signature() -> impl Strategy<Value = Bytes> {
    prop::collection::vec(any::<u8>(), 0..=65).prop_map(Bytes::from)
}

// --- Expectations ---

/// Amount string the parser should show, and the symbol it should use.
fn expected_for(token: Address, amount: U160) -> (String, String) {
    match known(token) {
        Some((symbol, decimals)) => (expected_amount(amount, Some(decimals)), symbol),
        None => (expected_amount(amount, None), format!("{token:?}")),
    }
}

/// The `permit` subtitle: "Unlimited Amount" if and only if the amount is
/// `type(uint160).max`, the scaled amount otherwise.
fn expected_permit_subtitle(p: &PermitSingle) -> String {
    let shown = if p.details.amount == U160::MAX {
        "Unlimited Amount".to_string()
    } else {
        expected_for(p.details.token, p.details.amount).0
    };
    let token = p.details.token.to_string().to_lowercase();
    format!("Permit {} to spend {shown} of {token}", p.spender)
}

fn text_v2(field: &SignablePayloadField) -> Option<(&str, &str)> {
    match field {
        SignablePayloadField::TextV2 { common, text_v2 } => {
            Some((common.label.as_str(), text_v2.text.as_str()))
        }
        _ => None,
    }
}

fn preview(field: &SignablePayloadField) -> Option<(&str, &SignablePayloadFieldPreviewLayout)> {
    match field {
        SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } => Some((common.label.as_str(), preview_layout)),
        _ => None,
    }
}

/// The `(label, text)` pairs of a preview layout's expanded TextV2 fields.
fn expanded_texts(layout: &SignablePayloadFieldPreviewLayout) -> Vec<(String, String)> {
    layout
        .expanded
        .iter()
        .flat_map(|l| l.fields.iter())
        .filter_map(|f| text_v2(&f.signable_payload_field))
        .map(|(l, t)| (l.to_string(), t.to_string()))
        .collect()
}

/// Check a rendered `permit` against the PermitSingle it was built from.
fn assert_permit_field(
    field: &SignablePayloadField,
    p: &PermitSingle,
) -> Result<(), TestCaseError> {
    let (label, layout) = preview(field).ok_or_else(|| {
        TestCaseError::fail(format!(
            "permit should render as a PreviewLayout: {field:?}"
        ))
    })?;
    prop_assert_eq!(label, "Permit2 Permit");
    let subtitle = layout
        .subtitle
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default();
    prop_assert_eq!(&subtitle, &expected_permit_subtitle(p));

    let fields = expanded_texts(layout);
    let get = |name: &str| {
        fields
            .iter()
            .find(|(l, _)| l == name)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    };
    // The expanded Amount is the raw integer, so it round-trips exactly.
    prop_assert_eq!(get("Amount").parse::<U160>().ok(), Some(p.details.amount));
    prop_assert_eq!(get("Token").parse::<Address>().ok(), Some(p.details.token));
    prop_assert_eq!(get("Spender").parse::<Address>().ok(), Some(p.spender));
    assert_timestamp(&get("Expires"), p.details.expiration.to::<u64>())?;
    match u64::try_from(p.sigDeadline) {
        Ok(secs) => assert_timestamp(&get("Sig Deadline"), secs)?,
        Err(_) => prop_assert_eq!(get("Sig Deadline"), format!("unix:{}", p.sigDeadline)),
    }
    // The scaled amount in the rendered subtitle parses back to the raw amount.
    if p.details.amount != U160::MAX {
        let decimals = known(p.details.token).map(|(_, d)| d);
        let rendered = subtitle
            .split_once(" to spend ")
            .and_then(|(_, rest)| rest.rsplit_once(" of "))
            .map(|(amount, _)| amount);
        prop_assert_eq!(
            rendered.and_then(|a| parse_rendered_amount(a, decimals)),
            Some(U256::from(p.details.amount))
        );
    }
    Ok(())
}

/// A rendered timestamp matches the independent formatter.
fn assert_timestamp(rendered: &str, secs: u64) -> Result<(), TestCaseError> {
    let want = expected_timestamp(secs);
    prop_assert_eq!(rendered, want.as_str());
    Ok(())
}

fn ur_execute(commands: Vec<u8>, inputs: Vec<Bytes>, deadline: Option<u64>) -> Vec<u8> {
    match deadline {
        Some(d) => IUniversalRouterRef::execute_0Call {
            commands: commands.into(),
            inputs,
            deadline: U256::from(d),
        }
        .abi_encode(),
        None => IUniversalRouterRef::execute_1Call {
            commands: commands.into(),
            inputs,
        }
        .abi_encode(),
    }
}

/// Every display string of the full-pipeline payload.
fn pipeline_strings(to: Address, calldata: &[u8]) -> Vec<String> {
    let input = serde_json::json!({
        "type": "transaction",
        "to": to.to_string(),
        "chainId": "0x1",
        "data": format!("0x{}", hex::encode(calldata)),
    })
    .to_string();
    let options = VisualSignOptions {
        include_intermediate_output: false,
        decode_transfers: true,
        transaction_name: None,
        metadata: None,
        developer_config: None,
    };
    let payload = transaction_string_to_visual_sign(&input, options).unwrap();
    let json: Value = serde_json::from_str(&payload.to_json().unwrap()).unwrap();
    let mut out = vec![];
    collect_strings(&json, &mut out);
    out
}

/// The visualizer must not invent a PermitSingle: if it renders a "Permit2
/// Permit" for a batch call, the token and amount shown must be ones the batch
/// actually carries.
fn assert_no_fabricated_permit(
    field: Option<SignablePayloadField>,
    carried: &[(Address, U160)],
) -> Result<(), TestCaseError> {
    let Some(field) = field else { return Ok(()) };
    let Some((label, layout)) = preview(&field) else {
        return Ok(());
    };
    if label != "Permit2 Permit" {
        return Ok(());
    }
    let fields = expanded_texts(layout);
    let token = fields
        .iter()
        .find(|(l, _)| l == "Token")
        .and_then(|(_, t)| t.parse::<Address>().ok());
    let amount = fields
        .iter()
        .find(|(l, _)| l == "Amount")
        .and_then(|(_, t)| t.parse::<U160>().ok());
    prop_assert!(
        carried
            .iter()
            .any(|(t, a)| Some(*t) == token && Some(*a) == amount),
        "rendered a Permit2 Permit for token {token:?} amount {amount:?} that the call does not carry: {carried:?}"
    );
    Ok(())
}

/// Hand-pinned amount strings, so the `format_units` oracle the properties
/// share with the parser is itself checked against literals.
#[test]
fn expected_amount_literals() {
    let two_pow_159 = U160::from(1u8) << 159;
    let cases = [
        (
            two_pow_159,
            18,
            "730750818665451459101842416358.141509827966271488",
        ),
        (
            two_pow_159,
            6,
            "730750818665451459101842416358141509827966.271488",
        ),
        (
            U160::MAX,
            18,
            "1461501637330902918203684832716.283019655932542975",
        ),
        (
            U160::from(u128::MAX) + U160::from(1u8),
            6,
            "340282366920938463463374607431768.211456",
        ),
        (
            U160::MAX - U160::from(1u8),
            77,
            "0.00000000000000000000000000001461501637330902918203684832716283019655932542974",
        ),
    ];
    for (amount, decimals, want) in cases {
        assert_eq!(expected_amount(amount, Some(decimals)), want);
        assert_eq!(
            parse_rendered_amount(want, Some(decimals)),
            Some(U256::from(amount))
        );
    }
    assert_eq!(
        expected_amount(two_pow_159, None),
        "730750818665451459101842416358141509827966271488"
    );
}

// --- Properties ---

proptest! {
    // Default 256 cases; override with PROPTEST_CASES=N.
    #![proptest_config(ProptestConfig::default())]

    /// `approve`: the full amount, scaled by the registry's decimals. The
    /// parser has no unlimited sentinel on `approve`, so U160::MAX renders as
    /// its number too.
    #[test]
    fn approve_renders_full_amount(
        token in arb_token(),
        spender in arb_address(),
        amount in arb_amount(),
        expiration in arb_u48(),
    ) {
        let registry = test_registry();
        let calldata = IPermit2Ref::approveCall { token, spender, amount, expiration }.abi_encode();
        let field = Permit2Visualizer
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("approve decodes");
        let (label, text) = text_v2(&field).expect("approve is TextV2");
        prop_assert_eq!(label, "Permit2 Approve");
        let (shown, symbol) = expected_for(token, amount);
        let prefix = format!("Approve {spender} {shown} {symbol} to spend {symbol} (expires: ");
        prop_assert!(text.starts_with(&prefix), "got {text:?}, want prefix {prefix:?}");
        prop_assert!(!text.contains("Unlimited"));
        let expires = text
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or_default();
        assert_timestamp(expires, expiration.to::<u64>())?;
        // Round trips: the spender and the amount parse back out of the text.
        let words: Vec<&str> = text.split(' ').collect();
        prop_assert_eq!(words[1].parse::<Address>().ok(), Some(spender));
        let decimals = known(token).map(|(_, d)| d);
        prop_assert_eq!(parse_rendered_amount(words[2], decimals), Some(U256::from(amount)));
    }

    /// `permit(owner, PermitSingle, sig)` called on Permit2 directly.
    #[test]
    fn permit_single_renders_amount_or_unlimited(
        owner in arb_address(),
        permit in arb_permit_single(),
        signature in arb_signature(),
    ) {
        let registry = test_registry();
        let calldata = IPermit2Ref::permit_0Call {
            owner,
            permitSingle: permit.clone(),
            signature,
        }
        .abi_encode();
        let field = Permit2Visualizer
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("permit decodes");
        assert_permit_field(&field, &permit)?;
    }

    /// Universal Router `execute` with a PERMIT2_PERMIT (0x0a) command. This
    /// reaches `decode_permit` through the inline-PermitSingle decoder, so it
    /// pins the U160::MAX sentinel on the path the unit tests miss.
    #[test]
    fn universal_router_permit2_permit_renders_amount_or_unlimited(
        permit in arb_permit_single(),
        signature in arb_signature(),
        deadline in prop::option::of(any::<u32>().prop_map(u64::from)),
    ) {
        let registry = test_registry();
        let input = (permit.clone(), signature).abi_encode_params();
        let calldata = ur_execute(vec![CMD_PERMIT2_PERMIT], vec![input.into()], deadline);
        let field = UniversalRouterVisualizer {}
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("execute decodes");
        let (_, layout) = preview(&field).expect("execute is a PreviewLayout");
        let commands = &layout.expanded.as_ref().expect("commands").fields;
        let permit_field = commands
            .iter()
            .map(|f| &f.signable_payload_field)
            .find(|f| preview(f).is_some_and(|(l, _)| l == "Permit2 Permit"))
            .expect("a Permit2 Permit command");
        assert_permit_field(permit_field, &permit)?;
    }

    /// `transferFrom(from, to, amount, token)` called on Permit2 directly.
    #[test]
    fn transfer_from_renders_full_amount(
        from in arb_address(),
        to in arb_address(),
        amount in arb_amount(),
        token in arb_token(),
    ) {
        let registry = test_registry();
        let calldata = IPermit2Ref::transferFrom_0Call { from, to, amount, token }.abi_encode();
        let field = Permit2Visualizer
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("transferFrom decodes");
        let (label, text) = text_v2(&field).expect("transferFrom is TextV2");
        prop_assert_eq!(label, "Permit2 Transfer");
        let (shown, symbol) = expected_for(token, amount);
        prop_assert_eq!(text, format!("Transfer {shown} {symbol} from {from} to {to}"));
        prop_assert!(!text.contains("Unlimited"));
    }

    /// Full pipeline, dispatched by `to`: Permit2 `permit` and `approve`, and a
    /// PERMIT2_PERMIT sent to any of the registered mainnet Universal Routers,
    /// through `transaction_string_to_visual_sign` with the built-in registry.
    #[test]
    fn pipeline_renders_permit2_amounts(
        permit in arb_permit_single(),
        weth in any::<bool>(),
        signature in arb_signature(),
        router in prop::sample::select(MAINNET_UNIVERSAL_ROUTERS.to_vec()),
    ) {
        let mut permit = permit;
        if weth {
            permit.details.token = WETH;
        } else if known(permit.details.token).is_some() && builtin_token(permit.details.token).is_none() {
            // The pipeline's registry does not hold the synthetic test tokens.
            permit.details.token = Address::repeat_byte(0x5a);
        }
        let subtitle = expected_permit_subtitle(&permit);

        let direct = IPermit2Ref::permit_0Call {
            owner: Address::repeat_byte(0x11),
            permitSingle: permit.clone(),
            signature: signature.clone(),
        }
        .abi_encode();
        let strings = pipeline_strings(PERMIT2, &direct);
        prop_assert!(strings.contains(&subtitle), "direct permit: want {subtitle:?}");

        let ur_input = (permit.clone(), signature).abi_encode_params();
        let ur = ur_execute(vec![CMD_PERMIT2_PERMIT], vec![ur_input.into()], Some(1_900_000_000));
        let strings = pipeline_strings(router, &ur);
        prop_assert!(strings.contains(&subtitle), "UR {router} permit: want {subtitle:?}");

        let d = &permit.details;
        let approve = IPermit2Ref::approveCall {
            token: d.token,
            spender: permit.spender,
            amount: d.amount,
            expiration: d.expiration,
        }
        .abi_encode();
        let (shown, symbol) = expected_for(d.token, d.amount);
        let prefix = format!("Approve {} {shown} {symbol} to spend", permit.spender);
        let strings = pipeline_strings(PERMIT2, &approve);
        prop_assert!(strings.iter().any(|s| s.starts_with(&prefix)), "approve: want {prefix:?}");
    }

    /// Universal Router PERMIT2_PERMIT_BATCH (0x03) and
    /// PERMIT2_TRANSFER_FROM_BATCH (0x0d) are not decoded yet: each shows its
    /// command name and the raw input hex. Pin that, and that neither is passed
    /// off as a PermitSingle, so a decoder for them has to update this test.
    // TODO: decode 0x03 and 0x0d, then assert their amounts here.
    #[test]
    fn universal_router_batch_commands_render_raw_input(
        details in prop::collection::vec(arb_details(), 1..=5),
        spender in arb_address(),
        signature in arb_signature(),
        transfers in prop::collection::vec(
            (arb_address(), arb_address(), arb_amount(), arb_token()),
            1..=5,
        ),
        batch_first in any::<bool>(),
    ) {
        let registry = test_registry();
        let permit_batch = PermitBatch {
            details,
            spender,
            sigDeadline: U256::from(1_900_000_000u64),
        };
        let permit_input = (permit_batch, signature).abi_encode_params();
        let transfer_input = transfers
            .into_iter()
            .map(|(from, to, amount, token)| AllowanceTransferDetails { from, to, amount, token })
            .collect::<Vec<_>>()
            .abi_encode();
        let mut commands = [
            (CMD_PERMIT2_PERMIT_BATCH, "Permit2PermitBatch", permit_input),
            (CMD_PERMIT2_TRANSFER_FROM_BATCH, "Permit2TransferFromBatch", transfer_input),
        ];
        if !batch_first {
            commands.reverse();
        }
        let calldata = ur_execute(
            commands.iter().map(|(c, _, _)| *c).collect(),
            commands.iter().map(|(_, _, i)| i.clone().into()).collect(),
            Some(1_900_000_000),
        );
        let field = UniversalRouterVisualizer {}
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("execute decodes");
        let (_, layout) = preview(&field).expect("execute is a PreviewLayout");
        let rendered = &layout.expanded.as_ref().expect("commands").fields;
        prop_assert_eq!(rendered.len(), commands.len() + 1, "commands plus the deadline");
        for (i, (_, name, input)) in commands.iter().enumerate() {
            let (label, cmd) = preview(&rendered[i].signable_payload_field)
                .expect("command is a PreviewLayout");
            prop_assert_eq!(label, format!("Command {}", i + 1));
            prop_assert_eq!(cmd.title.as_ref().map(|t| t.text.as_str()), Some(*name));
            let want = format!("Input: 0x{}", hex::encode(input));
            prop_assert_eq!(cmd.subtitle.as_ref().map(|t| t.text.as_str()), Some(want.as_str()));
        }
    }

    /// Batch Permit2 calls the parser does not decode must still render without
    /// panicking.
    #[test]
    fn batch_calls_never_panic(
        details in prop::collection::vec(arb_details(), 1..=5),
        spender in arb_address(),
        transfers in prop::collection::vec(
            (arb_address(), arb_address(), arb_amount(), arb_token()),
            1..=5,
        ),
        signature in arb_signature(),
    ) {
        let registry = test_registry();
        let permit_batch = IPermit2Ref::permit_1Call {
            owner: Address::repeat_byte(0x11),
            permitBatch: PermitBatch { details, spender, sigDeadline: U256::from(1_900_000_000u64) },
            signature,
        }
        .abi_encode();
        let _ = Permit2Visualizer.visualize_tx_commands(&permit_batch, CHAIN_ID, Some(&registry));

        let transfer_batch = IPermit2Ref::transferFrom_1Call {
            transferDetails: transfers
                .into_iter()
                .map(|(from, to, amount, token)| AllowanceTransferDetails { from, to, amount, token })
                .collect(),
        }
        .abi_encode();
        let _ = Permit2Visualizer.visualize_tx_commands(&transfer_batch, CHAIN_ID, Some(&registry));
    }

    /// Arbitrary bytes behind a Permit2 or Universal Router selector never panic.
    #[test]
    fn arbitrary_calldata_never_panics(
        selector in prop::sample::select(vec![
            IPermit2Ref::approveCall::SELECTOR,
            IPermit2Ref::permit_0Call::SELECTOR,
            IPermit2Ref::permit_1Call::SELECTOR,
            IPermit2Ref::transferFrom_0Call::SELECTOR,
            IPermit2Ref::transferFrom_1Call::SELECTOR,
            IPermit2Ref::lockdownCall::SELECTOR,
            IUniversalRouterRef::execute_0Call::SELECTOR,
            IUniversalRouterRef::execute_1Call::SELECTOR,
        ]),
        body in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let registry = test_registry();
        let mut calldata = selector.to_vec();
        calldata.extend(body);
        let _ = Permit2Visualizer.visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry));
        let _ = UniversalRouterVisualizer {}.visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    /// A `permit(owner, PermitBatch, sig)` must not render as a PermitSingle
    /// the call does not carry.
    #[test]
    #[ignore = "known bug #610: Permit2Visualizer's inline-PermitSingle fallback misreads PermitBatch calldata"]
    fn permit_batch_is_not_rendered_as_fabricated_permit(
        details in prop::collection::vec(arb_details(), 1..=5),
        spender in arb_address(),
        signature in arb_signature(),
    ) {
        let registry = test_registry();
        let carried: Vec<(Address, U160)> = details.iter().map(|d| (d.token, d.amount)).collect();
        let calldata = IPermit2Ref::permit_1Call {
            owner: Address::repeat_byte(0x11),
            permitBatch: PermitBatch { details, spender, sigDeadline: U256::from(1_900_000_000u64) },
            signature,
        }
        .abi_encode();
        let field = Permit2Visualizer.visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry));
        assert_no_fabricated_permit(field, &carried)?;
    }

    /// A batch `transferFrom(AllowanceTransferDetails[])` must not render as a
    /// PermitSingle.
    #[test]
    #[ignore = "known bug #610: Permit2Visualizer's inline-PermitSingle fallback misreads batch transferFrom calldata"]
    fn transfer_from_batch_is_not_rendered_as_fabricated_permit(
        transfers in prop::collection::vec(
            (arb_address(), arb_address(), arb_amount(), arb_token()),
            1..=5,
        ),
    ) {
        let registry = test_registry();
        let calldata = IPermit2Ref::transferFrom_1Call {
            transferDetails: transfers
                .into_iter()
                .map(|(from, to, amount, token)| AllowanceTransferDetails { from, to, amount, token })
                .collect(),
        }
        .abi_encode();
        let field = Permit2Visualizer.visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry));
        assert_no_fabricated_permit(field, &[])?;
    }

    /// PERMIT2_PERMIT with FLAG_ALLOW_REVERT (0x8a) still executes on chain
    /// (the dispatcher masks the command with 0x3f), so it must be shown.
    #[test]
    #[ignore = "known bug #611: Universal Router commands with FLAG_ALLOW_REVERT are dropped from the display"]
    fn universal_router_allow_revert_permit_is_shown(
        permit in arb_permit_single(),
        signature in arb_signature(),
    ) {
        let registry = test_registry();
        let input = (permit.clone(), signature).abi_encode_params();
        let calldata = ur_execute(vec![CMD_PERMIT2_PERMIT | 0x80], vec![input.into()], Some(1_900_000_000));
        let field = UniversalRouterVisualizer {}
            .visualize_tx_commands(&calldata, CHAIN_ID, Some(&registry))
            .expect("execute decodes");
        let (_, layout) = preview(&field).expect("execute is a PreviewLayout");
        let commands = &layout.expanded.as_ref().expect("commands").fields;
        let permit_field = commands
            .iter()
            .map(|f| &f.signable_payload_field)
            .find(|f| preview(f).is_some_and(|(l, _)| l == "Permit2 Permit"));
        prop_assert!(permit_field.is_some(), "allow-revert PERMIT2_PERMIT is not displayed");
        assert_permit_field(permit_field.unwrap(), &permit)?;
    }
}
