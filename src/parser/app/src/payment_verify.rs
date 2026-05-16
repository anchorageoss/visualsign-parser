//! VerifiedPaymentMarker verification inside parser_app.
//!
//! Only checks the gateway's signature + binds the marker to this specific
//! request. The deeper buyer-Ed25519-on-the-Solana-tx check is deferred to
//! v3.1 (see plan).
//!
//! Policy is set at startup via `GATEWAY_SIGNING_PUBKEY_HEX`. When unset
//! (local dev / gRPC-direct calls), `PaymentPolicy::Disabled` is used and
//! VPM is not required. When set, the policy refuses any request whose
//! `payment_marker` doesn't carry a valid gateway-signed VPM bound to the
//! exact request body.

use borsh::BorshDeserialize;
use generated::google::rpc::Code;
use generated::parser::ParseRequest;
use host_primitives::payment_marker::{SignedVerifiedPaymentMarker, VPM_VERSION, request_hash};
use qos_p256::sign::P256SignPublic;
use subtle::ConstantTimeEq;

use crate::errors::GrpcError;

/// Whether `parser_app` requires (and verifies) a `VerifiedPaymentMarker`
/// on every parse call. Loaded once at startup from
/// `GATEWAY_SIGNING_PUBKEY_HEX`.
pub enum PaymentPolicy {
    /// No payment enforcement. Used by the open `/v1/parse` route and by
    /// local-dev / direct-gRPC callers.
    Disabled,
    /// Require a valid gateway-signed VPM in `ParseRequest.payment_marker`.
    Required {
        /// The gateway's P256 signing public key, pinned at TVC deploy
        /// time via `GATEWAY_SIGNING_PUBKEY_HEX`.
        pinned: P256SignPublic,
        /// Lower-cased hex of `pinned.to_bytes()`, memoized for log
        /// messages and for the cross-check against
        /// `vpm.gateway_pubkey_hex`.
        pinned_hex_lower: String,
    },
}

impl std::fmt::Debug for PaymentPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "PaymentPolicy::Disabled"),
            Self::Required {
                pinned_hex_lower, ..
            } => f
                .debug_struct("PaymentPolicy::Required")
                .field("pinned_hex", pinned_hex_lower)
                .finish(),
        }
    }
}

impl PaymentPolicy {
    /// Load from `GATEWAY_SIGNING_PUBKEY_HEX` env. Returns `Disabled` if
    /// unset. Returns a configuration error if set but unparseable.
    pub fn from_env() -> Result<Self, GrpcError> {
        match std::env::var("GATEWAY_SIGNING_PUBKEY_HEX") {
            Err(_) => Ok(Self::Disabled),
            Ok(hex_value) => Self::from_hex(&hex_value),
        }
    }

    /// Build a `Required` policy from a hex-encoded P256 sign pubkey
    /// (`P256SignPublic::to_bytes` SEC1 uncompressed). Surfacing this
    /// separately from `from_env` keeps env-coupling out of tests.
    pub fn from_hex(hex_value: &str) -> Result<Self, GrpcError> {
        let trimmed = hex_value.trim();
        let bytes = qos_hex::decode(trimmed).map_err(|e| {
            GrpcError::new(
                Code::Internal,
                &format!("GATEWAY_SIGNING_PUBKEY_HEX hex decode: {e:?}"),
            )
        })?;
        let pinned = P256SignPublic::from_bytes(&bytes).map_err(|e| {
            GrpcError::new(
                Code::Internal,
                &format!("GATEWAY_SIGNING_PUBKEY_HEX is not a valid P256 sign pubkey: {e:?}"),
            )
        })?;
        Ok(Self::Required {
            pinned,
            pinned_hex_lower: trimmed.to_ascii_lowercase(),
        })
    }
}

/// Reasons a request can be rejected for missing or invalid payment proof.
#[derive(Debug, thiserror::Error)]
pub enum PaymentVerifyError {
    /// `payment_marker` was empty in `Required` mode.
    #[error("payment marker is required for this endpoint")]
    Missing,
    /// The marker bytes weren't valid Borsh / didn't match the schema.
    #[error("payment marker decode error: {0}")]
    Decode(String),
    /// The marker was signed against an unknown VPM schema version.
    #[error("payment marker version {0} is not supported")]
    UnsupportedVersion(u32),
    /// The marker's `request_hash` doesn't match this request's
    /// (`chain`, `unsigned_payload`) tuple.
    #[error("payment marker does not match this request (request_hash mismatch)")]
    RequestHashMismatch,
    /// The marker claimed a different gateway pubkey than the pinned one.
    #[error("payment marker gateway_pubkey_hex does not match pinned key")]
    PinnedKeyMismatch,
    /// The gateway signature on the marker didn't verify.
    #[error("payment marker signature verification failed")]
    BadSignature,
}

impl From<PaymentVerifyError> for GrpcError {
    fn from(e: PaymentVerifyError) -> Self {
        // `FailedPrecondition` is what the gateway translates to HTTP 402.
        // We keep parser_app HTTP-unaware; the gateway maps gRPC status
        // codes to HTTP and synthesizes the canonical x402 PaymentRequired
        // body from its own config.
        GrpcError::new(Code::FailedPrecondition, &format!("{e}"))
    }
}

