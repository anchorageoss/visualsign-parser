//! End-to-end x402 gating against the **real** payai facilitator on Solana
//! **devnet**. Gated by `#[ignore]` AND `X402_E2E=1` so it stays out of the
//! default `cargo test` run.
//!
//! Run with:
//! ```sh
//! X402_E2E=1 cargo test -p integration --test x402_payai_devnet_test -- --ignored --nocapture
//! ```
//!
//! Requirements:
//! - Network egress to `https://facilitator.payai.network` + Solana devnet RPC.
//! - The committed fixture wallet (derived from
//!   `src/integration/fixtures/devnet/wallet.seed`) must be funded with at
//!   least `MIN_DEVNET_SOL` SOL and `MIN_DEVNET_USDC` USDC on devnet. Faucets:
//!   <https://faucet.solana.com> and <https://faucet.circle.com>.
//! - The gateway is built locally (`make -C src build` first).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/solana_x402_client.rs"]
mod solana_x402_client;

use base64::Engine;
use qos_p256::P256Pair;
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signer::Signer;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

use solana_x402_client::{
    PaymentRequirementsLite, build_payment_transaction, build_x_payment_header, load_devnet_keypair,
};

// Lock with the existing x402_gateway_test paths — they share ports 18080 /
// 18090 / 44020 and both touch the parser_grpc_server.
static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

const GW_PORT: u16 = 18180;
const PAYAI_FACILITATOR: &str = "https://facilitator.payai.network";
const DEVNET_RPC: &str = "https://api.devnet.solana.com";
const MIN_DEVNET_SOL_LAMPORTS: u64 = 50_000_000; // 0.05 SOL
const MIN_DEVNET_USDC_ATOMIC: u64 = 1_000_000; // 1.00 USDC
const PARSER_PRICE_USD: &str = "0.001"; // matches X402_NETWORK=solana-devnet default
const WALLET_SEED_PATH: &str = "fixtures/devnet/wallet.seed";

fn target_bin(name: &str) -> String {
    format!("../target/debug/{name}")
}

struct Procs {
    grpc: Child,
    gateway: Child,
}

impl Drop for Procs {
    fn drop(&mut self) {
        let _ = self.grpc.kill();
        let _ = self.gateway.kill();
        let _ = self.grpc.wait();
        let _ = self.gateway.wait();
    }
}

