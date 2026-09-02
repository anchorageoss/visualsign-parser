//! VerifiedPaymentMarker verification inside parser_app.
//!
//! Only checks the gateway's signature + binds the marker to this specific
//! request. The deeper buyer-Ed25519-on-the-Solana-tx check is deferred to
//! v3.1 (see plan).
//!
//! Policy is meant to be built once at startup from a hex-encoded pinned
//! gateway pubkey via `PaymentPolicy::from_hex`, with the binary deciding
//! where that hex value comes from (CLI arg / env). No binary does that
//! yet: every call site currently passes `Disabled`, so this module is the
//! enforcement point sitting in place, switched off. Local dev / gRPC-direct
//! callers pass `PaymentPolicy::Disabled` and VPM is not required. When
//! `Required`, the policy refuses any request whose `payment_marker` doesn't
//! carry a valid gateway-signed VPM bound to the exact request body.

use borsh::BorshDeserialize;
use generated::google::rpc::Code;
use generated::parser::{ChainMetadata, ParseRequest};
use host_primitives::payment_marker::{SignedVerifiedPaymentMarker, VPM_VERSION, request_hash};
use qos_p256::sign::P256SignPublic;
use visualsign::encodings::decode_hex;

use crate::errors::GrpcError;

/// Borsh-encodes `chain_metadata`, or returns an empty `Vec` if absent.
/// Matches (and is reused by) the convention used for
/// `ParsedTransactionPayload::metadata_digest` in `routes::parse`.
pub(crate) fn chain_metadata_bytes(
    metadata: Option<&ChainMetadata>,
) -> Result<Vec<u8>, borsh::io::Error> {
    metadata
        .map(borsh::to_vec)
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Whether `parser_app` requires (and verifies) a `VerifiedPaymentMarker`
/// on every parse call. Built by the binary from its own config via
/// `PaymentPolicy::from_hex`; there is no env-coupled constructor here (see
/// the module doc above).
pub enum PaymentPolicy {
    /// No payment enforcement. Used by local-dev / direct-gRPC callers, and
    /// by every call site today (no binary constructs `Required` yet).
    Disabled,
    /// Require a valid gateway-signed VPM in `ParseRequest.payment_marker`.
    Required {
        /// The gateway's P256 signing public key, pinned at TVC deploy
        /// time via `GATEWAY_SIGNING_PUBKEY_HEX`.
        pinned: P256SignPublic,
    },
}

impl std::fmt::Debug for PaymentPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "PaymentPolicy::Disabled"),
            Self::Required { pinned } => {
                // Encoded on demand rather than memoized: this is log/Debug
                // output only, not the hot verify() path, and deriving it
                // from `pinned` (not the raw config string) keeps it the
                // canonical unprefixed lower-case encoding regardless of
                // whether the operator supplied a `0x`-prefixed value in
                // `GATEWAY_SIGNING_PUBKEY_HEX`. The cross-check against
                // `vpm.gateway_pubkey_hex` compares decoded bytes, not this
                // string.
                f.debug_struct("PaymentPolicy::Required")
                    .field("pinned_hex", &qos_hex::encode(&pinned.to_bytes()))
                    .finish()
            }
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
        Ok(Self::Required { pinned })
    }
}

/// Reasons a request can be rejected for missing or invalid payment proof.
#[derive(Debug, thiserror::Error)]
pub enum PaymentVerifyError {
    /// `payment_marker` was empty in `Required` mode.
    #[error("payment marker is required for this endpoint")]
    Missing,
    /// The marker bytes weren't valid Borsh, or a decoded field didn't match
    /// the schema (e.g. `gateway_pubkey_hex` that isn't valid hex).
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
    /// A failure unrelated to payment status (e.g. Borsh-encoding this
    /// request's own `chain_metadata` to compute `request_hash`). Kept
    /// distinct from the payment-conditional variants above so a caller
    /// that matches on `PaymentVerifyError` for logging/metrics doesn't see
    /// a deployment bug reported as a payment failure.
    #[error("internal error verifying payment marker: {0}")]
    Internal(String),
}

