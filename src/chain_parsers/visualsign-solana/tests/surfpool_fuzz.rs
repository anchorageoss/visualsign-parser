#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Surfpool-backed integration tests for the Solana visual-sign parser.
//!
//! Tests are network-bound (start a `surfpool` mainnet fork; require the
//! `surfpool` binary on `$PATH`) and are therefore `#[ignore]`. Each test
//! references a `solana_parser::solana::embedded_idls::*` const directly,
//! so the IDL contents are baked in at compile time -- no filesystem
//! lookup, no env var, no `cargo metadata`.
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
//! Adding a new IDL: once it's exposed as a `pub const` in
//! `solana_parser::solana::embedded_idls`, add an `idl_test!(name, CONST)`
//! line below; cargo's harness picks it up.

mod common;

use common::{build_transaction, options_with_idl};
use solana_parser::decode_idl_data;
use solana_parser::solana::embedded_idls::{
    APE_PRO_IDL, CANDY_MACHINE_IDL, DRIFT_IDL, JUPITER_AGG_V6_IDL, JUPITER_IDL, JUPITER_LIMIT_IDL,
    KAMINO_IDL, LIFINITY_IDL, METEORA_IDL, OPENBOOK_IDL, ORCA_IDL, RAYDIUM_IDL, STABBLE_IDL,
};
use solana_sdk::pubkey::Pubkey;
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};
use visualsign::vsptrait::{Transaction, VisualSignConverter};
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

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

/// Per-IDL roundtrip: decode the IDL, build a synthetic transaction whose data
/// starts with the first instruction's discriminator, run it through the
/// visual-sign converter, and assert the payload is non-empty.
async fn run_idl_roundtrip(idl_label: &str, idl_json: &str) {
    // Distinguish the three failure modes explicitly so a red test names the
    // IDL and the actual cause (decode rejection from a malformed IDL, empty
    // instruction list, or a missing discriminator).
    let idl = decode_idl_data(idl_json)
        .unwrap_or_else(|e| panic!("{idl_label}: decode_idl_data rejected the IDL: {e}"));
    assert!(
        !idl.instructions.is_empty(),
        "{idl_label}: IDL has no instructions"
    );
    let disc = idl.instructions[0]
        .discriminator
        .as_ref()
        .unwrap_or_else(|| panic!("{idl_label}: instructions[0] has no discriminator"));
    let mut data = disc.clone();
    data.extend_from_slice(&[0u8; 32]);

    let _manager = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("surfpool should start");

    let program_id = Pubkey::new_unique();
    let tx = build_transaction(program_id, vec![Pubkey::new_unique()], data);
    let tx_bytes = bincode::serialize(&tx).expect("tx should serialize");
    let tx_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &tx_bytes);

    let wrapper = SolanaTransactionWrapper::from_string(&tx_b64)
        .expect("from_string should succeed for a valid base64 transaction");

    let options = options_with_idl(&program_id, idl_json, "test_program");
    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(wrapper, options)
        .expect("converter should succeed");

    assert!(
        !payload.fields.is_empty(),
        "payload must contain at least one field"
    );
}

macro_rules! idl_test {
    ($name:ident, $idl:expr) => {
        #[tokio::test(flavor = "multi_thread")]
        #[ignore]
        async fn $name() {
            run_idl_roundtrip(stringify!($name), $idl).await;
        }
    };
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
