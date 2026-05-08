#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Surfpool-backed integration tests for the Solana visual-sign parser.
//!
//! Tests are network-bound (start a `surfpool` mainnet fork; require the
//! `surfpool` binary on `$PATH`) and are therefore `#[ignore]`. The roundtrip
//! body and the `idl_test!` macro live in `tests/common/mod.rs` so other test
//! files (e.g. preset-specific surfpool tests) can reuse them.
//!
//! Run all surfpool tests:
//!
//! ```bash
//! HELIUS_API_KEY=<key> cargo test \
//!     --manifest-path src/Cargo.toml -p visualsign-solana \
//!     --test surfpool_fuzz -- --ignored --test-threads=1
//! ```
//!
//! Run a single IDL:
//!
//! ```bash
//! cargo test ... --test surfpool_fuzz surfpool_idl_jupiter -- --ignored
//! ```
//!
//! Adding a new IDL:
//! - Upstream `solana_parser::solana::embedded_idls::*`: add a `use` import and
//!   an `idl_test!(name, CONST)` line below.
//! - Vsp-local preset IDL (e.g. one added by the `solana-add-idl` skill):
//!   drop `<name>/<name>.json` into `src/presets/`. `build.rs` discovers it
//!   and `surfpool_preset_idls` (below) iterates it on every run -- no test-
//!   file edit required.

mod common;

use solana_parser::solana::embedded_idls::{
    APE_PRO_IDL, CANDY_MACHINE_IDL, DRIFT_IDL, JUPITER_AGG_V6_IDL, JUPITER_IDL, JUPITER_LIMIT_IDL,
    KAMINO_IDL, LIFINITY_IDL, METEORA_IDL, OPENBOOK_IDL, ORCA_IDL, RAYDIUM_IDL, STABBLE_IDL,
};
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};

/// Smoke test: start surfpool, verify the RPC responds, let `Drop` tear it down.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn surfpool_lifecycle() {
    let manager = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("surfpool should start");

    let client = manager.rpc_client();
    let version = client
        .get_version()
        .expect("RPC should respond with a version");

    assert!(
        !version.solana_core.is_empty(),
        "solana_core version string must not be empty"
    );
}

// `collision.json` and `cyclic.json` exist in `solana_parser`'s `idls/`
// directory but are negative test fixtures (duplicate type names / cyclic
// type refs); they're rejected by `decode_idl_data` and therefore not
// exposed via `embedded_idls`.

idl_test!(surfpool_idl_ape_pro, APE_PRO_IDL);
idl_test!(surfpool_idl_cndy, CANDY_MACHINE_IDL);
idl_test!(surfpool_idl_drift, DRIFT_IDL);
idl_test!(surfpool_idl_jupiter, JUPITER_IDL);
idl_test!(surfpool_idl_jupiter_agg_v6, JUPITER_AGG_V6_IDL);
idl_test!(surfpool_idl_jupiter_limit, JUPITER_LIMIT_IDL);
idl_test!(surfpool_idl_kamino, KAMINO_IDL);
idl_test!(surfpool_idl_lifinity, LIFINITY_IDL);
idl_test!(surfpool_idl_meteora, METEORA_IDL);
idl_test!(surfpool_idl_openbook, OPENBOOK_IDL);
idl_test!(surfpool_idl_orca, ORCA_IDL);
idl_test!(surfpool_idl_raydium, RAYDIUM_IDL);
idl_test!(surfpool_idl_stabble, STABBLE_IDL);

/// Auto-discovered preset IDLs: every `src/presets/<name>/<name>.json` file
/// that `build.rs` finds is exercised here through the same roundtrip used
/// by the named `idl_test!` invocations above. The skill (and any future
/// contributor) only needs to drop the JSON file -- this test picks it up
/// without any code edit. Empty when no presets ship an IDL JSON.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn surfpool_preset_idls() {
    if visualsign_solana::PRESET_IDLS.is_empty() {
        // Nothing to do: no preset has an embedded IDL JSON yet. Don't fail
        // the test -- it'd be an unhelpful red whenever the upstream stack
        // ships before the first preset IDL lands.
        return;
    }
    for (name, idl_json) in visualsign_solana::PRESET_IDLS {
        common::run_idl_roundtrip(&format!("preset_{name}"), idl_json).await;
    }
}
