//! VerifiedPaymentMarker, the signed proof the gateway hands to parser_app
//! to certify that an x402 payment was verified + settled before the parse
//! request was forwarded.
//!
//! Wire shape: `borsh(SignedVerifiedPaymentMarker)` rides as
//! `ParseRequest.payment_marker` (bytes field). Same type defined here so
//! both gateway (signer) and parser_app (verifier) deserialize identical
//! bytes, no schema drift.
//!
//! Trust model: parser_app verifies the gateway's P256 signature against a
//! pubkey pinned at TVC deploy time (`GATEWAY_SIGNING_PUBKEY_HEX`), and
//! binds the marker to the full authenticated request body via
//! `request_hash` (all of `chain`, `unsigned_payload`, `chain_metadata`, and
//! `include_intermediate_output`). This stops a party holding one valid
//! marker (but not the gateway's signing key) from replaying it against a
//! different request with different metadata or intermediate-output
//! settings; it does not protect against a compromised signing key itself,
//! which could mint a fresh marker for any request. The settlement-side
//! fields live in [`PaymentDetails`] and are carried in the signed marker
//! for the record, but are not independently cross-checked by parser_app
//! today; that deeper verification (including recomputing
//! `x_payment_hash` from the forwarded `X-PAYMENT` bytes, which has no
//! transport in this proto yet) is deferred to v3.1.

use borsh::{BorshDeserialize, BorshSerialize};

/// The settlement facts for one payment, keyed by the scheme that produced
/// them.
///
/// Borsh encodes an enum as a 1-byte little-endian variant index followed by
/// that variant's fields, so the variant order here is part of the wire
/// contract rather than an implementation detail: the gateway-side signer
/// writes the discriminant by hand. Add schemes by appending variants, never
/// by inserting or reordering.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum PaymentDetails {
    /// A single x402 payment settled straight from one payer to one
    /// recipient. The only scheme the gateway mints today.
    X402Direct {
        /// On-chain settlement signature, base58 (Solana).
        txid: String,
        /// Payer pubkey, base58 (Solana).
        payer: String,
        /// Recipient pubkey, base58 (Solana).
        pay_to: String,
        /// Atomic units paid (USDC has 6 decimals; "1000" = $0.001).
        amount: String,
        /// Asset mint, base58 (Solana). E.g. devnet USDC
        /// `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`.
        mint: String,
        /// SHA-256 of the inner base64-decoded `X-PAYMENT` body. Intended to
        /// let parser_app confirm the gateway didn't pair the buyer's signed
        /// Solana tx with a different VPM, but this proto has no field
        /// carrying the forwarded `X-PAYMENT` bytes yet and parser_app does
        /// not check this value today; recomputing and checking it is
        /// deferred to v3.1.
        x_payment_hash: [u8; 32],
        /// CAIP-2 network identifier (e.g. `solana:EtWTRABZaYq6...` for
        /// devnet).
        network: String,
    },
}

