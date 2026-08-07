#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Airdrop confirmation against a live surfpool fork.
//!
//! Network-bound: starts a `surfpool` mainnet fork and requires the `surfpool`
//! binary on `$PATH`, so the test is `#[ignore]` and runs on request only.
//! `HELIUS_API_KEY` is optional -- it upgrades the datasource off the
//! rate-limited public endpoint (see `SurfpoolConfig::default`).
//!
//! The confirmation outcomes themselves are covered offline in
//! `surfpool::manager`'s unit tests; this exercises the real RPC path.
//!
//! ```bash
//! cargo test -p solana_test_utils --test airdrop -- --ignored
//! ```

use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey::Pubkey;
use solana_test_utils::{SurfpoolConfig, SurfpoolManager};

/// `airdrop` returns only once the transaction confirms without a
/// `TransactionError`, and the requested lamports land on the account.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn airdrop_confirms_and_credits_the_account() {
    let manager = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .expect("surfpool should start");

    let target = Pubkey::new_unique();
    let client = manager.rpc_client();

    // `Pubkey::new_unique` is a counter, not a random source, so this address is
    // the same on every run and the fork could already hold a balance for it.
    // Comparing the delta keeps the credited amount exact regardless.
    let before = client
        .get_balance(&target)
        .expect("balance query should succeed");

    let signature = manager
        .airdrop(&target, LAMPORTS_PER_SOL)
        .await
        .expect("airdrop should confirm");

    // `airdrop` returning Ok means the status was `Some(Ok(()))`; re-reading it
    // here pins that contract against the live fork rather than trusting the
    // return value alone.
    let status = client
        .get_signature_status(&signature)
        .expect("signature status query should succeed")
        .expect("status is available once airdrop() returns");
    assert!(
        status.is_ok(),
        "a confirmed airdrop must not carry a transaction error: {status:?}"
    );

    let after = client
        .get_balance(&target)
        .expect("balance query should succeed");
    assert_eq!(
        after - before,
        LAMPORTS_PER_SOL,
        "airdrop must credit exactly the requested lamports"
    );
}
