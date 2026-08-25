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
//! pubkey pinned at TVC deploy time (`GATEWAY_SIGNING_PUBKEY_HEX`), and
//! binds the marker to the full authenticated request body via
//! `request_hash` (all of `chain`, `unsigned_payload`, `chain_metadata`, and
//! `include_intermediate_output`), so a compromised gateway cannot replay a
//! marker against a request with different metadata or intermediate-output
//! settings. The settlement-side fields (`txid`, `payer`, `pay_to`,
//! `amount`, `mint`, `network`, `x_payment_hash`) are carried in the signed
//! marker for the record but are not independently cross-checked by
//! parser_app today; that deeper verification (including recomputing
//! `x_payment_hash` from the forwarded `X-PAYMENT` bytes, which has no
//! transport in this proto yet) is deferred to v3.1.

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
    /// SHA-256 binding this marker to one specific parse request: see
    /// [`request_hash`] for the exact preimage (chain, unsigned_payload,
    /// chain_metadata, include_intermediate_output).
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
    /// SHA-256 of the inner base64-decoded `X-PAYMENT` body. Intended to let
    /// parser_app confirm the gateway didn't pair the buyer's signed Solana
    /// tx with a different VPM, but this proto has no field carrying the
    /// forwarded `X-PAYMENT` bytes yet and parser_app does not check this
    /// value today; recomputing and checking it is deferred to v3.1.
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
    /// P256 ECDSA (SHA-256) signature over the *message*
    /// `qos_crypto::sha_256(borsh(vpm))` (i.e. `qos_p256::P256SignPair::sign`
    /// hashes this 32-byte value again internally, so the effective digest
    /// signed is `sha256(sha256(borsh(vpm)))`). Raw `r||s`, 64 bytes, no DER.
    pub signature: Vec<u8>,
}

impl VerifiedPaymentMarker {
    /// Borsh-encode + SHA-256 the encoded bytes. This is what the gateway
    /// signs and parser_app verifies against.
    ///
    /// # Errors
    ///
    /// Returns an error if Borsh serialization fails. This should not
    /// happen in practice (all fields are owned scalar/String/array types),
    /// but callers must propagate the error rather than substitute a fixed
    /// fallback digest: signer and verifier run identical code, so a fixed
    /// fallback would let a signature over that fallback verify against any
    /// VPM that hits the same serialization failure.
    pub fn signing_digest(&self) -> Result<[u8; 32], borsh::io::Error> {
        let bytes = borsh::to_vec(self)?;
        Ok(qos_crypto::sha_256(&bytes))
    }
}

/// Compute `request_hash` over the full authenticated request: `chain`,
/// `unsigned_payload`, `chain_metadata` (pass the caller's
/// `borsh::to_vec` of `Option<ChainMetadata>`, or an empty slice if absent,
/// matching the convention used for `ParsedTransactionPayload::metadata_digest`),
/// and `include_intermediate_output`. Both gateway and parser_app call this
/// so the binding is unambiguous; every variable-length field is
/// length-prefixed so no two distinct inputs can hash to the same preimage.
#[must_use]
pub fn request_hash(
    chain: i32,
    unsigned_payload: &str,
    chain_metadata_bytes: &[u8],
    include_intermediate_output: bool,
) -> [u8; 32] {
    let mut buf =
        Vec::with_capacity(4 + 4 + unsigned_payload.len() + 4 + chain_metadata_bytes.len() + 1);
    buf.extend_from_slice(&chain.to_le_bytes());
    buf.extend_from_slice(&(unsigned_payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(unsigned_payload.as_bytes());
    buf.extend_from_slice(&(chain_metadata_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(chain_metadata_bytes);
    buf.push(u8::from(include_intermediate_output));
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
        assert_eq!(vpm.signing_digest().unwrap(), vpm.signing_digest().unwrap());
    }

    #[test]
    fn request_hash_is_stable_and_sensitive_to_every_bound_field() {
        let h1 = request_hash(1, "0xdeadbeef", &[], false);
        let h2 = request_hash(1, "0xdeadbeef", &[], false);
        let h3 = request_hash(2, "0xdeadbeef", &[], false);
        let h4 = request_hash(1, "0xdeadbeee", &[], false);
        let h5 = request_hash(1, "0xdeadbeef", &[1, 2, 3], false);
        let h6 = request_hash(1, "0xdeadbeef", &[], true);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3, "must be sensitive to chain");
        assert_ne!(h1, h4, "must be sensitive to unsigned_payload");
        assert_ne!(h1, h5, "must be sensitive to chain_metadata bytes");
        assert_ne!(h1, h6, "must be sensitive to include_intermediate_output");
    }

    #[test]
    fn request_hash_length_prefixes_defeat_field_boundary_collisions() {
        // Without the length prefixes, the preimage is a bare concatenation
        // of unsigned_payload and chain_metadata, so shifting a byte across
        // the boundary yields identical bytes: "AB" + [] == "A" + [b'B'].
        // A marker paid for one request would then verify against the other.
        // These two calls MUST differ, or the binding is forgeable.
        let shifted_right = request_hash(1, "AB", &[], false);
        let shifted_left = request_hash(1, "A", b"B", false);
        assert_ne!(
            shifted_right, shifted_left,
            "unsigned_payload/chain_metadata boundary must be unambiguous"
        );
    }
}
