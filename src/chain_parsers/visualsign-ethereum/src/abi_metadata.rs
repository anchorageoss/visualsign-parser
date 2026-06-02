//! ABI metadata extraction and signature validation
//!
//! Converts `ChainMetadata` ABI mappings into an `AbiRegistry` and optionally
//! validates secp256k1 signatures attached to individual ABI entries.

use crate::abi_registry::{AbiKind, AbiRegistry};
use crate::embedded_abis::register_embedded_abi;
use generated::parser::{ChainMetadata, chain_metadata};
use k256::EncodedPoint;
#[cfg(any(test, feature = "dev-signing"))]
use k256::ecdsa::SigningKey;
#[cfg(any(test, feature = "dev-signing"))]
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature, VerifyingKey};
use visualsign::signing::SignerAllowlist;

/// Maximum size for ABI JSON from proto messages (1 MB).
/// File-based ABI loading has a 10 MB cap; proto-supplied ABIs use a tighter bound
/// since they arrive per-request and are deserialized on the hot path.
const MAX_ABI_JSON_BYTES: usize = 1_024 * 1_024;

/// Error type for ABI signature validation.
#[derive(Debug, thiserror::Error)]
enum AbiSignatureError {
    #[error("ABI signature validation failed: {0}")]
    Validation(String),
}

/// Extract and validate ABIs from `ChainMetadata`, if present.
///
/// Navigates `ChainMetadata -> Ethereum -> abi_mappings` and registers each ABI
/// with its contract address. Returns `None` if the metadata doesn't contain
/// any Ethereum ABI mappings.
///
/// The `chain_id` is needed to register address-to-ABI mappings in the registry.
///
/// # Security notes
///
/// - **Unsigned ABIs are rejected.** Every ABI mapping must carry a signature;
///   entries with `signature: None` are skipped with a warning. Without this check,
///   a wallet could supply any ABI for any address and dictate the human-readable
///   rendering of the call.
/// - **Every accepted entry's signature is validated**, using secp256k1 over a
///   domain-separated prehash that binds the chain id and the contract address to
///   the ABI JSON (see [`visualsign::signing::ethereum_metadata_prehash`]). Since
///   unsigned entries are rejected above, no ABI reaches the registry without a
///   verified signature.
/// - **Signatures bind chain id + contract address.** The prehash commits to the
///   resolved `chain_id` and the entry's map-key address, so a signature minted for
///   an ABI at one (chain, address) no longer verifies when replayed under a
///   different address or chain. Existing signatures must be re-issued.
/// - **Signers are checked against an authorized allowlist.** A verified signature
///   only proves the ABI was signed by *some* secp256k1 key. Validation additionally
///   requires the recovered signer to appear in an authorized-signer allowlist (see
///   [`authorized_abi_signers`]); unauthorized signers are rejected even when their
///   signature verifies. An EMPTY allowlist rejects all signed ABIs (fail-closed),
///   which is the secure default because the whole caller-supplied ABI path is
///   display-only and untrusted.
/// - **`abi_type` and `implementation_address` are NOT covered by the ABI
///   signature.** The signature is computed over `abi.value` (the JSON ABI string)
///   only, so a man-in-the-middle could flip a signed implementation ABI to
///   `Proxy` or repoint `implementation_address` without invalidating the
///   signature. This is acceptable because the whole caller-ABI decode path is
///   untrusted and display-only: proxy resolution runs strictly after the
///   known-token short-circuit (so canonical tokens can never be redirected), and
///   an attacker who can tamper metadata could already swap the bound ABI itself.
///   The full `ChainMetadata` (including these fields) is still committed to by
///   `metadata_digest` in the signed enclave output.
pub fn try_extract_from_chain_metadata(
    chain_metadata: Option<&ChainMetadata>,
    chain_id: u64,
    allowlist: &SignerAllowlist,
) -> Option<AbiRegistry> {
    let chain_metadata = chain_metadata?;
    let chain_metadata::Metadata::Ethereum(ethereum) = chain_metadata.metadata.as_ref()? else {
        return None;
    };
    if ethereum.abi_mappings.is_empty() {
        return None;
    }

    let mut registry = AbiRegistry::new();
    for (address, abi) in &ethereum.abi_mappings {
        // Validate address first (cheap) before expensive signature/ABI operations
        let parsed_address = match address.parse::<alloy_primitives::Address>() {
            Ok(addr) => addr,
            Err(e) => {
                log::warn!("Skipping ABI mapping with invalid address '{address}': {e}");
                continue;
            }
        };

        // Reject oversized ABI JSON before expensive operations
        if abi.value.len() > MAX_ABI_JSON_BYTES {
            log::warn!(
                "Skipping ABI mapping for '{address}': exceeds size limit ({} bytes > {MAX_ABI_JSON_BYTES})",
                abi.value.len()
            );
            continue;
        }

        // Reject unsigned ABI entries unconditionally. Allowing
        // signature: None would let a wallet supply arbitrary ABIs for any
        // address and steer the human-readable rendering of the call.
        let Some(proto_sig) = abi.signature.as_ref() else {
            log::warn!(
                "Skipping ABI mapping for '{address}': missing signature (unsigned ABI entries are rejected)"
            );
            continue;
        };
        let signature = convert_proto_signature(proto_sig);
        if let Err(e) =
            validate_abi_signature(&abi.value, &parsed_address, chain_id, &signature, allowlist)
        {
            log::warn!("Skipping ABI mapping for '{address}': signature validation failed: {e}");
            continue;
        }

        // Determine the kind of contract this ABI describes. An unset or
        // unspecified type collapses to `Implementation`, preserving today's
        // behaviour. `abi_type` is not covered by the ABI signature; see the
        // module-level security notes.
        let (abi_kind, implementation) = resolve_abi_kind(abi);

        match register_embedded_abi(&mut registry, address, &abi.value) {
            Ok(()) => {
                registry.map_address_with_type(
                    chain_id,
                    parsed_address,
                    address,
                    abi_kind,
                    implementation,
                );
            }
            Err(e) => {
                log::warn!("Skipping ABI mapping for '{address}': {e}");
            }
        }
    }
    if registry.list_abis().is_empty() {
        return None;
    }
    Some(registry)
}

