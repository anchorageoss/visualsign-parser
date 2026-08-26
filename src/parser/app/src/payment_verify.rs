//! VerifiedPaymentMarker verification inside parser_app.
//!
//! Only checks the gateway's signature + binds the marker to this specific
//! request. The deeper buyer-Ed25519-on-the-Solana-tx check is deferred to
//! v3.1 (see plan).
//!
//! Policy is built once at startup from a hex-encoded pinned gateway pubkey
//! via `PaymentPolicy::from_hex`, and the binary decides where that hex
//! value comes from (CLI arg / env). Local dev / gRPC-direct callers pass
//! `PaymentPolicy::Disabled` and VPM is not required. When `Required`, the
//! policy refuses any request whose `payment_marker` doesn't carry a valid
//! gateway-signed VPM bound to the exact request body.

use borsh::BorshDeserialize;
use generated::google::rpc::Code;
use generated::parser::{ChainMetadata, ParseRequest};
use host_primitives::payment_marker::{SignedVerifiedPaymentMarker, VPM_VERSION, request_hash};
use qos_p256::sign::P256SignPublic;
use visualsign::encodings::decode_hex;

use crate::errors::GrpcError;

/// Borsh-encodes `chain_metadata`, or returns an empty `Vec` if absent.
/// Matches the convention used for `ParsedTransactionPayload::metadata_digest`.
fn chain_metadata_bytes(metadata: Option<&ChainMetadata>) -> Result<Vec<u8>, borsh::io::Error> {
    metadata
        .map(borsh::to_vec)
        .transpose()
        .map(Option::unwrap_or_default)
}

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
        /// `qos_hex::encode(&pinned.to_bytes())`, memoized for log messages
        /// and for the cross-check against `vpm.gateway_pubkey_hex`.
        /// Derived from `pinned` (not from the raw config string) so it's
        /// always the canonical unprefixed lower-case encoding, matching
        /// what the gateway emits via the same `qos_hex::encode` call,
        /// regardless of whether the operator supplied a `0x`-prefixed
        /// value in `GATEWAY_SIGNING_PUBKEY_HEX`.
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
    /// Build a `Required` policy from a hex-encoded P256 sign pubkey
    /// (`P256SignPublic::to_bytes` SEC1 uncompressed). Binaries read their
    /// own config (CLI args / env) and call this directly; there is no
    /// env-coupled constructor here.
    pub fn from_hex(hex_value: &str) -> Result<Self, GrpcError> {
        let trimmed = hex_value.trim();
        let bytes = decode_hex(trimmed).map_err(|e| {
            GrpcError::internal(&format!("GATEWAY_SIGNING_PUBKEY_HEX hex decode: {e:?}"))
        })?;
        let pinned = P256SignPublic::from_bytes(&bytes).map_err(|e| {
            GrpcError::internal(&format!(
                "GATEWAY_SIGNING_PUBKEY_HEX is not a valid P256 sign pubkey: {e:?}"
            ))
        })?;
        let pinned_hex_lower = qos_hex::encode(&pinned.to_bytes());
        Ok(Self::Required {
            pinned,
            pinned_hex_lower,
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
    /// The marker's `request_hash` doesn't match this request (see
    /// `host_primitives::payment_marker::request_hash` for the exact
    /// fields covered).
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
        // `FailedPrecondition` is meant to be translated to HTTP 402 by the
        // gateway. We keep parser_app HTTP-unaware; the gateway is
        // responsible for mapping gRPC status codes to HTTP and
        // synthesizing the canonical x402 PaymentRequired body from its
        // own config (not yet wired as of this policy being `Disabled`
        // everywhere).
        GrpcError::new(Code::FailedPrecondition, &format!("{e}"))
    }
}

/// Returns `Ok(())` if the policy allows the request to proceed.
pub fn verify(parse_request: &ParseRequest, policy: &PaymentPolicy) -> Result<(), GrpcError> {
    let (pinned, pinned_hex_lower) = match policy {
        PaymentPolicy::Disabled => return Ok(()),
        PaymentPolicy::Required {
            pinned,
            pinned_hex_lower,
        } => (pinned, pinned_hex_lower.as_str()),
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

    // Bind the VPM to this exact request: chain, unsigned_payload,
    // chain_metadata, and include_intermediate_output all feed the
    // enclave's own attested output, so all four must be covered.
    //
    // Destructured exhaustively on purpose. `ParseRequest` is a plain
    // generated struct with no `#[non_exhaustive]`, so a new proto field
    // breaks this line and forces a decision: hash it below, or bind it to
    // `_` here with a note saying why it is out of scope. Field 4
    // (`include_intermediate_output`) was already missed once while this
    // work sat on an unmerged branch. Note this only guards the verifier:
    // whoever fixes the compile error also has to extend the gateway-side
    // signer and its pinned-preimage test, or the two sides silently
    // disagree.
    let ParseRequest {
        unsigned_payload,
        chain,
        chain_metadata,
        include_intermediate_output,
        // Deliberately not hashed: it carries the marker itself, and a hash
        // the marker commits to cannot also cover the marker.
        payment_marker: _,
    } = parse_request;

    let chain_metadata_bytes = chain_metadata_bytes(chain_metadata.as_ref())
        .map_err(|e| GrpcError::internal(&format!("chain_metadata borsh encode: {e:?}")))?;
    let expected = request_hash(
        *chain,
        unsigned_payload,
        &chain_metadata_bytes,
        *include_intermediate_output,
    );
    if expected != vpm.request_hash {
        return Err(PaymentVerifyError::RequestHashMismatch.into());
    }

    // Cross-check the gateway pubkey claimed in the VPM against the pinned
    // key. Both values are public keys, not secrets, so a plain compare is
    // fine; the actual trust decision is the signature check below.
    if !vpm
        .gateway_pubkey_hex
        .eq_ignore_ascii_case(pinned_hex_lower)
    {
        return Err(PaymentVerifyError::PinnedKeyMismatch.into());
    }

    let digest = vpm
        .signing_digest()
        .map_err(|e| GrpcError::internal(&format!("payment marker signing_digest: {e:?}")))?;
    pinned
        .verify(&digest, &signed.signature)
        .map_err(|_| PaymentVerifyError::BadSignature)?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{ChainMetadata, EthereumMetadata, chain_metadata};
    use host_primitives::payment_marker::{PaymentDetails, VerifiedPaymentMarker};
    use qos_p256::sign::P256SignPair;

    fn sign_with(pair: &P256SignPair, vpm: VerifiedPaymentMarker) -> Vec<u8> {
        let signed = SignedVerifiedPaymentMarker {
            signature: pair.sign(&vpm.signing_digest().unwrap()).unwrap(),
            vpm,
        };
        borsh::to_vec(&signed).unwrap()
    }

    fn make_vpm(req: &ParseRequest, gateway_hex: &str) -> VerifiedPaymentMarker {
        let chain_metadata_bytes = chain_metadata_bytes(req.chain_metadata.as_ref()).unwrap();
        VerifiedPaymentMarker {
            version: VPM_VERSION,
            request_hash: request_hash(
                req.chain,
                &req.unsigned_payload,
                &chain_metadata_bytes,
                req.include_intermediate_output,
            ),
            details: PaymentDetails::X402Direct {
                txid: "txsig".into(),
                payer: "Pay".into(),
                pay_to: "Recv".into(),
                amount: "1000".into(),
                mint: "Mint".into(),
                x_payment_hash: [0u8; 32],
                network: "solana:test".into(),
            },
            settled_at_ms: 0,
            gateway_pubkey_hex: gateway_hex.to_string(),
        }
    }

    fn req_with_marker(marker: Vec<u8>) -> ParseRequest {
        ParseRequest {
            unsigned_payload: "0xdeadbeef".into(),
            chain: 1,
            chain_metadata: None,
            include_intermediate_output: false,
            payment_marker: marker,
        }
    }

    /// Generates a fresh gateway keypair and a `Required` policy pinned to it.
    fn generate_policy() -> (P256SignPair, String, PaymentPolicy) {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();
        (pair, pub_hex, policy)
    }

    #[test]
    fn disabled_policy_accepts_anything() {
        let req = req_with_marker(vec![]);
        verify(&req, &PaymentPolicy::Disabled).unwrap();
    }

    #[test]
    fn required_policy_accepts_valid_marker() {
        let (pair, pub_hex, policy) = generate_policy();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_hex);
        req.payment_marker = sign_with(&pair, vpm);

        verify(&req, &policy).unwrap();
    }

    #[test]
    fn required_policy_rejects_missing_marker() {
        let (_pair, _pub_hex, policy) = generate_policy();
        let req = req_with_marker(vec![]);
        let err = verify(&req, &policy).unwrap_err();
        assert_eq!(err.code, Code::FailedPrecondition);
    }

    #[test]
    fn required_policy_rejects_request_hash_mismatch() {
        let (pair, pub_hex, policy) = generate_policy();

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
        let (_pair_a, _pub_a, policy) = generate_policy();
        let pair_b = P256SignPair::generate();
        let pub_b = qos_hex::encode(&pair_b.public_key().to_bytes());

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_b); // claims a different key
        req.payment_marker = sign_with(&pair_b, vpm);

        let err = verify(&req, &policy).unwrap_err();
        assert!(err.message.contains("pinned"));
    }

    #[test]
    fn required_policy_rejects_replay_with_different_chain_metadata() {
        // A marker paid+signed for a request with no chain_metadata must not
        // verify against a request that is otherwise identical but carries
        // different chain_metadata (which changes the enclave's own signed
        // output). Regression test for request_hash covering all of
        // ParseRequest, not just (chain, unsigned_payload).
        let (pair, pub_hex, policy) = generate_policy();

        let req_no_metadata = req_with_marker(vec![]);
        let vpm = make_vpm(&req_no_metadata, &pub_hex);
        let marker = sign_with(&pair, vpm);

        let mut req_with_metadata = req_with_marker(marker);
        req_with_metadata.chain_metadata = Some(ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: std::collections::BTreeMap::default(),
            })),
        });

        let err = verify(&req_with_metadata, &policy).unwrap_err();
        assert!(err.message.contains("request_hash"));
    }

    #[test]
    fn required_policy_rejects_replay_with_different_intermediate_output_flag() {
        // Same replay concern as above, but toggling
        // include_intermediate_output instead of chain_metadata.
        let (pair, pub_hex, policy) = generate_policy();

        let req_without_flag = req_with_marker(vec![]);
        let vpm = make_vpm(&req_without_flag, &pub_hex);
        let marker = sign_with(&pair, vpm);

        let mut req_with_flag = req_with_marker(marker);
        req_with_flag.include_intermediate_output = true;

        let err = verify(&req_with_flag, &policy).unwrap_err();
        assert!(err.message.contains("request_hash"));
    }

    #[test]
    fn required_policy_rejects_forged_marker_claiming_pinned_key() {
        // Attacker claims the pinned gateway key in gateway_pubkey_hex (so
        // the string precheck passes) but actually signs with a different
        // keypair. This must fail at the signature check, not pass because
        // the claimed-key string happened to match.
        let (_pinned_pair, pinned_pub_hex, policy) = generate_policy();
        let attacker_pair = P256SignPair::generate();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pinned_pub_hex); // claims the pinned key
        req.payment_marker = sign_with(&attacker_pair, vpm); // signed by someone else

        let err = verify(&req, &policy).unwrap_err();
        assert_eq!(err.code, Code::FailedPrecondition);
        assert!(err.message.contains("signature"));
    }

    #[test]
    fn required_policy_rejects_tampered_signature() {
        let (pair, pub_hex, policy) = generate_policy();

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
