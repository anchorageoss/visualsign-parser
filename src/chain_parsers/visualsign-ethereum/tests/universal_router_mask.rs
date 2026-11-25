//! `COMMAND_TYPE_MASK` per Universal Router version, through the public entry point.
//!
//! The 6-bit routers (V1.2, V2.0) dispatch `0x40` as command `0x00`. The 7-bit routers
//! (V2.1.1, V2.2.0) dispatch it as `ACROSS_V4_DEPOSIT_V3`, a bridge deposit. The same
//! calldata must therefore render differently by the `to` address of the transaction.
//!
//! `REGISTERED` below is written out by hand from the verified `Commands.sol` of each
//! address, except the two Blast addresses. No verified source of those was found
//! (Sourcify has none, and Blast's explorer answers "Service Discontinued"), so their rows
//! follow Uniswap's `deploy-addresses/blast.json`, which names the deployments read on
//! other chains. `REGISTERED` does not use `RouterVersion`, so a wrong version in
//! `config.rs` fails here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::SolCall;
use serde_json::Value;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::protocols::uniswap::config::UniswapConfig;
use visualsign_ethereum::transaction_string_to_visual_sign;

use common::{IUniversalRouterRef, MAINNET_UNIVERSAL_ROUTERS, collect_strings};

const V1_2: usize = 0;
const V2_0: usize = 1;
const V2_1_1: usize = 2;
const V2_2_0: usize = 3;

/// What the deployed `Commands.sol` of a router does with the plan below.
#[derive(Clone, Copy, Debug)]
enum Runs {
    /// Mask `0x3f`, `0x10` to `0x14` are SEAPORT_V1_5, LOOKS_RARE_V2, NFTX, CRYPTOPUNKS or none.
    SixBitSeaport,
    /// Mask `0x3f`, `0x10` to `0x14` are the V4 and position-manager commands.
    SixBitV4,
    /// Mask `0x7f`, `0x40` is `ACROSS_V4_DEPOSIT_V3`.
    SevenBit,
}

use Runs::{SevenBit, SixBitSeaport, SixBitV4};

/// Every registered (chain id, address) pair of `UniswapConfig`, with what its router runs.
const REGISTERED: [(u64, &str, Runs); 35] = [
    (
        1,
        "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
        SixBitSeaport,
    ),
    (1, "0x66a9893cc07d91d95644aedd05d03f95e1dba8af", SixBitV4),
    (1, "0x4C82D1fBFe28C977cBB58D8C7FF8FCF9F70a2cCA", SevenBit),
    (1, "0xCb640A86855f1A828c27241bA364348de28abe66", SevenBit),
    (
        10,
        "0xCb1355ff08Ab38bBCE60111F1bb2B784bE25D7e8",
        SixBitSeaport,
    ),
    (10, "0x851116d9223fabed8e56c0e6b8ad0c31d98b3507", SixBitV4),
    (10, "0x8B844f885672f333Bc0042cB669255f93a4C1E6b", SevenBit),
    (
        56,
        "0x4Dae2f939ACf50408e13d58534Ff8c2776d45265",
        SixBitSeaport,
    ),
    (56, "0x1906c1d672b88cd1b9ac7593301ca990f94eae07", SixBitV4),
    (56, "0x8B844f885672f333Bc0042cB669255f93a4C1E6b", SevenBit),
    (
        137,
        "0xec7BE89e9d109e7e3Fec59c222CF297125FEFda2",
        SixBitSeaport,
    ),
    (137, "0x1095692a6237d83c6a72f3f5efedb9a670c49223", SixBitV4),
    (137, "0x8B844f885672f333Bc0042cB669255f93a4C1E6b", SevenBit),
    (
        480,
        "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
        SixBitSeaport,
    ),
    (480, "0x8ac7bee993bb44dab564ea4bc9ea67bf9eb5e743", SixBitV4),
    (480, "0x8B844f885672f333Bc0042cB669255f93a4C1E6b", SevenBit),
    (
        8453,
        "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
        SixBitSeaport,
    ),
    (8453, "0x6ff5693b99212da76ad316178a184ab56d299b43", SixBitV4),
    (8453, "0xfdf682f51fe81aa4898f0ae2163d8a55c127fbc7", SevenBit),
    (
        42161,
        "0x5E325eDA8064b456f4781070C0738d849c824258",
        SixBitSeaport,
    ),
    (
        42161,
        "0xa51afafe0263b40edaef0df8781ea9aa03e381a3",
        SixBitV4,
    ),
    (
        42161,
        "0x8B844f885672f333Bc0042cB669255f93a4C1E6b",
        SevenBit,
    ),
    (
        42220,
        "0x643770e279d5d0733f21d6dc03a8efbabf3255b4",
        SixBitSeaport,
    ),
    (
        42220,
        "0xcb695bc5D3Aa22cAD1E6DF07801b061a05A0233A",
        SixBitV4,
    ),
    (
        42220,
        "0x8B844f885672f333Bc0042cB669255f93a4C1E6b",
        SevenBit,
    ),
    (
        43114,
        "0x4Dae2f939ACf50408e13d58534Ff8c2776d45265",
        SixBitSeaport,
    ),
    (
        43114,
        "0x94b75331ae8d42c1b61065089b7d48fe14aa73b7",
        SixBitV4,
    ),
    (
        43114,
        "0x8B844f885672f333Bc0042cB669255f93a4C1E6b",
        SevenBit,
    ),
    (
        81457,
        "0x643770E279d5D0733F21d6DC03A8efbABf3255B4",
        SixBitSeaport,
    ),
    (
        81457,
        "0xeabbcb3e8e415306207ef514f660a3f820025be3",
        SixBitV4,
    ),
    (
        81457,
        "0x8B844f885672f333Bc0042cB669255f93a4C1E6b",
        SevenBit,
    ),
    (
        11155111,
        "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD",
        SixBitSeaport,
    ),
    (
        11155111,
        "0x3a9d48ab9751398bbfa63ad67599bb04e4bdf98b",
        SixBitV4,
    ),
    (
        11155111,
        "0x8B844f885672f333Bc0042cB669255f93a4C1E6b",
        SevenBit,
    ),
    (
        11155111,
        "0xB0C89059d7190EDb17eFF19829cc009cEe923916",
        SevenBit,
    ),
];