/// Resolve the `AbiKind` and (for proxies) the implementation address from a proto
/// `Abi` entry.
///
/// An unset or `ABI_TYPE_UNSPECIFIED`/`ABI_TYPE_IMPLEMENTATION` type maps to
/// `Implementation`. `ABI_TYPE_PROXY` maps to `Proxy`; its `implementation_address`
/// is parsed best-effort, and a missing or malformed address yields a proxy with no
/// link (decoding falls back to the proxy's own ABI) rather than dropping the entry.
fn resolve_abi_kind(abi: &generated::parser::Abi) -> (AbiKind, Option<alloy_primitives::Address>) {
    let proto_type = abi
        .abi_type
        .and_then(generated::parser::AbiType::from_i32)
        .unwrap_or(generated::parser::AbiType::Unspecified);

    match proto_type {
        generated::parser::AbiType::Proxy => {
            let implementation = match abi.implementation_address.as_deref() {
                Some(addr) => match addr.parse::<alloy_primitives::Address>() {
                    Ok(parsed) => Some(parsed),
                    Err(e) => {
                        log::warn!(
                            "Proxy ABI has invalid implementation_address '{addr}': {e}; \
                             falling back to the proxy's own ABI"
                        );
                        None
                    }
                },
                None => {
                    log::warn!(
                        "Proxy ABI has no implementation_address; \
                         falling back to the proxy's own ABI"
                    );
                    None
                }
            };
            (AbiKind::Proxy, implementation)
        }
        generated::parser::AbiType::Unspecified | generated::parser::AbiType::Implementation => {
            (AbiKind::Implementation, None)
        }
    }
}

/// Convert protobuf `SignatureMetadata` (key-value pairs) to local `SignatureMetadata`.
fn convert_proto_signature(proto: &generated::parser::SignatureMetadata) -> SignatureMetadata {
    let get = |key: &str| -> Option<String> {
        proto
            .metadata
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value.clone())
    };

    SignatureMetadata {
        value: proto.value.clone(),
        algorithm: get("algorithm"),
        public_key: get("public_key"),
        issuer: get("issuer"),
        timestamp: get("timestamp"),
    }
}

/// The only supported signature algorithm.
const SUPPORTED_ALGORITHM: &str = "secp256k1";

