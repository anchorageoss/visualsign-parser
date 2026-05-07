#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Surfpool-backed integration tests for the Solana visual-sign parser.
//!
//! Tests are network-bound (start a `surfpool` mainnet fork; require the
//! `surfpool` binary on `$PATH`) and are therefore `#[ignore]`. The IDL
//! directory is resolved at build time via the `SOLANA_IDL_DIR` env var
//! emitted by `build.rs`, so the test binary needs no runtime cargo lookup.
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
//! Adding a new IDL: drop the `.json` file into `solana_parser/src/solana/idls`
//! and add an `idl_test!(...)` line below; cargo's harness picks it up.

mod common;

use common::{build_transaction, options_with_idl};
use solana_parser::decode_idl_data;
use solana_sdk::pubkey::Pubkey;
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};
use std::path::PathBuf;
use visualsign::vsptrait::{Transaction, VisualSignConverter};
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

const IDL_DIR: &str = env!("SOLANA_IDL_DIR");

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

/// Per-IDL roundtrip: load the IDL, build a synthetic transaction whose data
/// starts with the first instruction's discriminator, run it through the
/// visual-sign converter, and assert the payload is non-empty.
async fn run_idl_roundtrip(idl_filename: &str) {
    let idl_path = PathBuf::from(IDL_DIR).join(idl_filename);
    let idl_json = std::fs::read_to_string(&idl_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", idl_path.display()));

    // Distinguish the three failure modes explicitly so a red test names the
    // IDL file and the actual cause (decode rejection from a malformed IDL,
    // empty instruction list, or a missing discriminator).
    let idl = decode_idl_data(&idl_json)
        .unwrap_or_else(|e| panic!("{idl_filename}: decode_idl_data rejected the IDL: {e}"));
    assert!(
        !idl.instructions.is_empty(),
        "{idl_filename}: IDL has no instructions"
    );
    let disc = idl.instructions[0]
        .discriminator
        .as_ref()
        .unwrap_or_else(|| panic!("{idl_filename}: instructions[0] has no discriminator"));
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

    let options = options_with_idl(&program_id, &idl_json, "test_program");
    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(wrapper, options)
        .expect("converter should succeed");

    assert!(
        !payload.fields.is_empty(),
        "payload must contain at least one field"
    );
}

macro_rules! idl_test {
    ($name:ident, $file:literal) => {
        #[tokio::test(flavor = "multi_thread")]
        #[ignore]
        async fn $name() {
            run_idl_roundtrip($file).await;
        }
    };
}

// `collision.json` and `cyclic.json` are solana_parser's negative test fixtures
// (duplicate type names / cyclic type refs); they're rejected by
// `decode_idl_data` and therefore excluded from this positive-path suite.

idl_test!(surfpool_idl_ape_pro, "ape_pro.json");
idl_test!(surfpool_idl_cndy, "cndy.json");
idl_test!(surfpool_idl_drift, "drift.json");
idl_test!(surfpool_idl_jupiter, "jupiter.json");
idl_test!(surfpool_idl_jupiter_agg_v6, "jupiter_agg_v6.json");
idl_test!(surfpool_idl_jupiter_limit, "jupiter_limit.json");
idl_test!(surfpool_idl_kamino, "kamino.json");
idl_test!(surfpool_idl_lifinity, "lifinity.json");
idl_test!(surfpool_idl_meteora, "meteora.json");
idl_test!(surfpool_idl_openbook, "openbook.json");
idl_test!(surfpool_idl_orca, "orca.json");
idl_test!(surfpool_idl_raydium, "raydium.json");
idl_test!(surfpool_idl_stabble, "stabble.json");