/// `0x40`, `0x10` to `0x14`, `0x21`, then `0xc0` (`0x40` with allow-revert).
const PLAN: [u8; 8] = [0x40, 0x10, 0x11, 0x12, 0x13, 0x14, 0x21, 0xc0];

/// The exact summary line a wallet shows for `PLAN` sent to a router that runs `runs`.
fn expected_summary(runs: Runs) -> String {
    let (first, last, middle) = match runs {
        SixBitSeaport => (
            "V3SwapExactIn",
            "V3SwapExactIn (allow revert)",
            "Unknown command 0x10, Unknown command 0x11, Unknown command 0x12, \
             Unknown command 0x13, Unknown command 0x14",
        ),
        SixBitV4 => (
            "V3SwapExactIn",
            "V3SwapExactIn (allow revert)",
            "V4Swap, V3PositionManagerPermit, V3PositionManagerCall, V4InitializePool, \
             V4PositionManagerCall",
        ),
        SevenBit => (
            "AcrossV4DepositV3",
            "AcrossV4DepositV3 (allow revert)",
            "V4Swap, V3PositionManagerPermit, V3PositionManagerCall, V4InitializePool, \
             V4PositionManagerCall",
        ),
    };
    format!(
        "Uniswap Universal Router Execute: 8 commands ([{first}, {middle}, ExecuteSubPlan, {last}])"
    )
}

/// Which `execute` form carries the calldata.
#[derive(Clone, Copy, Debug)]
enum Form {
    /// `execute(bytes,bytes[],uint256)`
    WithDeadline,
    /// `execute(bytes,bytes[])`
    NoDeadline,
}

fn rendered_strings_on(chain_id: u64, router: Address, commands: &[u8], form: Form) -> Vec<String> {
    let commands = Bytes::from(commands.to_vec());
    let inputs: Vec<Bytes> = commands.iter().map(|_| Bytes::from(vec![0xaa])).collect();
    let data = match form {
        Form::WithDeadline => IUniversalRouterRef::execute_0Call {
            commands,
            inputs,
            deadline: U256::from(0u64),
        }
        .abi_encode(),
        Form::NoDeadline => IUniversalRouterRef::execute_1Call { commands, inputs }.abi_encode(),
    };
    let input = serde_json::json!({
        "type": "transaction",
        "to": router.to_string(),
        "chainId": format!("0x{chain_id:x}"),
        "data": format!("0x{}", hex::encode(data)),
    })
    .to_string();
    let payload = transaction_string_to_visual_sign(
        &input,
        VisualSignOptions {
            include_intermediate_output: false,
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
        },
    )
    .unwrap();
    let json: Value = serde_json::from_str(&payload.to_json().unwrap()).unwrap();
    let mut strings = vec![];
    collect_strings(&json, &mut strings);
    strings
}