/// ABI signature metadata for validation
///
/// Mirrors the protobuf `SignatureMetadata` structure in a local type
#[derive(Debug, Clone)]
struct SignatureMetadata {
    /// Signature value (hex-encoded, DER format for secp256k1)
    value: String,
    /// Algorithm used (e.g., "secp256k1")
    algorithm: Option<String>,
    /// Public key for signature verification (hex-encoded)
    public_key: Option<String>,
    /// Issuer of the signature (mirrors proto field; not used in validation)
    #[allow(dead_code)]
    issuer: Option<String>,
    /// Timestamp of signature (mirrors proto field; not used in validation)
    #[allow(dead_code)]
    timestamp: Option<String>,
}

/// Validate ABI using secp256k1 signature, enforcing an authorized-signer allowlist.
///
/// The prehash is domain-separated and binds the chain id and contract address to
/// the ABI JSON, so a signature minted for one (chain, address) does not verify when
/// replayed under another. See [`visualsign::signing::ethereum_metadata_prehash`].
///
/// Both checks must pass: the signature must verify over the prehash AND the
/// recovered signer must appear in `allowlist`. An empty allowlist rejects every
/// signed ABI (fail-closed); see [`authorized_abi_signers`].
///
/// # Arguments
/// * `abi_json` - The ABI JSON string that was signed
/// * `address` - The contract address the ABI is bound to (the entry's map key)
/// * `chain_id` - The resolved chain id the ABI is bound to
/// * `signature` - Signature and metadata for validation
/// * `allowlist` - Authorized signer public keys (canonical uncompressed bytes)
///
/// # Returns
/// * `Ok(())` if the signature is valid and the signer is authorized
/// * `Err(AbiSignatureError)` if signature validation fails or the signer is not
///   in the allowlist
fn validate_abi_signature(
    abi_json: &str,
    address: &alloy_primitives::Address,
    chain_id: u64,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<(), AbiSignatureError> {
    // 1. Get algorithm - must be secp256k1
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| AbiSignatureError::Validation("Missing algorithm".to_string()))?;

    if algorithm != SUPPORTED_ALGORITHM {
        return Err(AbiSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {SUPPORTED_ALGORITHM} is supported."
        )));
    }

    // 2. Get public key
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| AbiSignatureError::Validation("Missing public_key".to_string()))?;

    // 3. Compute the domain-separated prehash binding chain id + contract address
    //    to the ABI JSON.
    let hash = visualsign::signing::ethereum_metadata_prehash(
        chain_id,
        &address.into_array(),
        abi_json.as_bytes(),
    );

    // 4. Decode signature (DER format) from hex
    let sig_hex = signature
        .value
        .strip_prefix("0x")
        .unwrap_or(&signature.value);
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid signature hex: {e}")))?;

    let sig = Signature::from_der(&sig_bytes)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid DER signature: {e}")))?;

    // 5. Decode public key from hex
    let pubkey_hex = public_key_hex.strip_prefix("0x").unwrap_or(public_key_hex);
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid public key hex: {e}")))?;

    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid public key point: {e}")))?;

    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point)
        .map_err(|e| AbiSignatureError::Validation(format!("Invalid verifying key: {e}")))?;

    // 6. Verify pre-hashed signature (hash was computed in step 3)
    verifying_key.verify_prehash(&hash, &sig).map_err(|e| {
        AbiSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    // 7. Enforce the authorized-signer allowlist. A verified signature only proves
    //    the ABI was signed by some secp256k1 key; it must also be an authorized
    //    one. Compare on the canonical uncompressed SEC1 encoding so the lookup
    //    matches how keys are stored in the allowlist. An empty allowlist contains
    //    nothing, so this rejects every signed ABI (fail-closed).
    let signer_pubkey = verifying_key.to_encoded_point(false).as_bytes().to_vec();
    if !allowlist.contains(&signer_pubkey) {
        return Err(AbiSignatureError::Validation(
            "signer not in allowlist".to_string(),
        ));
    }

    Ok(())
}

/// Build the authorized ABI-signer allowlist from compile-time + runtime sources.
///
/// Under the `dev-signing` feature (and in this crate's own tests) the dev key
/// derived from [`CLI_DEV_SIGNING_KEY_SEED`] is allowlisted, so locally-signed ABIs
/// are accepted. The env var `VISUALSIGN_ETH_ABI_SIGNERS` (comma-separated hex
/// secp256k1 public keys, any SEC1 encoding) extends the allowlist for configured
/// deployments. Each runtime entry is canonicalized to its uncompressed encoding so
/// compressed and uncompressed inputs for the same key match.
///
/// An empty result (no dev key, no env entries) rejects all signed ABIs
/// (fail-closed), which is the secure default for the untrusted, display-only
/// caller-ABI path.
#[must_use]
pub fn authorized_abi_signers() -> SignerAllowlist {
    let mut allow = SignerAllowlist::new();

    #[cfg(any(test, feature = "dev-signing"))]
    {
        if let Ok(sk) = SigningKey::from_bytes(&CLI_DEV_SIGNING_KEY_SEED) {
            let vk = VerifyingKey::from(&sk);
            allow.insert(vk.to_encoded_point(false).as_bytes().to_vec());
        }
    }

    if let Ok(list) = std::env::var("VISUALSIGN_ETH_ABI_SIGNERS") {
        for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match canonical_pubkey_from_hex(entry) {
                Some(bytes) => allow.insert(bytes),
                None => log::warn!("Ignoring invalid pubkey in VISUALSIGN_ETH_ABI_SIGNERS"),
            }
        }
    }

    allow
}

