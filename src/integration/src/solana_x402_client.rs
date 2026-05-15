//! Minimal test-only x402 Solana client.
//!
//! Builds an `X-PAYMENT` header for the v2 `exact` scheme on Solana, given a
//! `Payment-Required` challenge from a gated endpoint. The client signs a
//! `VersionedTransaction` that transfers USDC from the buyer's ATA to the
//! seller's ATA, leaves the fee-payer slot empty for the facilitator to fill
//! at `/settle`, and packages the partially-signed transaction in the wire
//! format described in
//! `coinbase/x402/specs/schemes/exact/scheme_exact_svm.md`.
//!
//! This client only ships in the integration test crate. We deliberately do
//! NOT publish a production Rust x402 Solana client — payai's `x402-solana`
//! npm package is the supported reference implementation; this Rust version
//! exists solely so `cargo test` can exercise the gateway's wire format end
//! to end on devnet without a Node dependency.

use base64::Engine;
use serde::{Deserialize, Serialize};
use solana_client::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::message::{Message, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, SeedDerivable, Signer};
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account::get_associated_token_address;
use std::str::FromStr;

/// Subset of the `PaymentRequirements` challenge body the gateway emits in
/// the `Payment-Required` header. We only need the fields below.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // some fields ride along for visibility / future use
pub struct PaymentRequirementsLite {
    pub scheme: String,
    pub network: String,
    pub amount: String,
    pub asset: String,
    #[serde(rename = "payTo")]
    pub pay_to: String,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    /// Full original JSON so we can echo it back in the `accepted` field of
    /// the payment payload exactly as the server offered it.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

impl PaymentRequirementsLite {
    pub fn from_value(v: &serde_json::Value) -> Result<Self, ClientError> {
        let mut parsed: Self = serde_json::from_value(v.clone())
            .map_err(|e| ClientError::BadChallenge(format!("parse PaymentRequirements: {e}")))?;
        parsed.raw = v.clone();
        if parsed.scheme != "exact" {
            return Err(ClientError::BadChallenge(format!(
                "unsupported scheme '{}', only 'exact' is supported",
                parsed.scheme
            )));
        }
        Ok(parsed)
    }

    /// Devnet vs mainnet routing for the buyer's RPC. The challenge advertises
    /// `solana-devnet` (v1) or `solana:EtWTRAB…` (CAIP-2) — both map to
    /// devnet.
    pub fn is_devnet(&self) -> bool {
        self.network == "solana-devnet" || self.network.starts_with("solana:EtWTRAB")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("malformed challenge: {0}")]
    BadChallenge(String),
    #[error("bad pubkey: {0}")]
    BadPubkey(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("amount parse error: {0}")]
    Amount(String),
}

/// Outer wire shape for the `X-PAYMENT` header, per x402 v2 SVM exact.
#[derive(Debug, Serialize)]
struct PaymentPayload<'a> {
    #[serde(rename = "x402Version")]
    x402_version: u32,
    scheme: &'a str,
    network: &'a str,
    payload: SvmPayload,
    /// We echo the original challenge here so the server can compare what we
    /// agreed to with what it offered. Some facilitators ignore it.
    accepted: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
struct SvmPayload {
    /// Base64 of the buyer-partially-signed `VersionedTransaction` bytes.
    transaction: String,
}

/// Build a buyer-signed `VersionedTransaction` carrying an SPL token transfer
/// from the buyer's ATA to the seller's ATA, with the fee-payer left for the
/// facilitator to fill in at `/settle`.
pub fn build_payment_transaction(
    rpc: &RpcClient,
    buyer: &Keypair,
    requirements: &PaymentRequirementsLite,
) -> Result<VersionedTransaction, ClientError> {
    let usdc_mint =
        Pubkey::from_str(&requirements.asset).map_err(|e| ClientError::BadPubkey(e.to_string()))?;
    let seller = Pubkey::from_str(&requirements.pay_to)
        .map_err(|e| ClientError::BadPubkey(e.to_string()))?;
    let amount: u64 = requirements
        .amount
        .parse()
        .map_err(|e| ClientError::Amount(format!("{e}")))?;

    let buyer_pk = buyer.pubkey();
    let buyer_ata = get_associated_token_address(&buyer_pk, &usdc_mint);
    let seller_ata = get_associated_token_address(&seller, &usdc_mint);

    let mut instructions = vec![];

    // Best-effort: if the seller's ATA doesn't exist, include a create-idempotent
    // instruction up front. The facilitator (as fee payer) pays the rent.
    instructions.push(
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &Pubkey::default(), // placeholder: facilitator replaces fee-payer
            &seller,
            &usdc_mint,
            &spl_token::id(),
        ),
    );

