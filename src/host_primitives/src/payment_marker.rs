//! VerifiedPaymentMarker — the signed proof the gateway hands to parser_app
//! to certify that an x402 payment was verified + settled before the parse
//! request was forwarded.
//!
//! Wire shape: `borsh(SignedVerifiedPaymentMarker)` rides as
//! `ParseRequest.payment_marker` (bytes field). Same type defined here so
//! both gateway (signer) and parser_app (verifier) deserialize identical
//! bytes — no schema drift.
//!
//! Trust model: parser_app verifies the gateway's P256 signature against a
//! pubkey pinned at TVC deploy time (`GATEWAY_SIGNING_PUBKEY_HEX`). The
//! marker itself binds to a specific parse request (`request_hash`) and a
//! specific on-chain settlement (`txid` + `x_payment_hash`), so a compromised
//! gateway cannot re-use a marker across requests or pair an old payment
//! with a new parse body.

use borsh::{BorshDeserialize, BorshSerialize};

/// The signed payload parser_app verifies.
#[derive(
    BorshSerialize,
    BorshDeserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct VerifiedPaymentMarker {
    /// Bumped if the schema changes incompatibly.
    pub version: u32,
    /// SHA-256 of the (chain, unsigned_payload) tuple — binds this marker
    /// to one specific parse request.
    pub request_hash: [u8; 32],
    /// On-chain settlement signature, base58 (Solana).
    pub txid: String,
    /// Payer pubkey, base58 (Solana).
    pub payer: String,
    /// Recipient pubkey, base58 (Solana).
    pub pay_to: String,
    /// Atomic units paid (USDC has 6 decimals; "1000" = $0.001).
    pub amount: String,
    /// Asset mint, base58 (Solana). E.g. devnet USDC
    /// `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`.
    pub mint: String,
    /// SHA-256 of the inner base64-decoded `X-PAYMENT` body. parser_app
    /// recomputes this from the forwarded X-PAYMENT bytes to confirm the
    /// gateway didn't pair the buyer's signed Solana tx with a different
    /// VPM.
    pub x_payment_hash: [u8; 32],
    /// CAIP-2 network identifier (e.g. `solana:EtWTRABZaYq6...` for devnet).
    pub network: String,
    /// Unix millis at which the gateway received the facilitator's settle
    /// response.
    pub settled_at_ms: u64,
    /// SEC1-uncompressed hex of the gateway's P256 signing public key.
    /// MUST equal the pinned `GATEWAY_SIGNING_PUBKEY_HEX` on parser_app.
    pub gateway_pubkey_hex: String,
}

pub const VPM_VERSION: u32 = 1;

/// `borsh(SignedVerifiedPaymentMarker)` is what rides in
/// `ParseRequest.payment_marker`.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct SignedVerifiedPaymentMarker {
    pub vpm: VerifiedPaymentMarker,
    /// P256 ECDSA signature over `qos_crypto::sha_256(borsh(vpm))`. Raw
    /// `r||s` (64 bytes) per qos_p256's `P256Pair::sign` output convention.
    pub signature: Vec<u8>,
}

impl VerifiedPaymentMarker {
    /// Borsh-encode + SHA-256 the encoded bytes. This is what the gateway
    /// signs and parser_app verifies against.
    ///
    /// Borsh serialization of this struct can only fail on an OOM / I/O
    /// error since the type uses owned types end-to-end and derives
    /// `BorshSerialize` directly. Falls back to an empty buffer on the
    /// impossible-in-practice path so the function stays infallible (the
    /// resulting digest is the SHA-256 of empty bytes — distinct enough
    /// that any caller comparing will reject it).
    #[must_use]
    pub fn signing_digest(&self) -> [u8; 32] {
        let bytes = borsh::to_vec(self).unwrap_or_default();
        qos_crypto::sha_256(&bytes)
    }
}

/// Compute `request_hash` over the (chain, unsigned_payload) tuple. Both
/// gateway and parser_app call this so the binding is unambiguous.
///
/// `chain` is the proto `Chain` discriminant value (`i32`), serialized as
/// 4 little-endian bytes followed by the UTF-8 bytes of `unsigned_payload`.
#[must_use]
pub fn request_hash(chain: i32, unsigned_payload: &str) -> [u8; 32] {
    let mut buf = Vec::with_capacity(4 + unsigned_payload.len());
    buf.extend_from_slice(&chain.to_le_bytes());
    buf.extend_from_slice(unsigned_payload.as_bytes());
    qos_crypto::sha_256(&buf)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn vpm_round_trip_borsh() {
        let vpm = VerifiedPaymentMarker {
            version: VPM_VERSION,
            request_hash: [1u8; 32],
            txid: "abc".into(),
            payer: "Pay".into(),
            pay_to: "Recv".into(),
            amount: "1000".into(),
            mint: "Mint".into(),
            x_payment_hash: [2u8; 32],
            network: "solana:test".into(),
            settled_at_ms: 1_700_000_000_000,
            gateway_pubkey_hex: "04abcd".into(),
        };
        let bytes = borsh::to_vec(&vpm).unwrap();
        let decoded = VerifiedPaymentMarker::try_from_slice(&bytes).unwrap();
        assert_eq!(vpm, decoded);
    }

    #[test]
    fn signing_digest_is_deterministic() {
        let vpm = VerifiedPaymentMarker {
            version: VPM_VERSION,
            request_hash: [0u8; 32],
            txid: "tx".into(),
            payer: String::new(),
            pay_to: String::new(),
            amount: "0".into(),
            mint: String::new(),
            x_payment_hash: [0u8; 32],
            network: String::new(),
            settled_at_ms: 0,
            gateway_pubkey_hex: String::new(),
        };
        assert_eq!(vpm.signing_digest(), vpm.signing_digest());
    }

    #[test]
    fn request_hash_is_stable_and_chain_sensitive() {
        let h1 = request_hash(1, "0xdeadbeef");
        let h2 = request_hash(1, "0xdeadbeef");
        let h3 = request_hash(2, "0xdeadbeef");
        let h4 = request_hash(1, "0xdeadbeee");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_ne!(h1, h4);
    }
}