/// Parse a hex secp256k1 public key (optionally `0x`-prefixed, any SEC1 encoding)
/// and return its canonical UNCOMPRESSED encoded-point bytes, or `None` if the input
/// is not a valid point. Canonicalizing here means a compressed input and an
/// uncompressed input for the same key both reduce to identical allowlist bytes.
fn canonical_pubkey_from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let trimmed = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(trimmed).ok()?;
    let encoded_point = EncodedPoint::from_bytes(&bytes).ok()?;
    let verifying_key = VerifyingKey::from_encoded_point(&encoded_point).ok()?;
    Some(verifying_key.to_encoded_point(false).as_bytes().to_vec())
}

/// Deterministic 32-byte secp256k1 seed used to sign ABI JSON in local dev tooling
/// (e.g. the `parser_cli --abi-json-mappings` flow). Not a production key. Any caller
/// that trusts identity (rather than integrity) must verify the public key against an
/// allowlist; see `try_extract_from_chain_metadata` security notes.
///
/// Gated behind the `dev-signing` cargo feature (and `cfg(test)` for the crate's own
/// unit tests). It is never present in production builds (the enclave binary and the
/// gRPC server do not enable the feature), so those binaries cannot derive the dev
/// keypair and mint ABI signatures the parser would accept.
#[cfg(any(test, feature = "dev-signing"))]
pub const CLI_DEV_SIGNING_KEY_SEED: [u8; 32] = [0x42u8; 32];