impl From<PaymentVerifyError> for GrpcError {
    fn from(e: PaymentVerifyError) -> Self {
        // `FailedPrecondition` is meant to be translated to HTTP 402 by the
        // gateway. We keep parser_app HTTP-unaware; the gateway is
        // responsible for mapping gRPC status codes to HTTP and
        // synthesizing the canonical x402 PaymentRequired body from its
        // own config (not yet wired as of this policy being `Disabled`
        // everywhere).
        //
        // Only the payment-conditional variants get that treatment. Corrupt
        // marker bytes (`Decode`) and schema skew (`UnsupportedVersion`) are
        // caller or deployment bugs, not "you have not paid": under x402
        // retry semantics a 402 tells the caller to pay again for a request
        // that cannot succeed however many times it retries. Those map to
        // `InvalidArgument` so the gateway surfaces a 400 instead.
        let code = match &e {
            PaymentVerifyError::Decode(_) | PaymentVerifyError::UnsupportedVersion(_) => {
                Code::InvalidArgument
            }
            PaymentVerifyError::Missing
            | PaymentVerifyError::RequestHashMismatch
            | PaymentVerifyError::PinnedKeyMismatch
            | PaymentVerifyError::BadSignature => Code::FailedPrecondition,
            PaymentVerifyError::Internal(_) => Code::Internal,
        };
        GrpcError::new(code, &format!("{e}"))
    }
}

/// Upper bound on `payment_marker` bytes accepted before Borsh decoding.
///
/// A real marker (fixed 32-byte arrays, a handful of short base58/hex
/// strings, a 64-byte signature) serializes to well under 1 KiB. The gRPC
/// server otherwise only caps `payment_marker` at the whole-request size
/// (`GRPC_MAX_RECV_MSG_SIZE`, 25 MiB), so without this an attacker who
/// cannot forge a signature could still make every rejected request pay
/// for decoding, and later re-serializing, up to 25 MiB of unauthenticated
/// bytes before the signature check ever runs. 8 KiB leaves headroom for
/// added fields while cutting that cost by three orders of magnitude.
const MAX_PAYMENT_MARKER_BYTES: usize = 8 * 1024;