/// Returns `Ok(())` if the policy allows the request to proceed.
pub fn verify(parse_request: &ParseRequest, policy: &PaymentPolicy) -> Result<(), GrpcError> {
    let Some((pinned, pinned_hex_lower)) = (match policy {
        PaymentPolicy::Disabled => None,
        PaymentPolicy::Required {
            pinned,
            pinned_hex_lower,
        } => Some((pinned, pinned_hex_lower.as_str())),
    }) else {
        return Ok(());
    };

    if parse_request.payment_marker.is_empty() {
        return Err(PaymentVerifyError::Missing.into());
    }

    let signed = SignedVerifiedPaymentMarker::try_from_slice(&parse_request.payment_marker)
        .map_err(|e| PaymentVerifyError::Decode(format!("{e}")))?;

    let vpm = &signed.vpm;

    if vpm.version != VPM_VERSION {
        return Err(PaymentVerifyError::UnsupportedVersion(vpm.version).into());
    }

    // Bind the VPM to this exact request.
    let expected = request_hash(parse_request.chain, &parse_request.unsigned_payload);
    if expected != vpm.request_hash {
        return Err(PaymentVerifyError::RequestHashMismatch.into());
    }

    // Cross-check the gateway pubkey claimed in the VPM against the pinned
    // key — constant-time compare on bytes (after length check; ct_eq on
    // unequal-length slices returns 0).
    let claimed = vpm.gateway_pubkey_hex.to_ascii_lowercase();
    let a = claimed.as_bytes();
    let b = pinned_hex_lower.as_bytes();
    if a.len() != b.len() || a.ct_eq(b).unwrap_u8() != 1 {
        return Err(PaymentVerifyError::PinnedKeyMismatch.into());
    }

    let digest = vpm.signing_digest();
    pinned
        .verify(&digest, &signed.signature)
        .map_err(|_| GrpcError::from(PaymentVerifyError::BadSignature))?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use host_primitives::payment_marker::VerifiedPaymentMarker;
    use qos_p256::sign::P256SignPair;

    fn sign_with(pair: &P256SignPair, vpm: VerifiedPaymentMarker) -> Vec<u8> {
        let signed = SignedVerifiedPaymentMarker {
            signature: pair.sign(&vpm.signing_digest()).unwrap(),
            vpm,
        };
        borsh::to_vec(&signed).unwrap()
    }

    fn make_vpm(req: &ParseRequest, gateway_hex: &str) -> VerifiedPaymentMarker {
        VerifiedPaymentMarker {
            version: VPM_VERSION,
            request_hash: request_hash(req.chain, &req.unsigned_payload),
            txid: "txsig".into(),
            payer: "Pay".into(),
            pay_to: "Recv".into(),
            amount: "1000".into(),
            mint: "Mint".into(),
            x_payment_hash: [0u8; 32],
            network: "solana:test".into(),
            settled_at_ms: 0,
            gateway_pubkey_hex: gateway_hex.to_string(),
        }
    }

    fn req_with_marker(marker: Vec<u8>) -> ParseRequest {
        ParseRequest {
            unsigned_payload: "0xdeadbeef".into(),
            chain: 1,
            chain_metadata: None,
            payment_marker: marker,
        }
    }

    #[test]
    fn disabled_policy_accepts_anything() {
        let req = req_with_marker(vec![]);
        verify(&req, &PaymentPolicy::Disabled).unwrap();
    }

    #[test]
    fn required_policy_accepts_valid_marker() {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_hex);
        req.payment_marker = sign_with(&pair, vpm);

        verify(&req, &policy).unwrap();
    }

    #[test]
    fn required_policy_rejects_missing_marker() {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();
        let req = req_with_marker(vec![]);
        let err = verify(&req, &policy).unwrap_err();
        assert_eq!(err.code, Code::FailedPrecondition);
    }

    #[test]
    fn required_policy_rejects_request_hash_mismatch() {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();

        let req = req_with_marker(vec![]);
        let mut vpm = make_vpm(&req, &pub_hex);
        vpm.request_hash = [99u8; 32]; // does not match the actual request
        let marker = sign_with(&pair, vpm);
        let req = req_with_marker(marker);

        let err = verify(&req, &policy).unwrap_err();
        assert!(err.message.contains("request_hash"));
    }

    #[test]
    fn required_policy_rejects_wrong_gateway_key() {
        let pair_a = P256SignPair::generate();
        let pair_b = P256SignPair::generate();
        let pub_a = qos_hex::encode(&pair_a.public_key().to_bytes());
        let pub_b = qos_hex::encode(&pair_b.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_a).unwrap();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_b); // claims a different key
        req.payment_marker = sign_with(&pair_b, vpm);

        let err = verify(&req, &policy).unwrap_err();
        assert!(err.message.contains("pinned"));
    }

    #[test]
    fn required_policy_rejects_tampered_signature() {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_hex);
        let mut marker = sign_with(&pair, vpm);
        // Flip the last byte (inside the signature region — the signature
        // is the tail of the borsh-encoded struct).
        let last = marker.len() - 1;
        marker[last] ^= 0xff;
        req.payment_marker = marker;

        let err = verify(&req, &policy).unwrap_err();
        assert!(err.message.contains("signature"));
    }
}