/// Sign `abi_json` with the given 32-byte secp256k1 seed and return a proto
/// `SignatureMetadata` ready to drop into `Abi.signature`.
///
/// Used by the CLI to attach an integrity signature to locally-loaded ABI files so
/// the metadata-ABI extraction path (which rejects unsigned entries) can
/// register them. The signature is over the domain-separated prehash that binds the
/// `chain_id` and contract `address` to `abi_json` (see
/// [`visualsign::signing::ethereum_metadata_prehash`]), so it is valid only for the
/// exact (chain, address) it was produced for, matching the verifier in
/// [`validate_abi_signature`]. The signature is DER-encoded and hex-stringified.
///
/// # Errors
/// Returns `Err` if the seed does not form a valid secp256k1 scalar or signing fails.
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_abi(
    abi_json: &str,
    address: &alloy_primitives::Address,
    chain_id: u64,
    signing_key_seed: &[u8; 32],
) -> Result<generated::parser::SignatureMetadata, String> {
    let signing_key = SigningKey::from_bytes(signing_key_seed)
        .map_err(|e| format!("invalid secp256k1 signing key seed: {e}"))?;
    let verifying_key = VerifyingKey::from(&signing_key);

    let hash = visualsign::signing::ethereum_metadata_prehash(
        chain_id,
        &address.into_array(),
        abi_json.as_bytes(),
    );

    let signature: Signature = signing_key
        .sign_prehash(&hash)
        .map_err(|e| format!("failed to sign ABI hash: {e}"))?;
    let signature_hex = hex::encode(signature.to_der().as_bytes());
    let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

    Ok(generated::parser::SignatureMetadata {
        value: signature_hex,
        metadata: vec![
            generated::parser::Metadata {
                key: "algorithm".to_string(),
                value: SUPPORTED_ALGORITHM.to_string(),
            },
            generated::parser::Metadata {
                key: "public_key".to_string(),
                value: public_key_hex,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use generated::parser::{Abi, EthereumMetadata, SolanaMetadata};
    use k256::ecdsa::SigningKey;
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    const VALID_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        }
    ]"#;

    /// Helper to create a valid signature for testing.
    ///
    /// The signature is over the domain-separated prehash binding `chain_id` and
    /// `address` to `abi_json`, matching what [`validate_abi_signature`] verifies.
    fn create_test_signature(abi_json: &str, address: &Address, chain_id: u64) -> (String, String) {
        // Use a deterministic test seed
        let seed: [u8; 32] = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed).expect("valid key");
        let verifying_key = VerifyingKey::from(&signing_key);

        // Compute the shared domain-separated prehash.
        let hash = visualsign::signing::ethereum_metadata_prehash(
            chain_id,
            &address.into_array(),
            abi_json.as_bytes(),
        );

        // Sign the pre-hashed content
        let signature: Signature = signing_key.sign_prehash(&hash).expect("signing failed");
        let signature_der = signature.to_der();
        let signature_hex = hex::encode(signature_der.as_bytes());

        // Get public key (uncompressed format)
        let public_key_hex = hex::encode(verifying_key.to_encoded_point(false).as_bytes());

        (signature_hex, public_key_hex)
    }

    /// Canonical uncompressed public-key bytes derived from a 32-byte seed.
    fn pubkey_bytes_from_seed(seed: &[u8; 32]) -> Vec<u8> {
        let signing_key = SigningKey::from_bytes(seed).expect("valid key");
        let verifying_key = VerifyingKey::from(&signing_key);
        verifying_key.to_encoded_point(false).as_bytes().to_vec()
    }

    /// Allowlist authorizing the deterministic test signer (`create_test_signature`
    /// and `signed_abi` both sign with seed `[0x42u8; 32]`). Built explicitly so the
    /// tests never depend on the env var or the dev-signing feature.
    fn test_signer_allowlist() -> SignerAllowlist {
        let mut allow = SignerAllowlist::new();
        allow.insert(pubkey_bytes_from_seed(&[0x42u8; 32]));
        allow
    }

    #[test]
    fn test_valid_signature_verification() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);

        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: Some("test".to_string()),
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &test_signer_allowlist());
        assert!(
            result.is_ok(),
            "Valid signature should verify: {:?}",
            result.err()
        );
    }

    /// Core regression for this change: a signature is valid only for the exact
    /// (chain, address) it was produced for. Signing VALID_ABI for (address A,
    /// chain 1) must verify under (A, 1) but fail under a different address (same
    /// chain) or a different chain (same address).
    #[test]
    fn test_signature_bound_to_address_and_chain_rejects_replay() {
        let addr_a = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let addr_b = "0x1111111111111111111111111111111111111111"
            .parse::<Address>()
            .expect("valid address B");

        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr_a, 1);
        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        let allow = test_signer_allowlist();
        // Valid for the exact (chain, address) it was produced for.
        assert!(
            validate_abi_signature(VALID_ABI, &addr_a, 1, &sig, &allow).is_ok(),
            "signature must verify for the bound (chain, address)"
        );
        // Same chain, different address: rejected.
        assert!(
            validate_abi_signature(VALID_ABI, &addr_b, 1, &sig, &allow).is_err(),
            "signature must not verify when replayed under a different address"
        );
        // Same address, different chain: rejected.
        assert!(
            validate_abi_signature(VALID_ABI, &addr_a, 137, &sig, &allow).is_err(),
            "signature must not verify when replayed under a different chain"
        );
    }

    /// A signature that verifies AND whose signer is in the allowlist is accepted.
    #[test]
    fn validate_abi_signature_accepts_allowlisted_signer() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);
        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        // Allowlist holds the test signer's uncompressed key bytes.
        let allow = test_signer_allowlist();
        assert!(
            validate_abi_signature(VALID_ABI, &addr, 1, &sig, &allow).is_ok(),
            "a verified signature from an allowlisted signer must be accepted"
        );
    }

    /// A signature that verifies but whose signer is NOT in the allowlist is rejected.
    #[test]
    fn validate_abi_signature_rejects_unlisted_signer() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);
        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        // Allowlist holds a DIFFERENT key (seed 0x43), so the seed-0x42 signer is
        // absent even though its signature verifies.
        let mut allow = SignerAllowlist::new();
        allow.insert(pubkey_bytes_from_seed(&[0x43u8; 32]));
        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &allow);
        assert!(
            result.is_err(),
            "a verified signature from an unlisted signer must be rejected"
        );
        assert!(
            result.unwrap_err().to_string().contains("not in allowlist"),
            "rejection must cite the allowlist check"
        );
    }

    /// An empty allowlist rejects every signed ABI, even a perfectly valid one
    /// (fail-closed default).
    #[test]
    fn validate_abi_signature_fails_closed_on_empty_allowlist() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);
        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        let empty = SignerAllowlist::new();
        assert!(empty.is_empty(), "precondition: allowlist is empty");
        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &empty);
        assert!(
            result.is_err(),
            "an empty allowlist must reject all signed ABIs (fail-closed)"
        );
    }

    #[test]
    fn test_tampering_detection() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);

        let sig = SignatureMetadata {
            value: signature_hex,
            algorithm: Some("secp256k1".to_string()),
            public_key: Some(public_key_hex),
            issuer: None,
            timestamp: None,
        };

        // Try to verify with tampered ABI
        let tampered_abi = r#"[{"type":"function","name":"approve"}]"#;
        let result = validate_abi_signature(tampered_abi, &addr, 1, &sig, &test_signer_allowlist());
        assert!(result.is_err(), "Tampered content should fail verification");
    }

    #[test]
    fn test_missing_algorithm_error() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: None,
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &test_signer_allowlist());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing algorithm"), "Error: {err}");
    }

    #[test]
    fn test_missing_public_key_error() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: None,
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &test_signer_allowlist());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing public_key"), "Error: {err}");
    }

    #[test]
    fn test_unsupported_algorithm_error() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let sig = SignatureMetadata {
            value: "deadbeef".to_string(),
            algorithm: Some("ed25519".to_string()),
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &test_signer_allowlist());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unsupported algorithm"), "Error: {err}");
    }

    #[test]
    fn test_invalid_signature_hex() {
        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let sig = SignatureMetadata {
            value: "not_hex".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: Some("deadbeef".to_string()),
            issuer: None,
            timestamp: None,
        };

        let result = validate_abi_signature(VALID_ABI, &addr, 1, &sig, &test_signer_allowlist());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid signature hex"), "Error: {err}");
    }

    #[test]
    fn test_signature_metadata_struct() {
        let sig = SignatureMetadata {
            value: "0xabc123".to_string(),
            algorithm: Some("secp256k1".to_string()),
            public_key: Some("04abcd1234".to_string()),
            issuer: Some("issuer.example.com".to_string()),
            timestamp: Some("2024-01-01T00:00:00Z".to_string()),
        };

        assert_eq!(sig.value, "0xabc123");
        assert_eq!(sig.algorithm, Some("secp256k1".to_string()));
        assert_eq!(sig.public_key, Some("04abcd1234".to_string()));
        assert_eq!(sig.issuer, Some("issuer.example.com".to_string()));
        assert_eq!(sig.timestamp, Some("2024-01-01T00:00:00Z".to_string()));
    }

    // --- try_extract_from_chain_metadata tests ---

    const TEST_ADDRESS: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";

    /// Builds the test fixture as a `BTreeMap` (crate determinism rule) and lets each
    /// call site `.into_iter().collect()` into the proto field's `HashMap`.
    fn make_abi_mappings(entries: Vec<(&str, Abi)>) -> std::collections::BTreeMap<String, Abi> {
        entries
            .into_iter()
            .map(|(addr, abi)| (addr.to_string(), abi))
            .collect()
    }

    /// Build a valid proto signature for `abi_json`, bound to `address` on chain 1,
    /// using the deterministic test key. `address` must be the map key the entry is
    /// stored under so the signature matches what `validate_abi_signature` verifies.
    ///
    /// If `address` is not a valid Ethereum address, the signature is bound to the
    /// zero address instead. That only happens for entries whose key fails the
    /// earlier address parse and are skipped before signature verification, so the
    /// bound address is never actually checked.
    fn signed_abi(abi_json: &str, address: &str) -> Abi {
        let addr = address.parse::<Address>().unwrap_or(Address::ZERO);
        let (signature_hex, public_key_hex) = create_test_signature(abi_json, &addr, 1);
        let proto_sig = generated::parser::SignatureMetadata {
            value: signature_hex,
            metadata: vec![
                generated::parser::Metadata {
                    key: "algorithm".to_string(),
                    value: "secp256k1".to_string(),
                },
                generated::parser::Metadata {
                    key: "public_key".to_string(),
                    value: public_key_hex,
                },
            ],
        };
        Abi {
            value: abi_json.to_string(),
            signature: Some(proto_sig),
            ..Default::default()
        }
    }

    #[test]
    fn test_try_extract_no_metadata() {
        assert!(try_extract_from_chain_metadata(None, 1, &test_signer_allowlist()).is_none());
    }

    #[test]
    fn test_try_extract_non_ethereum_metadata() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                network_id: None,
                idl: None,
                idl_mappings: Default::default(),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist()).is_none()
        );
    }

    #[test]
    fn test_try_extract_ethereum_without_abi() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: Default::default(),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist()).is_none()
        );
    }

    #[test]
    fn test_try_extract_valid_abi() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    signed_abi(VALID_ABI, TEST_ADDRESS),
                )])
                .into_iter()
                .collect(),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("should contain ABI");
        assert!(registry.list_abis().contains(&TEST_ADDRESS));
    }

    /// Regression: an ABI mapping that omits the signature must be rejected,
    /// even when the address and ABI JSON are otherwise valid. Without this check a
    /// wallet could supply arbitrary ABIs for any address and dictate the parsed
    /// payload rendering.
    #[test]
    fn test_try_extract_unsigned_abi_rejected() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: None,
                        ..Default::default()
                    },
                )])
                .into_iter()
                .collect(),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist()).is_none(),
            "unsigned ABI entries must be rejected"
        );
    }

    #[test]
    fn test_try_extract_valid_abi_with_signature() {
        use generated::parser::Metadata;

        let addr = TEST_ADDRESS.parse::<Address>().expect("valid test address");
        let (signature_hex, public_key_hex) = create_test_signature(VALID_ABI, &addr, 1);

        let proto_sig = generated::parser::SignatureMetadata {
            value: signature_hex.clone(),
            metadata: vec![
                Metadata {
                    key: "algorithm".to_string(),
                    value: "secp256k1".to_string(),
                },
                Metadata {
                    key: "public_key".to_string(),
                    value: public_key_hex.clone(),
                },
                Metadata {
                    key: "issuer".to_string(),
                    value: "test-issuer".to_string(),
                },
                Metadata {
                    key: "timestamp".to_string(),
                    value: "2024-01-01T00:00:00Z".to_string(),
                },
            ],
        };

        // Verify convert_proto_signature maps fields correctly
        let local_sig = convert_proto_signature(&proto_sig);
        assert_eq!(local_sig.value, signature_hex);
        assert_eq!(local_sig.algorithm, Some("secp256k1".to_string()));
        assert_eq!(local_sig.public_key, Some(public_key_hex));
        assert_eq!(local_sig.issuer, Some("test-issuer".to_string()));
        assert_eq!(
            local_sig.timestamp,
            Some("2024-01-01T00:00:00Z".to_string())
        );

        // Verify end-to-end: ABI with valid signature extracts successfully
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: Some(proto_sig),
                        ..Default::default()
                    },
                )])
                .into_iter()
                .collect(),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("should contain ABI");
        assert!(registry.list_abis().contains(&TEST_ADDRESS));
    }

    #[test]
    fn test_convert_proto_signature_missing_keys() {
        let proto_sig = generated::parser::SignatureMetadata {
            value: "sig_value".to_string(),
            metadata: vec![],
        };

        let local_sig = convert_proto_signature(&proto_sig);
        assert_eq!(local_sig.value, "sig_value");
        assert!(local_sig.algorithm.is_none());
        assert!(local_sig.public_key.is_none());
        assert!(local_sig.issuer.is_none());
        assert!(local_sig.timestamp.is_none());
    }

    #[test]
    fn test_try_extract_invalid_address_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    "not_an_address",
                    Abi {
                        value: VALID_ABI.to_string(),
                        signature: None,
                        ..Default::default()
                    },
                )])
                .into_iter()
                .collect(),
            })),
        };
        // Invalid entries are skipped; with no valid entries left, result is None
        assert!(
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist()).is_none()
        );
    }

    #[test]
    fn test_try_extract_invalid_abi_json_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    signed_abi("not valid json", TEST_ADDRESS),
                )])
                .into_iter()
                .collect(),
            })),
        };
        // Invalid ABI JSON is skipped; with no valid entries left, result is None.
        // The signature is valid, so this exercises the JSON parse rejection path
        // rather than short-circuiting on the unsigned check.
        assert!(
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist()).is_none()
        );
    }

    #[test]
    fn test_try_extract_mixed_valid_and_invalid() {
        let valid_address = TEST_ADDRESS;
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![
                    ("not_an_address", signed_abi(VALID_ABI, "not_an_address")),
                    (valid_address, signed_abi(VALID_ABI, valid_address)),
                ])
                .into_iter()
                .collect(),
            })),
        };
        // The valid entry should be registered; the invalid one skipped
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("should contain the valid ABI");
        assert!(registry.list_abis().contains(&valid_address));
    }

    // --- proxy / abi_type tests ---

    const PROXY_ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    const IMPL_ADDRESS: &str = "0x2222222222222222222222222222222222222222";

    fn parse_addr(s: &str) -> alloy_primitives::Address {
        s.parse().expect("valid address")
    }

    #[test]
    fn test_extract_default_type_is_implementation() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    TEST_ADDRESS,
                    signed_abi(VALID_ABI, TEST_ADDRESS),
                )])
                .into_iter()
                .collect(),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("has ABI");
        assert_eq!(
            registry.get_abi_kind(1, parse_addr(TEST_ADDRESS)),
            Some(AbiKind::Implementation)
        );
    }

    #[test]
    fn test_extract_proxy_links_to_implementation() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![
                    (
                        PROXY_ADDRESS,
                        Abi {
                            abi_type: Some(generated::parser::AbiType::Proxy as i32),
                            implementation_address: Some(IMPL_ADDRESS.to_string()),
                            ..signed_abi("[]", PROXY_ADDRESS)
                        },
                    ),
                    (IMPL_ADDRESS, signed_abi(VALID_ABI, IMPL_ADDRESS)),
                ])
                .into_iter()
                .collect(),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("has ABIs");

        assert_eq!(
            registry.get_abi_kind(1, parse_addr(PROXY_ADDRESS)),
            Some(AbiKind::Proxy)
        );
        let (impl_addr, impl_abi) = registry
            .get_implementation_abi(1, parse_addr(PROXY_ADDRESS))
            .expect("proxy resolves to implementation");
        assert_eq!(impl_addr, parse_addr(IMPL_ADDRESS));
        // Resolved ABI is the implementation's; the synthesized "[]" proxy ABI parses
        // to an empty function set.
        assert!(impl_abi.functions().any(|f| f.name == "transfer"));
    }

    #[test]
    fn test_extract_proxy_with_invalid_impl_address_keeps_proxy_without_link() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: make_abi_mappings(vec![(
                    PROXY_ADDRESS,
                    Abi {
                        abi_type: Some(generated::parser::AbiType::Proxy as i32),
                        implementation_address: Some("not_an_address".to_string()),
                        ..signed_abi(VALID_ABI, PROXY_ADDRESS)
                    },
                )])
                .into_iter()
                .collect(),
            })),
        };
        let registry =
            try_extract_from_chain_metadata(Some(&metadata), 1, &test_signer_allowlist())
                .expect("has ABI");
        // Entry is kept as a proxy, but with no resolvable implementation link.
        assert_eq!(
            registry.get_abi_kind(1, parse_addr(PROXY_ADDRESS)),
            Some(AbiKind::Proxy)
        );
        assert!(
            registry
                .get_implementation_abi(1, parse_addr(PROXY_ADDRESS))
                .is_none()
        );
        // The proxy's own ABI is still available for fallback decoding.
        assert!(
            registry
                .get_abi_for_address(1, parse_addr(PROXY_ADDRESS))
                .is_some()
        );
    }
}
