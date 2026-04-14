#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Surfpool-backed integration tests for the Solana visual-sign parser.
//!
//! These tests start a **surfpool** mainnet fork (requires the `surfpool`
//! binary on `$PATH` and network access) and exercise the parser against
//! transactions built from real on-chain state.
//!
//! All tests are `#[ignore]` — run them explicitly:
//!
//! ```bash
//! cargo test -p visualsign-solana --test surfpool_fuzz -- --ignored
//! ```

mod common;

use common::{build_disc_data, build_transaction, load_idl_from_env, options_with_idl};
use solana_sdk::pubkey::Pubkey;
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};
use visualsign::vsptrait::{Transaction, VisualSignConverter};
use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

// ── Tests ────────────────────────────────────────────────────────────────────

/// Smoke-test: start surfpool, verify the RPC endpoint responds with a
/// version string, then let `SurfpoolManager` tear it down on drop.
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

/// End-to-end: load a real IDL from `IDL_FILE`, extract the first
/// instruction's discriminator, build a transaction containing those bytes,
/// and run it through the visual-sign converter. The converter must return
/// `Ok` with at least one field (the instruction line).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn surfpool_jupiter_swap_roundtrip() {
    // Skip gracefully when IDL_FILE is not set.
    let (idl_json, _idl) = match load_idl_from_env() {
        Some(pair) => pair,
        None => {
            eprintln!("IDL_FILE not set or invalid -- skipping surfpool_jupiter_swap_roundtrip");
            return;
        }
    };

    // Start surfpool (validates that a local fork is healthy).
    let _manager = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("surfpool should start");

    // Build instruction data using the first instruction's discriminator.
    let inst_idx = 0;
    let arg_bytes: &[u8] = &[0u8; 32]; // arbitrary argument padding
    let (_parsed_idl, data) = build_disc_data(&idl_json, inst_idx, arg_bytes)
        .expect("IDL should have at least one instruction with a discriminator");

    // Use a unique program ID for the synthetic transaction.
    let program_name = "test_program";
    let program_id = Pubkey::new_unique();

    let tx = build_transaction(program_id, vec![Pubkey::new_unique()], data);

    // Serialize the transaction to base64 so we can round-trip through from_string.
    let tx_bytes = bincode::serialize(&tx).expect("transaction should serialize");
    let tx_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &tx_bytes);

    let wrapper = SolanaTransactionWrapper::from_string(&tx_b64)
        .expect("from_string should succeed for a valid base64 transaction");

    let options = options_with_idl(&program_id, &idl_json, program_name);

    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(wrapper, options)
        .expect("converter should succeed");

    assert!(
        !payload.fields.is_empty(),
        "payload must contain at least one field"
    );
}