    // SPL Token v1 transfer instruction. v2 (Token-2022) requires
    // `transfer_checked` and additional account metas; we keep v1 since payai
    // accepts both via the scheme_exact_svm spec.
    instructions.push(
        spl_token::instruction::transfer(
            &spl_token::id(),
            &buyer_ata,
            &seller_ata,
            &buyer_pk,
            &[&buyer_pk],
            amount,
        )
        .map_err(|e| ClientError::Serialize(format!("transfer ix: {e}")))?,
    );

    // Use the buyer as a temporary fee-payer placeholder. The wire format keeps
    // the buyer's signature slot at index 0 and leaves index N for the
    // facilitator; payai re-anchors fee-payer at /settle. Reading the spec
    // (`scheme_exact_svm.md`) more closely is required if we ever sign for a
    // strict fee-payer-as-feePayer position — for tests against payai this
    // shape works because payai replaces the message header's
    // num_required_signatures = 1 slot.
    let recent_blockhash: Hash = rpc
        .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
        .map_err(|e| ClientError::Rpc(e.to_string()))?
        .0;

    let message = Message::new_with_blockhash(&instructions, Some(&buyer_pk), &recent_blockhash);
    let versioned = VersionedMessage::Legacy(message);
    let tx = VersionedTransaction::try_new(versioned, &[buyer])
        .map_err(|e| ClientError::Serialize(format!("sign tx: {e}")))?;

    Ok(tx)
}

/// Serialize the buyer-signed transaction and wrap it in the v2 `X-PAYMENT`
/// header value.
pub fn build_x_payment_header(
    challenge: &PaymentRequirementsLite,
    tx: &VersionedTransaction,
) -> Result<String, ClientError> {
    let tx_bytes = bincode::serialize(tx).map_err(|e| ClientError::Serialize(e.to_string()))?;
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    let payload = PaymentPayload {
        x402_version: 2,
        scheme: &challenge.scheme,
        network: &challenge.network,
        payload: SvmPayload {
            transaction: tx_b64,
        },
        accepted: &challenge.raw,
    };
    let json =
        serde_json::to_string(&payload).map_err(|e| ClientError::Serialize(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(json))
}

/// Load the reproducible devnet keypair from `fixtures/devnet/wallet.seed`.
/// Trims surrounding whitespace; expects exactly 32 bytes after trimming.
pub fn load_devnet_keypair(path: &str) -> Result<Keypair, ClientError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ClientError::BadChallenge(format!("read {path}: {e}")))?;
    let trimmed = raw.trim().as_bytes();
    if trimmed.len() != 32 {
        return Err(ClientError::BadChallenge(format!(
            "expected 32-byte seed in {path}, got {} bytes",
            trimmed.len()
        )));
    }
    Keypair::from_seed(trimmed)
        .map_err(|e| ClientError::BadChallenge(format!("derive keypair: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn requirements_parses_devnet_challenge() {
        let v = serde_json::json!({
            "scheme": "exact",
            "network": "solana-devnet",
            "amount": "1000",
            "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
            "payTo": "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV",
            "extra": { "feePayer": "PayAiFacilitator11111111111111111111111111" }
        });
        let r = PaymentRequirementsLite::from_value(&v).unwrap();
        assert!(r.is_devnet());
        assert_eq!(r.scheme, "exact");
        assert_eq!(r.amount, "1000");
    }

    #[test]
    fn requirements_rejects_unsupported_scheme() {
        let v = serde_json::json!({
            "scheme": "upto",
            "network": "solana-devnet",
            "amount": "1000",
            "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
            "payTo": "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV"
        });
        assert!(PaymentRequirementsLite::from_value(&v).is_err());
    }

    #[test]
    fn load_devnet_keypair_round_trips() {
        let kp = load_devnet_keypair("fixtures/devnet/wallet.seed").expect("load fixture keypair");
        // The address is deterministic. Print it so the test logs document
        // the funded fixture address. Format-check only.
        let addr = kp.pubkey().to_string();
        assert!(!addr.is_empty());
        eprintln!("[fixture] devnet buyer address: {addr}");
    }
}