/// The signed payload parser_app verifies.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPaymentMarker {
    /// Bumped if this envelope changes incompatibly. A new payment scheme is
    /// a new [`PaymentDetails`] variant instead of a bump: a verifier that
    /// predates the variant fails the Borsh decode, which fails closed.
    pub version: u32,
    /// SHA-256 binding this marker to one specific parse request: see
    /// [`request_hash`] for the exact preimage (chain, unsigned_payload,
    /// chain_metadata, include_intermediate_output).
    pub request_hash: [u8; 32],
    /// What was paid, and under which scheme. Carried in the signed marker
    /// for the record; parser_app does not cross-check these values today.
    pub details: PaymentDetails,
    /// Unix millis at which the gateway received the facilitator's settle
    /// response.
    ///
    /// Signed for the record; nothing reads it today. There is deliberately
    /// no max-age check in `parser_app`: `request_hash` already binds a
    /// marker to one exact request, so a replayed marker can only re-run the
    /// byte-identical parse, and parse is read-only. The exposure is
    /// therefore repeat parses of an already-paid request, not a bypass,
    /// which doesn't justify making enclave verification depend on clock
    /// skew between the signer and the enclave. Revisit if a future scheme
    /// makes markers fungible across requests.
    pub settled_at_ms: u64,
    /// SEC1-uncompressed hex of the gateway's P256 signing public key.
    /// MUST decode to the same bytes as the pinned
    /// `GATEWAY_SIGNING_PUBKEY_HEX` on parser_app. The verifier compares
    /// decoded bytes, so an optional `0x` prefix and either case are
    /// accepted on both sides.
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
/// `unsigned_payload`, `chain_metadata` (pass `borsh::to_vec` of the inner
/// `ChainMetadata` value when present, NOT wrapped in `Option` (no
/// discriminant byte), or an empty slice if absent; matching the convention
/// used for `ParsedTransactionPayload::metadata_digest`), and
/// `include_intermediate_output`. Both gateway and parser_app call this so
/// the binding is unambiguous; every variable-length field is
/// length-prefixed so no two distinct inputs can hash to the same preimage,
/// as long as `unsigned_payload` and `chain_metadata_bytes` each stay under
/// 2^32 bytes (their length prefixes are `as u32`). The gRPC server's own
/// message-size cap is far below that bound today, so this is a
/// documentation caveat, not a live gap.
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
            details: PaymentDetails::X402Direct {
                txid: "abc".into(),
                payer: "Pay".into(),
                pay_to: "Recv".into(),
                amount: "1000".into(),
                mint: "Mint".into(),
                x_payment_hash: [2u8; 32],
                network: "solana:test".into(),
            },
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
            details: PaymentDetails::X402Direct {
                txid: "tx".into(),
                payer: String::new(),
                pay_to: String::new(),
                amount: "0".into(),
                mint: String::new(),
                x_payment_hash: [0u8; 32],
                network: String::new(),
            },
            settled_at_ms: 0,
            gateway_pubkey_hex: String::new(),
        };
        assert_eq!(vpm.signing_digest().unwrap(), vpm.signing_digest().unwrap());
    }

    #[test]
    fn signing_digest_matches_hand_encoded_wire_bytes() {
        // `signing_digest_is_deterministic` above only compares this
        // implementation with itself, so a field reorder or type change in
        // `VerifiedPaymentMarker`/`PaymentDetails` would still pass it even
        // though it silently breaks interop with the external gateway
        // signer. This test instead hand-assembles the expected bytes from
        // Borsh's documented encoding rules (LE integers, 4-byte-len-prefixed
        // strings, 1-byte enum variant index, raw fixed-size arrays) rather
        // than deriving them from `borsh::to_vec` on this same struct, so a
        // reorder changes `borsh::to_vec(&vpm)` but not `expected`, failing
        // the test.
        let vpm = VerifiedPaymentMarker {
            version: 1,
            request_hash: [0u8; 32],
            details: PaymentDetails::X402Direct {
                txid: "tx".into(),
                payer: String::new(),
                pay_to: String::new(),
                amount: "0".into(),
                mint: String::new(),
                x_payment_hash: [0u8; 32],
                network: String::new(),
            },
            settled_at_ms: 0,
            gateway_pubkey_hex: String::new(),
        };

        let mut expected = Vec::new();
        expected.extend_from_slice(&1u32.to_le_bytes()); // version
        expected.extend_from_slice(&[0u8; 32]); // request_hash
        expected.push(0); // PaymentDetails variant index: X402Direct
        for s in ["tx", "", "", "0", ""] {
            // txid, payer, pay_to, amount, mint: length-prefixed strings
            expected.extend_from_slice(&(s.len() as u32).to_le_bytes());
            expected.extend_from_slice(s.as_bytes());
        }
        expected.extend_from_slice(&[0u8; 32]); // x_payment_hash
        expected.extend_from_slice(&0u32.to_le_bytes()); // network (empty)
        expected.extend_from_slice(&0u64.to_le_bytes()); // settled_at_ms
        expected.extend_from_slice(&0u32.to_le_bytes()); // gateway_pubkey_hex (empty)

        assert_eq!(borsh::to_vec(&vpm).unwrap(), expected);
        assert_eq!(
            vpm.signing_digest().unwrap(),
            qos_crypto::sha_256(&expected)
        );
    }

    #[test]
    fn x402_direct_is_borsh_variant_zero() {
        // The gateway-side signer writes this discriminant by hand, so the
        // variant index is wire contract. Borsh prefixes an enum with a
        // 1-byte LE variant index; appending variants must leave this at 0.
        let details = PaymentDetails::X402Direct {
            txid: String::new(),
            payer: String::new(),
            pay_to: String::new(),
            amount: String::new(),
            mint: String::new(),
            x_payment_hash: [0u8; 32],
            network: String::new(),
        };
        assert_eq!(borsh::to_vec(&details).unwrap().first(), Some(&0u8));
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
    fn request_hash_matches_hand_encoded_preimage() {
        // The two differential tests around this one compare `request_hash`
        // with itself, so they pin the *presence* of the length prefixes and
        // nothing beyond it: reorder the four writes, swap `to_le_bytes` for
        // `to_be_bytes`, or drop the `chain` write entirely and both stay
        // green while every marker minted by the external gateway signer
        // stops verifying. This literal doesn't move when the implementation
        // does, so it catches all three.
        //
        // The expectation is hand-assembled from the documented preimage
        // layout rather than captured from `request_hash` itself, the same
        // way `signing_digest_matches_hand_encoded_wire_bytes` avoids
        // deriving its expectation from the code under test:
        //
        //   0100_0000                    chain = 1, i32 LE
        //   0a00_0000                    unsigned_payload len = 10, u32 LE
        //   30786465616462656566         "0xdeadbeef"
        //   0300_0000                    chain_metadata len = 3, u32 LE
        //   010203                       chain_metadata bytes
        //   01                           include_intermediate_output = true
        //
        // sha256 of that preimage:
        let expected: [u8; 32] = [
            0xf8, 0xb6, 0x4e, 0x41, 0xac, 0x19, 0xee, 0x85, 0xf7, 0x4d, 0x23, 0x3f, 0x9f, 0xf0,
            0x88, 0xdf, 0x73, 0xc1, 0x47, 0x6d, 0xcd, 0xe3, 0xce, 0xfc, 0x13, 0xd2, 0xb8, 0x5d,
            0x43, 0x4a, 0x5a, 0xc9,
        ];
        assert_eq!(request_hash(1, "0xdeadbeef", &[1, 2, 3], true), expected);
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

    #[test]
    fn signed_marker_pins_field_order_and_signature_length_prefix() {
        // `SignedVerifiedPaymentMarker` (vpm, then signature) is the struct
        // that actually rides in `ParseRequest.payment_marker`, not `vpm`
        // alone. This pins its wire layout independently of
        // `VerifiedPaymentMarker`'s own pinned-bytes test above: a field
        // swap (signature before vpm), or a signer that writes
        // `vpm_bytes || sig[64]` with no length prefix on `signature`,
        // passes every existing `sign_with`/`verify` round-trip test but
        // produces bytes the enclave cannot deserialize as intended.
        let vpm = VerifiedPaymentMarker {
            version: 1,
            request_hash: [0u8; 32],
            details: PaymentDetails::X402Direct {
                txid: "tx".into(),
                payer: String::new(),
                pay_to: String::new(),
                amount: "0".into(),
                mint: String::new(),
                x_payment_hash: [0u8; 32],
                network: String::new(),
            },
            settled_at_ms: 0,
            gateway_pubkey_hex: String::new(),
        };

        // Hand-assembled from the documented Borsh layout (LE integers, a
        // 1-byte enum variant index, 4-byte-length-prefixed
        // strings/Vec<u8>, raw fixed-size arrays), independent of
        // `borsh::to_vec` on either `vpm` or `signed`.
        let mut expected_vpm_bytes = Vec::new();
        expected_vpm_bytes.extend_from_slice(&1u32.to_le_bytes()); // version
        expected_vpm_bytes.extend_from_slice(&[0u8; 32]); // request_hash
        expected_vpm_bytes.push(0); // PaymentDetails variant index: X402Direct
        for s in ["tx", "", "", "0", ""] {
            // txid, payer, pay_to, amount, mint: length-prefixed strings
            expected_vpm_bytes.extend_from_slice(&(s.len() as u32).to_le_bytes());
            expected_vpm_bytes.extend_from_slice(s.as_bytes());
        }
        expected_vpm_bytes.extend_from_slice(&[0u8; 32]); // x_payment_hash
        expected_vpm_bytes.extend_from_slice(&0u32.to_le_bytes()); // network (empty)
        expected_vpm_bytes.extend_from_slice(&0u64.to_le_bytes()); // settled_at_ms
        expected_vpm_bytes.extend_from_slice(&0u32.to_le_bytes()); // gateway_pubkey_hex (empty)

        let signature = vec![0xABu8; 64];
        let mut expected = expected_vpm_bytes;
        expected.extend_from_slice(&(signature.len() as u32).to_le_bytes());
        expected.extend_from_slice(&signature);

        let signed = SignedVerifiedPaymentMarker { vpm, signature };

        assert_eq!(borsh::to_vec(&signed).unwrap(), expected);
    }
}