fn rendered_strings(router: Address, commands: &[u8]) -> Vec<String> {
    rendered_strings_on(1, router, commands, Form::WithDeadline)
}

fn mentions(strings: &[String], needle: &str) -> bool {
    strings.iter().any(|s| s.contains(needle))
}

#[test]
fn registered_table_is_the_registered_set() {
    let from_config: BTreeSet<(u64, Address)> = UniswapConfig::universal_router_chains()
        .iter()
        .flat_map(|chain| {
            UniswapConfig::universal_router_addresses(*chain)
                .unwrap()
                .into_iter()
                .map(|address| (*chain, address))
        })
        .collect();
    let from_table: BTreeSet<(u64, Address)> = REGISTERED
        .iter()
        .map(|(chain, address, _)| (*chain, address.parse().unwrap()))
        .collect();
    assert_eq!(REGISTERED.len(), 35);
    assert_eq!(from_table.len(), 35, "the table repeats a pair");
    assert_eq!(from_table, from_config);
}

#[test]
fn every_registered_router_shows_what_it_runs_in_both_execute_forms() {
    for (chain_id, address, runs) in REGISTERED {
        let router: Address = address.parse().unwrap();
        for form in [Form::WithDeadline, Form::NoDeadline] {
            let strings = rendered_strings_on(chain_id, router, &PLAN, form);
            let expected = expected_summary(runs);
            assert!(
                strings.iter().any(|s| s == &expected),
                "chain {chain_id}, router {address}, {runs:?}, {form:?}: expected {expected:?} in {strings:?}"
            );
        }
    }
}

#[test]
fn v2_1_1_router_0x40_is_not_shown_as_a_swap() {
    for index in [V2_1_1, V2_2_0] {
        let strings = rendered_strings(MAINNET_UNIVERSAL_ROUTERS[index], &[0x40, 0xc0]);
        assert!(mentions(&strings, "AcrossV4DepositV3"), "{strings:?}");
        assert!(
            mentions(&strings, "AcrossV4DepositV3 (allow revert)"),
            "{strings:?}"
        );
        assert!(mentions(&strings, "0xaa"), "{strings:?}");
        assert!(!mentions(&strings, "V3SwapExactIn"), "{strings:?}");
    }
}

#[test]
fn same_calldata_to_6_bit_routers_is_shown_as_a_swap() {
    for index in [V1_2, V2_0] {
        let strings = rendered_strings(MAINNET_UNIVERSAL_ROUTERS[index], &[0x40, 0xc0]);
        assert!(mentions(&strings, "V3SwapExactIn"), "{strings:?}");
        assert!(
            mentions(&strings, "V3SwapExactIn (allow revert)"),
            "{strings:?}"
        );
        assert!(!mentions(&strings, "AcrossV4DepositV3"), "{strings:?}");
    }
}

#[test]
fn v1_2_router_0x20_is_unknown_not_command_0x00() {
    let strings = rendered_strings(MAINNET_UNIVERSAL_ROUTERS[V1_2], &[0x20]);
    assert!(mentions(&strings, "Unknown command 0x20"), "{strings:?}");
    assert!(mentions(&strings, "Input: 0xaa"), "{strings:?}");
    assert!(!mentions(&strings, "V3SwapExactIn"), "{strings:?}");
}

#[test]
fn v1_2_router_0x10_and_0x90_are_not_shown_as_v4_swap() {
    let strings = rendered_strings(MAINNET_UNIVERSAL_ROUTERS[V1_2], &[0x10, 0x90]);
    assert!(mentions(&strings, "Unknown command 0x10"), "{strings:?}");
    assert!(
        mentions(&strings, "Unknown command 0x10 (allow revert)"),
        "{strings:?}"
    );
    assert!(mentions(&strings, "0xaa"), "{strings:?}");
    assert!(!mentions(&strings, "V4Swap"), "{strings:?}");
}

#[test]
fn v2_0_router_0x10_is_still_v4_swap() {
    let strings = rendered_strings(MAINNET_UNIVERSAL_ROUTERS[V2_0], &[0x10]);
    assert!(mentions(&strings, "V4Swap"), "{strings:?}");
    assert!(!mentions(&strings, "Unknown command"), "{strings:?}");
}