async fn wait_until_port_free(port: u16) {
    for _ in 0..100 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_ready(url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..200 {
        if let Ok(r) = client.get(url).send().await {
            if r.status().is_success() {
                return;
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("service at {url} never became ready (timed out after 20 s)");
}

async fn start_stack(receiver_b58: &str, tvc_pubkey_hex: &str) -> Procs {
    wait_until_port_free(44020).await;
    wait_until_port_free(GW_PORT).await;

    let grpc = Command::new(target_bin("parser_grpc_server"))
        .env("EPHEMERAL_FILE", "fixtures/ephemeral.secret")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parser_grpc_server");

    let gateway = Command::new(target_bin("parser_gateway"))
        .env("GATEWAY_PORT", GW_PORT.to_string())
        .env("GRPC_ADDR", "http://127.0.0.1:44020")
        .env("X402_PROFILE", "payai")
        .env("X402_FACILITATOR_URL", PAYAI_FACILITATOR)
        .env("X402_NETWORK", "solana-devnet")
        .env("X402_PAYTO", receiver_b58)
        .env("X402_TVC_VERIFIER_PUBKEY_HEX", tvc_pubkey_hex)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parser_gateway");

    wait_ready(&format!("http://127.0.0.1:{GW_PORT}/health")).await;
    Procs { grpc, gateway }
}

fn fixture_ephemeral_pubkey_hex() -> String {
    let pair = P256Pair::from_hex_file("fixtures/ephemeral.secret")
        .expect("load fixtures/ephemeral.secret");
    qos_hex::encode(&pair.public_key().to_bytes())
}

fn skip_unless_e2e() -> bool {
    if std::env::var("X402_E2E").as_deref().unwrap_or("") != "1" {
        eprintln!("skip: X402_E2E=1 not set (see test module docs for prerequisites)");
        return true;
    }
    false
}

fn assert_wallet_funded(rpc: &RpcClient, buyer_pk: &solana_sdk::pubkey::Pubkey) {
    let sol_lamports = rpc
        .get_balance_with_commitment(buyer_pk, CommitmentConfig::confirmed())
        .expect("query SOL balance")
        .value;
    if sol_lamports < MIN_DEVNET_SOL_LAMPORTS {
        // Best-effort airdrop. Faucet often rate-limits; we panic with
        // instructions if it doesn't grant enough.
        let _ = rpc.request_airdrop(buyer_pk, MIN_DEVNET_SOL_LAMPORTS * 2);
        std::thread::sleep(Duration::from_secs(8));
        let after = rpc
            .get_balance_with_commitment(buyer_pk, CommitmentConfig::confirmed())
            .expect("re-query SOL balance")
            .value;
        if after < MIN_DEVNET_SOL_LAMPORTS {
            panic!(
                "buyer wallet {buyer_pk} has only {after} lamports on devnet; \
                 fund it via https://faucet.solana.com or `solana airdrop` and retry"
            );
        }
    }

    // Circle devnet USDC.
    let usdc_mint =
        solana_sdk::pubkey::Pubkey::try_from("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU")
            .expect("devnet USDC mint pubkey");
    let buyer_ata =
        spl_associated_token_account::get_associated_token_address(buyer_pk, &usdc_mint);
    match rpc.get_token_account_balance(&buyer_ata) {
        Ok(bal) => {
            let amount: u64 = bal.amount.parse().expect("token amount must parse as u64");
            if amount < MIN_DEVNET_USDC_ATOMIC {
                panic!(
                    "buyer ATA {buyer_ata} has only {amount} USDC atoms on devnet \
                     (need {MIN_DEVNET_USDC_ATOMIC}); fund via https://faucet.circle.com"
                );
            }
        }
        Err(e) => {
            panic!(
                "buyer ATA {buyer_ata} for {buyer_pk} does not exist or is unreadable on \
                 devnet: {e}; fund via https://faucet.circle.com"
            );
        }
    }
}

async fn fetch_v2_challenge() -> serde_json::Value {
    let body = serde_json::json!({
        "request": { "unsigned_payload": "0xdeadbeef", "chain": "CHAIN_ETHEREUM" }
    });
    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .json(&body)
        .send()
        .await
        .expect("send probe");
    assert_eq!(resp.status(), 402, "expected 402 for probe");
    let header = resp
        .headers()
        .get("Payment-Required")
        .expect("Payment-Required header")
        .to_str()
        .unwrap()
        .to_string();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(header.as_bytes())
        .expect("base64 Payment-Required");
    let parsed: serde_json::Value =
        serde_json::from_slice(&decoded).expect("Payment-Required JSON");
    parsed
}

/// Path 7a: gateway boots with payai + solana-devnet and serves a 402 whose
/// challenge advertises `solana-devnet`. No payment is performed here; this
/// path is cheap and lets us validate the boot path without spending USDC.
#[tokio::test]
#[ignore]
async fn path7a_gateway_boots_with_payai_devnet() {
    if skip_unless_e2e() {
        return;
    }
    let _guard = TEST_MUTEX.lock().await;

    let buyer = load_devnet_keypair(WALLET_SEED_PATH).expect("load fixture keypair");
    let receiver = buyer.pubkey(); // self-receive is fine for this probe
    let tvc_hex = fixture_ephemeral_pubkey_hex();

    let _p = start_stack(&receiver.to_string(), &tvc_hex).await;

    let challenge = fetch_v2_challenge().await;
    let accepts = challenge["accepts"].as_array().expect("accepts array");
    let has_devnet = accepts
        .iter()
        .any(|t| t["network"].as_str() == Some("solana-devnet"));
    assert!(
        has_devnet,
        "expected solana-devnet in 402 accepts; got: {accepts:?}"
    );
}

/// Path 7b: full pay → parse → verify cycle against real payai facilitator
/// and real Solana devnet USDC. Spends `PARSER_PRICE_USD` from the fixture
/// wallet.
#[tokio::test]
#[ignore]
async fn path7b_full_devnet_pay_and_verify() {
    if skip_unless_e2e() {
        return;
    }
    let _guard = TEST_MUTEX.lock().await;

    let buyer = load_devnet_keypair(WALLET_SEED_PATH).expect("load fixture keypair");
    let buyer_pk = buyer.pubkey();
    eprintln!("[fixture] devnet buyer address: {buyer_pk}");

    let rpc = RpcClient::new_with_commitment(DEVNET_RPC.to_string(), CommitmentConfig::confirmed());
    assert_wallet_funded(&rpc, &buyer_pk);

    let receiver = buyer_pk; // self-transfer keeps test self-contained
    let tvc_hex = fixture_ephemeral_pubkey_hex();
    let _p = start_stack(&receiver.to_string(), &tvc_hex).await;

    let challenge = fetch_v2_challenge().await;
    let accepts = challenge["accepts"]
        .as_array()
        .expect("accepts must be array");
    let devnet = accepts
        .iter()
        .find(|t| t["network"].as_str() == Some("solana-devnet"))
        .expect("solana-devnet entry in accepts")
        .clone();

    let reqs = PaymentRequirementsLite::from_value(&devnet).expect("parse devnet challenge");
    let tx = build_payment_transaction(&rpc, &buyer, &reqs).expect("build payment tx");
    let header_value = build_x_payment_header(&reqs, &tx).expect("build X-PAYMENT header");

    let eth_tx_hex = "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";

    let body = serde_json::json!({
        "request": { "unsigned_payload": eth_tx_hex, "chain": "CHAIN_ETHEREUM" }
    });
    let resp = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{GW_PORT}/visualsign/api/v2/parse"
        ))
        .header("X-PAYMENT", header_value)
        .json(&body)
        .send()
        .await
        .expect("send paid request");
    let status = resp.status();
    let x_payment_response = resp
        .headers()
        .get("X-PAYMENT-RESPONSE")
        .map(|v| v.to_str().unwrap_or_default().to_string());
    let body_text = resp.text().await.unwrap_or_default();
    assert_eq!(
        status, 200,
        "expected 200 after paying; body: {body_text}; price ${PARSER_PRICE_USD}"
    );
    assert!(
        x_payment_response.is_some(),
        "expected X-PAYMENT-RESPONSE header on 200"
    );

    // Cross-check: the gateway's own pinned verifier already passed. We also
    // verify here in the test that the response's `signature.publicKey` equals
    // the pinned hex, so this test would catch any regression in the gateway's
    // attestation wiring.
    let v: serde_json::Value = serde_json::from_str(&body_text).expect("response JSON");
    let response_pubkey = v["response"]["parsedTransaction"]["signature"]["publicKey"]
        .as_str()
        .expect("response.signature.publicKey must be present");
    assert_eq!(
        response_pubkey.to_ascii_lowercase(),
        tvc_hex.to_ascii_lowercase(),
        "response pubkey must match pinned TVC verifier"
    );
}