/// Returns `Ok(())` if the policy allows the request to proceed.
pub fn verify(
    parse_request: &ParseRequest,
    policy: &PaymentPolicy,
) -> Result<(), PaymentVerifyError> {
    let pinned = match policy {
        PaymentPolicy::Disabled => return Ok(()),
        PaymentPolicy::Required { pinned } => pinned,
    };

    if parse_request.payment_marker.is_empty() {
        return Err(PaymentVerifyError::Missing);
    }

    if parse_request.payment_marker.len() > MAX_PAYMENT_MARKER_BYTES {
        return Err(PaymentVerifyError::Decode(format!(
            "payment marker is {} bytes, exceeds {MAX_PAYMENT_MARKER_BYTES} byte limit",
            parse_request.payment_marker.len()
        )));
    }

    let signed = SignedVerifiedPaymentMarker::try_from_slice(&parse_request.payment_marker)
        .map_err(|e| PaymentVerifyError::Decode(format!("{e}")))?;

    let vpm = &signed.vpm;

    if vpm.version != VPM_VERSION {
        return Err(PaymentVerifyError::UnsupportedVersion(vpm.version));
    }

    // Exhaustive on purpose, mirroring the `ParseRequest` destructure below:
    // `PaymentDetails` has one variant today, but a new scheme is a new
    // variant (see host_primitives::payment_marker doc), and this has no
    // wildcard arm so adding one is a compile error here, forcing an
    // explicit accept/reject decision for the new scheme rather than
    // silently accepting it. This is separate from cross-checking a
    // variant's settlement fields against forwarded X-PAYMENT bytes, which
    // is deferred to v3.1.
    match &vpm.details {
        host_primitives::payment_marker::PaymentDetails::X402Direct { .. } => {}
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
        .map_err(|e| PaymentVerifyError::Internal(format!("chain_metadata borsh encode: {e:?}")))?;
    let expected = request_hash(
        *chain,
        unsigned_payload,
        &chain_metadata_bytes,
        *include_intermediate_output,
    );
    if expected != vpm.request_hash {
        return Err(PaymentVerifyError::RequestHashMismatch);
    }

    // Cross-check the gateway pubkey claimed in the VPM against the pinned
    // key. Compare decoded bytes rather than the hex strings: `decode_hex`
    // accepts an optional `0x` prefix, exactly as `PaymentPolicy::from_hex`
    // does for the operator-supplied value, so a signer that reads the
    // `gateway_pubkey_hex` field doc literally and writes a prefixed key
    // still matches instead of failing every request with a valid signature
    // underneath. Both values are public keys, not secrets, so a plain
    // compare is fine; the actual trust decision is the signature check
    // below.
    // A `gateway_pubkey_hex` that isn't even valid hex is a malformed marker,
    // not an unpaid request, so it takes the `Decode` path (InvalidArgument)
    // rather than `PinnedKeyMismatch` (FailedPrecondition -> 402). Telling a
    // caller to pay again cannot fix a marker whose key field is corrupt.
    //
    // The decode error is deliberately not included: `hex::FromHexError`'s
    // `Display` embeds the offending character verbatim, and this field is
    // unauthenticated and attacker-controlled at this point (the signature
    // has not been checked yet), so echoing it into a client-visible gRPC
    // message would put attacker bytes in our error strings and logs.
    let claimed = decode_hex(vpm.gateway_pubkey_hex.trim())
        .map_err(|_| PaymentVerifyError::Decode("gateway_pubkey_hex is not valid hex".into()))?;
    // Valid hex of the wrong length (e.g. a truncated or padded key) falls
    // through to `PinnedKeyMismatch` (FailedPrecondition -> 402) rather than
    // `Decode` (InvalidArgument -> 400) above, even though both are
    // malformed-marker cases. Left as-is: this compares decoded bytes
    // against the pinned key regardless of length, so a length mismatch is
    // just a specific case of "does not match the pinned key". Revisit if
    // deploy-time key skew (a rotated gateway key with a stale pin) turns
    // out to be common enough that InvalidArgument's clearer "this can
    // never succeed" signal is worth splitting out here too.
    if claimed[..] != pinned.to_bytes()[..] {
        return Err(PaymentVerifyError::PinnedKeyMismatch);
    }

    let digest = vpm.signing_digest().map_err(|e| {
        PaymentVerifyError::Internal(format!("payment marker signing_digest: {e:?}"))
    })?;
    pinned
        .verify(&digest, &signed.signature)
        .map_err(|_| PaymentVerifyError::BadSignature)?;

    Ok(())
}

/// Shared VPM test-construction helpers. `pub(crate)` (rather than nested
/// inside `mod tests` below) so `routes::parse`'s tests can build the same
/// well-formed markers instead of re-deriving `VerifiedPaymentMarker`
/// construction from scratch.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod test_support {
    use super::{PaymentPolicy, chain_metadata_bytes};
    use generated::parser::ParseRequest;
    use host_primitives::payment_marker::{
        PaymentDetails, SignedVerifiedPaymentMarker, VPM_VERSION, VerifiedPaymentMarker,
        request_hash,
    };
    use qos_p256::sign::P256SignPair;

    pub(crate) fn sign_with(pair: &P256SignPair, vpm: VerifiedPaymentMarker) -> Vec<u8> {
        let signed = SignedVerifiedPaymentMarker {
            signature: pair.sign(&vpm.signing_digest().unwrap()).unwrap(),
            vpm,
        };
        borsh::to_vec(&signed).unwrap()
    }

    pub(crate) fn make_vpm(req: &ParseRequest, gateway_hex: &str) -> VerifiedPaymentMarker {
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

    pub(crate) fn req_with_marker(marker: Vec<u8>) -> ParseRequest {
        ParseRequest {
            unsigned_payload: "0xdeadbeef".into(),
            chain: 1,
            chain_metadata: None,
            include_intermediate_output: false,
            payment_marker: marker,
        }
    }

    /// Generates a fresh gateway keypair and a `Required` policy pinned to it.
    pub(crate) fn generate_policy() -> (P256SignPair, String, PaymentPolicy) {
        let pair = P256SignPair::generate();
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let policy = PaymentPolicy::from_hex(&pub_hex).unwrap();
        (pair, pub_hex, policy)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::test_support::{generate_policy, make_vpm, req_with_marker, sign_with};
    use super::*;
    use generated::parser::{
        ChainMetadata, EthereumMetadata, Idl, NearMetadata, SolanaIdlType, SolanaMetadata,
        chain_metadata,
    };
    use qos_p256::sign::P256SignPair;

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
        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::FailedPrecondition);
    }

    #[test]
    fn required_policy_rejects_oversized_marker_before_decoding() {
        // A marker larger than MAX_PAYMENT_MARKER_BYTES must be rejected on
        // size alone, before any Borsh decode of the attacker-controlled
        // bytes is attempted.
        let (_pair, _pub_hex, policy) = generate_policy();
        let oversized = vec![0u8; MAX_PAYMENT_MARKER_BYTES + 1];
        let req = req_with_marker(oversized);

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::InvalidArgument);
        assert!(err.message.contains("exceeds"));
    }

    #[test]
    fn required_policy_rejects_request_hash_mismatch() {
        let (pair, pub_hex, policy) = generate_policy();

        let req = req_with_marker(vec![]);
        let mut vpm = make_vpm(&req, &pub_hex);
        vpm.request_hash = [99u8; 32]; // does not match the actual request
        let marker = sign_with(&pair, vpm);
        let req = req_with_marker(marker);

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
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

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
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

        let err: GrpcError = verify(&req_with_metadata, &policy).unwrap_err().into();
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

        let err: GrpcError = verify(&req_with_flag, &policy).unwrap_err().into();
        assert!(err.message.contains("request_hash"));
    }

    #[test]
    fn required_policy_rejects_forged_marker_claiming_pinned_key() {
        // Attacker claims the pinned gateway key in gateway_pubkey_hex (so
        // the pubkey precheck passes) but actually signs with a different
        // keypair. This must fail at the signature check, not pass because
        // the claimed key happened to match the pinned one.
        let (_pinned_pair, pinned_pub_hex, policy) = generate_policy();
        let attacker_pair = P256SignPair::generate();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pinned_pub_hex); // claims the pinned key
        req.payment_marker = sign_with(&attacker_pair, vpm); // signed by someone else

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::FailedPrecondition);
        assert!(err.message.contains("signature"));
    }

    #[test]
    fn required_policy_accepts_prefixed_and_uppercase_gateway_pubkey_hex() {
        // `gateway_pubkey_hex` is cross-checked by decoded bytes, not by
        // string equality, so a signer that writes the key the way
        // `GATEWAY_SIGNING_PUBKEY_HEX` accepts it (optional `0x`, either
        // case) must still verify. Before this, such a marker failed with
        // PinnedKeyMismatch despite a valid signature underneath.
        let (pair, pub_hex, policy) = generate_policy();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &format!("0x{}", pub_hex.to_uppercase()));
        req.payment_marker = sign_with(&pair, vpm);

        verify(&req, &policy).unwrap();
    }

    #[test]
    fn required_policy_rejects_undecodable_gateway_pubkey_hex() {
        // A corrupt key field is a malformed marker, so it must land on the
        // InvalidArgument path with the other decode failures rather than on
        // the 402 path that asks the caller to pay again.
        let (pair, _pub_hex, policy) = generate_policy();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, "not-hex");
        req.payment_marker = sign_with(&pair, vpm);

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::InvalidArgument);
        assert!(err.message.contains("gateway_pubkey_hex"));
    }

    #[test]
    fn corrupt_marker_bytes_map_to_invalid_argument() {
        // Not payment-conditional: a truncated marker is a caller bug, and
        // a 402 would tell the caller to pay again for a request that can
        // never succeed.
        let (_pair, _pub_hex, policy) = generate_policy();
        let req = req_with_marker(vec![0xff; 4]);

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::InvalidArgument);
    }

    #[test]
    fn unsupported_version_maps_to_invalid_argument() {
        // Schema skew between gateway and enclave is a deployment bug, not
        // a missing payment.
        let (pair, pub_hex, policy) = generate_policy();

        let mut req = req_with_marker(vec![]);
        let mut vpm = make_vpm(&req, &pub_hex);
        vpm.version = VPM_VERSION + 1;
        req.payment_marker = sign_with(&pair, vpm);

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert_eq!(err.code, Code::InvalidArgument);
    }

    #[test]
    fn required_policy_rejects_tampered_signature() {
        let (pair, pub_hex, policy) = generate_policy();

        let mut req = req_with_marker(vec![]);
        let vpm = make_vpm(&req, &pub_hex);
        let mut marker = sign_with(&pair, vpm);
        // Flip the last byte (inside the signature region, the signature
        // is the tail of the borsh-encoded struct).
        let last = marker.len() - 1;
        marker[last] ^= 0xff;
        req.payment_marker = marker;

        let err: GrpcError = verify(&req, &policy).unwrap_err().into();
        assert!(err.message.contains("signature"));
    }

    #[test]
    fn chain_metadata_bytes_matches_hand_encoded_layout_for_ethereum_variant() {
        // `chain_metadata_bytes` (borsh(ChainMetadata)) is one-third of
        // `request_hash`'s preimage, alongside `unsigned_payload` and
        // `chain`, both of which already have hand-encoded pinned tests in
        // `host_primitives::payment_marker`. `prost`'s oneof codegen emits
        // struct fields in `.proto` declaration order, not tag order (see
        // `EthereumMetadata`: `network_id` is proto tag 2 but is declared,
        // and thus Borsh-serialized, before `abi_mappings`, proto tag 3),
        // and the enclosing oneof's Borsh variant index (Ethereum=0,
        // Solana=1, Near=2, by Rust declaration order) has no relationship
        // to the prost tags (1, 2, 3) either. A cosmetic-looking field or
        // variant reorder in `parser.proto` is wire-compatible from
        // `prost`'s point of view and produces no `make generated` diff
        // failure, but silently changes this preimage and breaks every
        // marker already minted against the old layout. This test
        // hand-assembles the expected bytes from the documented Borsh
        // encoding rules (LE integers, 1-byte `Option`/enum discriminants,
        // 4-byte-length-prefixed strings, a 4-byte-length-prefixed empty
        // `BTreeMap`) plus the current declaration order, rather than
        // deriving them from `borsh::to_vec` on the struct under test, so a
        // reorder fails this test instead of silently passing it.
        let network_id = "ETHEREUM_MAINNET";
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some(network_id.to_string()),
                abi_mappings: std::collections::BTreeMap::default(),
            })),
        };

        let mut expected = Vec::new();
        expected.push(1); // ChainMetadata.metadata: Option<Metadata>: Some
        expected.push(0); // Metadata variant index 0: Ethereum
        expected.push(1); // EthereumMetadata.network_id: Option<String>: Some
        expected.extend_from_slice(&u32::try_from(network_id.len()).unwrap().to_le_bytes());
        expected.extend_from_slice(network_id.as_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes()); // abi_mappings: empty BTreeMap

        assert_eq!(chain_metadata_bytes(Some(&metadata)).unwrap(), expected);
    }

    #[test]
    fn chain_metadata_bytes_matches_hand_encoded_layout_for_solana_variant() {
        // Companion to the Ethereum test above: pins the Solana variant's
        // discriminant (1) plus `SolanaMetadata`'s own declaration order
        // (`network_id` tag 2, then `idl` tag 1, then `idl_mappings` tag 3),
        // so swapping Solana and Near in the `.proto` oneof, or reordering
        // `SolanaMetadata`'s fields, fails this test instead of silently
        // changing the `request_hash` preimage.
        fn encode_idl(idl: &Idl, buf: &mut Vec<u8>) {
            buf.extend_from_slice(&u32::try_from(idl.value.len()).unwrap().to_le_bytes());
            buf.extend_from_slice(idl.value.as_bytes());
            match idl.idl_type {
                Some(t) => {
                    buf.push(1);
                    buf.extend_from_slice(&t.to_le_bytes());
                }
                None => buf.push(0),
            }
            match &idl.idl_version {
                Some(v) => {
                    buf.push(1);
                    buf.extend_from_slice(&u32::try_from(v.len()).unwrap().to_le_bytes());
                    buf.extend_from_slice(v.as_bytes());
                }
                None => buf.push(0),
            }
            buf.push(u8::from(idl.signature.is_some())); // None here
            match &idl.program_name {
                Some(n) => {
                    buf.push(1);
                    buf.extend_from_slice(&u32::try_from(n.len()).unwrap().to_le_bytes());
                    buf.extend_from_slice(n.as_bytes());
                }
                None => buf.push(0),
            }
        }

        let network_id = "SOLANA_MAINNET";
        let idl = Idl {
            value: "{}".to_string(),
            idl_type: Some(SolanaIdlType::Anchor as i32),
            idl_version: Some("0.30.0".to_string()),
            signature: None,
            program_name: Some("JupiterLend".to_string()),
        };
        let mut idl_mappings = std::collections::BTreeMap::new();
        idl_mappings.insert(
            "Prog11111111111111111111111111111111111111".to_string(),
            idl.clone(),
        );

        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                network_id: Some(network_id.to_string()),
                idl: Some(idl.clone()),
                idl_mappings,
            })),
        };

        let mut expected = Vec::new();
        expected.push(1); // ChainMetadata.metadata: Option<Metadata>: Some
        expected.push(1); // Metadata variant index 1: Solana
        expected.push(1); // SolanaMetadata.network_id: Option<String>: Some
        expected.extend_from_slice(&u32::try_from(network_id.len()).unwrap().to_le_bytes());
        expected.extend_from_slice(network_id.as_bytes());
        expected.push(1); // SolanaMetadata.idl: Option<Idl>: Some
        encode_idl(&idl, &mut expected);
        expected.extend_from_slice(&1u32.to_le_bytes()); // idl_mappings: 1 entry
        let key = "Prog11111111111111111111111111111111111111";
        expected.extend_from_slice(&u32::try_from(key.len()).unwrap().to_le_bytes());
        expected.extend_from_slice(key.as_bytes());
        encode_idl(&idl, &mut expected);

        assert_eq!(chain_metadata_bytes(Some(&metadata)).unwrap(), expected);
    }

    #[test]
    fn chain_metadata_bytes_matches_hand_encoded_layout_for_near_variant() {
        // Companion to the Ethereum/Solana tests above: pins the Near
        // variant's discriminant (2) and `NearMetadata`'s single field, so
        // a variant reorder that leaves Ethereum at index 0 but swaps
        // Solana/Near still fails somewhere in this trio.
        let network_id = "NEAR_MAINNET";
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(network_id.to_string()),
                token_mappings: std::collections::BTreeMap::default(),
            })),
        };

        let mut expected = Vec::new();
        expected.push(1); // ChainMetadata.metadata: Option<Metadata>: Some
        expected.push(2); // Metadata variant index 2: Near
        expected.push(1); // NearMetadata.network_id: Option<String>: Some
        expected.extend_from_slice(&u32::try_from(network_id.len()).unwrap().to_le_bytes());
        expected.extend_from_slice(network_id.as_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes()); // token_mappings: 0 entries

        assert_eq!(chain_metadata_bytes(Some(&metadata)).unwrap(), expected);
    }

    #[test]
    fn chain_metadata_bytes_is_empty_for_absent_metadata() {
        assert_eq!(chain_metadata_bytes(None).unwrap(), Vec::<u8>::new());
    }
}
