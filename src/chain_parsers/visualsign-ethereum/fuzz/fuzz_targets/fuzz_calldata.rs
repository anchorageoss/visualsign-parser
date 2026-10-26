#![no_main]

//! Calldata decoding, with the input steered towards Permit2 and the
//! Universal Router.
//!
//! Input layout: `[control][calldata...]`.
//! - `control & 0x03` picks the destination: Permit2, Universal Router V2.0,
//!   Universal Router V1.2, or an address the registry does not know.
//! - `control >> 2` (mod `SELECTORS.len() + 1`) optionally overwrites the
//!   first four calldata bytes with a Permit2 / Universal Router selector, so
//!   mutations past the selector reach the argument decoders. 0 keeps the
//!   calldata as is (the seed corpus is real mainnet calldata with control 0
//!   or 1).
//!
//! Each input is run through the Permit2 and Universal Router visualizers
//! directly and through the full transaction pipeline. The invariant is no
//! panic; libFuzzer's -rss_limit_mb / -malloc_limit_mb catch unbounded
//! allocation.

use libfuzzer_sys::fuzz_target;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::protocols::uniswap::{Permit2Visualizer, UniversalRouterVisualizer};
use visualsign_ethereum::registry::ContractRegistry;
use visualsign_ethereum::transaction_string_to_visual_sign;

const DESTINATIONS: [&str; 4] = [
    "0x000000000022D473030F116dDEE9F6B43aC78BA3", // Permit2
    "0x66a9893cc07d91d95644aedd05d03f95e1dba8af", // Universal Router V2.0
    "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD", // Universal Router V1.2
    "0x00000000000000000000000000000000000000f1", // unregistered
];

const SELECTORS: [[u8; 4]; 11] = [
    [0x87, 0x51, 0x7c, 0x45], // approve(address,address,uint160,uint48)
    [0x2b, 0x67, 0xb5, 0x70], // permit(address,PermitSingle,bytes)
    [0x2a, 0x2d, 0x80, 0xd1], // permit(address,PermitBatch,bytes)
    [0x36, 0xc7, 0x85, 0x16], // transferFrom(address,address,uint160,address)
    [0x0d, 0x58, 0xb1, 0xdb], // transferFrom(AllowanceTransferDetails[])
    [0xcc, 0x53, 0x28, 0x7f], // lockdown(TokenSpenderPair[])
    [0x65, 0xd9, 0x72, 0x3c], // invalidateNonces(address,address,uint48)
    [0x30, 0xf2, 0x8b, 0x7a], // permitTransferFrom(...)
    [0x3f, 0xf9, 0xdc, 0xb1], // invalidateUnorderedNonces(uint256,uint256)
    [0x35, 0x93, 0x56, 0x4c], // execute(bytes,bytes[],uint256)
    [0x24, 0x85, 0x6b, 0xc3], // execute(bytes,bytes[])
];

fuzz_target!(|data: &[u8]| {
    let Some((&control, rest)) = data.split_first() else {
        return;
    };
    let mut calldata = rest.to_vec();
    let pick = usize::from(control >> 2) % (SELECTORS.len() + 1);
    if pick > 0 && calldata.len() >= 4 {
        calldata[..4].copy_from_slice(&SELECTORS[pick - 1]);
    }

    let (registry, _) = ContractRegistry::with_default_protocols();
    let _ = Permit2Visualizer.visualize_tx_commands(&calldata, 1, Some(&registry));
    let _ = UniversalRouterVisualizer {}.visualize_tx_commands(&calldata, 1, Some(&registry));

    let to = DESTINATIONS[usize::from(control & 0x03)];
    let tx = serde_json::json!({
        "type": "transaction",
        "to": to,
        "chainId": "0x1",
        "data": format!("0x{}", hex::encode(&calldata)),
    })
    .to_string();
    let options = VisualSignOptions {
        decode_transfers: true,
        ..VisualSignOptions::default()
    };
    let _ = transaction_string_to_visual_sign(&tx, options);
});
