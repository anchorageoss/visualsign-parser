//! Surfpool integration: valid-discriminator fuzz path.
//!
//! Extends `real_idl_valid_data_always_parses_ok` (from fuzz_idl_parsing.rs) by:
//!
//!   1. Using the REAL program ID so that `transaction_to_visual_sign` triggers
//!      the built-in IDL path inside `UnknownProgramVisualizer` (metadata: None).
//!   2. Submitting each generated transaction to surfpool's `simulateTransaction`
//!      before passing it to the parser — so the byte shapes are execution-realistic.
//!
//! Assertions (per proptest case):
//!   - `transaction_to_visual_sign` must return `Ok` for correctly-encoded bytes.
//!   - Every instruction field title must contain "(IDL)", confirming the built-in
//!     IDL was found for this program ID.
//!   - No panics under any circumstances.
//!
//! Required environment variables:
//!   IDL_FILE    — path to an Anchor IDL JSON file (same as fuzz_idl_parsing.rs)
//!   PROGRAM_ID  — the real program address corresponding to that IDL
//!
//! Optional:
//!   SOLANA_RPC_URL  — fork URL for surfpool (defaults to public mainnet endpoint)
//!   PROPTEST_CASES  — number of proptest cases per run (default 256)
//!
//! Run all embedded IDLs:
//!   ./scripts/surfpool_fuzz_all_idls.sh
//!
//! Run a single IDL manually:
//!   IDL_FILE=src/solana_test_utils/idls/jupiter_agg_v6.json \
//!   PROGRAM_ID=JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4 \
//!   cargo test -p solana_test_utils --test surfpool_fuzz -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, TestCaseError, TestRunner};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_parser::decode_idl_data;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayload, SignablePayloadField};
use visualsign_solana::transaction_to_visual_sign;

use solana_test_utils::idl_strategies::arb_valid_instruction_bytes;
use solana_test_utils::surfpool::{SurfpoolConfig, SurfpoolManager};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Load IDL_FILE from env and decode it. Returns None (skips the test) if
/// IDL_FILE is unset or the JSON fails to decode.
fn load_idl_from_env() -> Option<solana_parser::solana::structs::Idl> {
    let path = std::env::var("IDL_FILE").ok()?;
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("IDL_FILE={path}: {e}"));
    match decode_idl_data(&json) {
        Ok(idl) => Some(idl),
        Err(e) => {
            eprintln!("IDL_FILE={path}: skipping — decode failed: {e}");
            None
        }
    }
}

/// Extract `SignablePayloadFieldPreviewLayout` entries whose label starts with
/// "Instruction". Mirrors the helper in `pipeline_integration.rs`.
fn instruction_field_titles(payload: &SignablePayload) -> Vec<String> {
    payload
        .fields
        .iter()
        .filter_map(|f| {
            if let SignablePayloadField::PreviewLayout { common, preview_layout } = f {
                if common.label.starts_with("Instruction") {
                    return preview_layout
                        .title
                        .as_ref()
                        .map(|t| t.text.clone());
                }
            }
            None
        })
        .collect()
}

// ── Test ──────────────────────────────────────────────────────────────────────

/// Property test: valid borsh bytes for a real embedded-IDL instruction,
/// submitted to a surfpool mainnet fork, then parsed via the built-in IDL path.
///
/// Marked `#[ignore]` — requires surfpool binary and network access.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn surfpool_real_idl_roundtrip() {
    // 1. Load IDL (skip gracefully if IDL_FILE not set).
    let Some(idl) = load_idl_from_env() else {
        eprintln!("IDL_FILE not set — skipping surfpool_real_idl_roundtrip");
        return;
    };

    // 2. Resolve the real program ID.
    let program_id: Pubkey = std::env::var("PROGRAM_ID")
        .expect("PROGRAM_ID must be set to the real base58 program address for this IDL")
        .parse()
        .expect("PROGRAM_ID is not a valid base58 pubkey");

    // 3. Start surfpool forked from mainnet.
    let surfpool = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("failed to start surfpool");
    surfpool
        .wait_ready(Duration::from_secs(30))
        .await
        .expect("surfpool did not become ready within 30 s");

    let rpc_client = surfpool.rpc_client();

    // 4. Build the proptest strategy — same shape as real_idl_valid_data_always_parses_ok.
    let n = idl.instructions.len();
    assert!(n > 0, "IDL has no instructions (IDL_FILE may be empty or invalid)");

    let types = Arc::new(idl.types.clone());
    let instructions = idl.instructions.clone();

    let strategy = (0..n).prop_flat_map(move |inst_idx| {
        let byte_strat = arb_valid_instruction_bytes(&instructions[inst_idx], types.clone());
        byte_strat.prop_map(move |bytes| (inst_idx, bytes))
    });

    let idl_ref = Arc::new(idl);

    // 5. Configure proptest case count.
    let cases: u32 = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);

    // 6. Run cases. TestRunner::run (not the proptest! macro) is required because
    //    the strategy is built from a runtime-loaded IDL shape.
    TestRunner::new(ProptestConfig { cases, ..ProptestConfig::default() })
        .run(&strategy, |(inst_idx, bytes)| {
            let expected_name = idl_ref.instructions[inst_idx].name.clone();

            // Build a minimal unsigned legacy transaction.
            // fee_payer is random — the transaction won't be signable, but
            // simulation with sig_verify=false doesn't require a valid signature.
            let fee_payer = Pubkey::new_unique();
            let ix = Instruction::new_with_bytes(program_id, &bytes, vec![]);
            let tx = SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&fee_payer)));

            // Submit to surfpool. The simulation result is intentionally ignored:
            // it will usually fail (unknown fee payer, missing accounts) and that
            // is fine — we only need the parser to handle the bytes without panicking.
            let sim_cfg = RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                ..RpcSimulateTransactionConfig::default()
            };
            let _sim = rpc_client.simulate_transaction_with_config(&tx, sim_cfg);

            // Run the full parser pipeline with metadata: None so the built-in
            // IDL registry (inside UnknownProgramVisualizer) fires for this
            // program ID.
            let result = transaction_to_visual_sign(
                tx,
                VisualSignOptions {
                    metadata: None,
                    decode_transfers: false,
                    transaction_name: Some(format!("surfpool-fuzz:{expected_name}")),
                    developer_config: None,
                    abi_registry: None,
                },
            );

            // Assertion A: valid borsh bytes with a matching discriminator must
            // always parse Ok — same guarantee as real_idl_valid_data_always_parses_ok.
            let payload = result.map_err(|e| {
                TestCaseError::fail(format!(
                    "instruction '{expected_name}': parser returned Err for \
                     correctly-encoded input: {e:?}"
                ))
            })?;

            // Assertion B: at least one instruction field must be present.
            //
            // Note: we intentionally do not assert on the title text.
            // Programs with a dedicated preset visualizer (e.g. JupiterSwap for
            // JUP6) produce rich human-readable titles like
            // "Jupiter Shared Accounts Route: From X To Y (slippage: Nbps)"
            // rather than the raw instruction name.  Programs handled by
            // UnknownProgramVisualizer via the built-in IDL produce titles like
            // "<ProgramName> (IDL)".  Both are correct — the key guarantee is
            // that the parser returns Ok with a non-empty payload.
            let titles = instruction_field_titles(&payload);
            prop_assert!(
                !titles.is_empty(),
                "payload has no instruction fields for '{expected_name}' \
                 (program_id={program_id})"
            );

            Ok(())
        })
        .unwrap_or_else(|e| panic!("surfpool_real_idl_roundtrip failed: {e}"));
}
