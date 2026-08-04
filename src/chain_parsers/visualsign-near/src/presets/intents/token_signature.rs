//! NEAR Intents token-metadata extraction and signature validation.
//!
//! Mirrors `visualsign-ethereum::abi_metadata` and
//! `visualsign-solana::idl::signature`: converts `ChainMetadata.near.token_mappings`
//! into a [`NearTokenRegistry`] override layer, optionally validating a signature
//! attached to each entry. Like `Abi.value`/`Idl.value`, `TokenMetadataEntry.value`
//! is signed verbatim as supplied -- the signature covers exactly those bytes,
//! not a re-derived encoding, so there is no field-order/escaping contract for
//! an external signer to get right.
//!
//! # Trust model: dispatch by asset origin, not by NEAR itself
//!
//! Every asset id renders as a NEAR-side identity (`nep141:...`), but the
//! underlying value may live on NEAR itself or have been bridged in from another
//! chain. Rather than a single NEAR-wide curator key, the signature curve is
//! chosen per entry by [`TokenOriginChain`]: `Ethereum` (and every EVM twin the
//! omni-bridge lists: Arbitrum, Base, BNB, Polygon, Abstract, HyperEVM) verifies
//! with secp256k1, reusing the same curve family as `visualsign-ethereum`'s ABI
//! curator identity; `Solana` (and its SVM twin, Fogo) verifies with ed25519,
//! matching `visualsign-solana`'s IDL curator identity; `Near` (and unset, which
//! defaults to it) verifies with ed25519 under a distinct, NEAR-only curator
//! identity. Chains the omni-bridge lists but this parser has no existing curve
//! support for (Bitcoin, Zcash, Starknet, Aptos) are not given bespoke curve
//! verification here; entries for such assets should omit `origin_chain` (or
//! leave it unset) and rely on the NEAR-native curator, or ship unsigned.
//!
//! Each curve has its own domain-separated prehash
//! ([`visualsign::signing::near_token_metadata_prehash`],
//! [`visualsign::signing::ethereum_token_metadata_prehash`],
//! [`visualsign::signing::solana_token_metadata_prehash`]) and its own
//! allowlist, kept separate from the Ethereum ABI / Solana IDL allowlists even
//! though the same physical key may be enrolled in both: revoking "trusted to
//! vouch for ABI decoding" must not silently also revoke (or, worse, leave
//! standing) "trusted to vouch for token metadata."
//!
//! As with the ABI/IDL paths, a signature that is present but fails to
//! validate is rejected outright: a present-but-invalid signature is a
//! stronger signal of tampering than simply omitting one. An unsigned entry
//! may still be accepted (not every caller signs yet), but only when the
//! deployment's [`MetadataTrustPolicy`] allows it and the asset isn't already
//! covered by the compiled-in `tokens::SEEDS` table -- an unsigned entry
//! fills a gap, it never overrides a curated value.

use std::sync::OnceLock;

use ed25519_dalek::{Signature as Ed25519Signature, VerifyingKey as Ed25519VerifyingKey};
use generated::parser::{ChainMetadata, TokenOriginChain, chain_metadata};
use k256::EncodedPoint;
#[cfg(any(test, feature = "dev-signing"))]
use k256::ecdsa::SigningKey as Secp256k1SigningKey;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature as Secp256k1Signature, VerifyingKey as Secp256k1VerifyingKey};
use serde::Deserialize;
use visualsign::signing::{MetadataTrustPolicy, SignerAllowlist};

use super::{NearTokenRegistry, TokenMeta, tokens};

/// The only supported ed25519 algorithm tag (Near and Solana origins).
const ED25519_ALGORITHM: &str = "ed25519";
/// The only supported secp256k1 algorithm tag (Ethereum origin).
const SECP256K1_ALGORITHM: &str = "secp256k1";

const ED25519_PUBLIC_KEY_LEN: usize = 32;
const ED25519_SIGNATURE_LEN: usize = 64;

/// Maximum size for a `TokenMetadataEntry.value` JSON string. Token metadata is
/// inherently tiny (a symbol and a decimals count); this bounds the untrusted,
/// per-request proto field before it is deserialized, mirroring
/// `MAX_ABI_JSON_BYTES` in `visualsign-ethereum::abi_metadata`.
const MAX_TOKEN_METADATA_VALUE_BYTES: usize = 1024;

/// Maximum accepted `decimals` value. `tokens::format_units` computes
/// `10u128.pow(u32::from(decimals))`, which overflows above 38 -- a remote
/// panic (debug) or a silently wrapped, wrong-looking amount (release, where
/// `overflow-checks` isn't enabled) from an unauthenticated request field, so
/// it is bounded here at ingest rather than defensively inside formatting.
const MAX_TOKEN_DECIMALS: u8 = 38;

/// Maximum accepted `symbol` length. Real NEP-141/ERC-20 symbols (even
/// bridged ones like `USDC.e`) are a handful of characters; this bounds an
/// unauthenticated request field before it reaches a rendered field, the
/// same reasoning as `MAX_TOKEN_DECIMALS`.
const MAX_TOKEN_SYMBOL_LEN: usize = 32;

/// Error type for token-metadata signature validation.
#[derive(Debug, thiserror::Error)]
pub enum TokenMetadataSignatureError {
    #[error("token metadata signature validation failed: {0}")]
    Validation(String),
}

/// Token-metadata signature metadata for validation. Mirrors the protobuf
/// `SignatureMetadata` structure in a local type.
#[derive(Debug, Clone)]
struct SignatureMetadata {
    value: String,
    algorithm: Option<String>,
    public_key: Option<String>,
}

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
    }
}

/// The shape of `TokenMetadataEntry.value` once parsed.
#[derive(Deserialize)]
struct TokenMetadataValue {
    symbol: String,
    decimals: u8,
}

fn decode_hex_fixed<const N: usize>(
    value: &str,
    what: &str,
) -> Result<[u8; N], TokenMetadataSignatureError> {
    let bytes = visualsign::encodings::decode_hex(value)
        .map_err(|e| TokenMetadataSignatureError::Validation(format!("Invalid {what} hex: {e}")))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        TokenMetadataSignatureError::Validation(format!(
            "Invalid {what} length: expected {N} bytes, got {}",
            v.len()
        ))
    })
}

/// Per-origin-chain allowlists for token-metadata curator keys, built once and
/// cached. Kept separate from the Ethereum ABI / Solana IDL allowlists (see
/// module docs).
pub struct TokenMetadataSignerAllowlists {
    near: SignerAllowlist,
    ethereum: SignerAllowlist,
    solana: SignerAllowlist,
}

/// Build the authorized token-metadata-signer allowlists from compile-time +
/// env-configured production lists, cached for the lifetime of the process.
///
/// - `VISUALSIGN_NEAR_TOKEN_SIGNERS`: comma-separated hex ed25519 public keys.
/// - `VISUALSIGN_ETH_TOKEN_SIGNERS`: comma-separated hex secp256k1 public keys
///   (any SEC1 encoding).
/// - `VISUALSIGN_SOL_TOKEN_SIGNERS`: comma-separated hex ed25519 public keys.
///
/// Under the `dev-signing` feature (and this crate's own tests), the NEAR
/// dev key derived from [`DEV_NEAR_SIGNING_KEY_SEED`] is also allowlisted,
/// matching `visualsign-ethereum`'s `authorized_abi_signers()` -- without
/// this, `sign_token_metadata_for_cli`'s own signature would never verify,
/// since it always signs with that key and the env var alone is empty in a
/// local dev run.
///
/// An unset (or entirely invalid) env var leaves that chain's allowlist with
/// only the dev-signing entry above (empty outside `dev-signing`), which
/// rejects every other signed entry for that origin chain (fail-closed). These
/// allowlists gate only entries that carry a signature; whether an unsigned
/// entry is accepted at all is controlled separately by the deployment's
/// [`MetadataTrustPolicy`] and by the gap-fill-only rule in
/// [`try_extract_from_chain_metadata`] (an unsigned entry is only accepted for
/// an asset `tokens::SEEDS` doesn't already cover).
#[must_use]
pub fn authorized_token_metadata_signers() -> &'static TokenMetadataSignerAllowlists {
    static ALLOW: OnceLock<TokenMetadataSignerAllowlists> = OnceLock::new();
    ALLOW.get_or_init(|| TokenMetadataSignerAllowlists {
        near: with_near_dev_signer(build_ed25519_allowlist("VISUALSIGN_NEAR_TOKEN_SIGNERS")),
        ethereum: build_secp256k1_allowlist("VISUALSIGN_ETH_TOKEN_SIGNERS"),
        solana: build_ed25519_allowlist("VISUALSIGN_SOL_TOKEN_SIGNERS"),
    })
}

/// Add the NEAR dev key to `allow`, so entries signed by
/// [`sign_token_metadata_for_cli`] verify in a local dev run.
#[cfg(any(test, feature = "dev-signing"))]
fn with_near_dev_signer(mut allow: SignerAllowlist) -> SignerAllowlist {
    allow.insert(
        ed25519_dalek::SigningKey::from_bytes(&DEV_NEAR_SIGNING_KEY_SEED)
            .verifying_key()
            .to_bytes()
            .to_vec(),
    );
    allow
}

/// Without `dev-signing` the dev key is not linked, so the allowlist carries
/// only what the env var configured.
#[cfg(not(any(test, feature = "dev-signing")))]
fn with_near_dev_signer(allow: SignerAllowlist) -> SignerAllowlist {
    allow
}

fn build_ed25519_allowlist(env_var: &str) -> SignerAllowlist {
    let mut allow = SignerAllowlist::new();
    if let Ok(list) = std::env::var(env_var) {
        for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match canonical_ed25519_pubkey_from_hex(entry) {
                Some(bytes) => allow.insert(bytes),
                None => tracing::warn!("Ignoring invalid pubkey in {env_var}"),
            }
        }
    }
    allow
}

fn build_secp256k1_allowlist(env_var: &str) -> SignerAllowlist {
    let mut allow = SignerAllowlist::new();
    if let Ok(list) = std::env::var(env_var) {
        for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match canonical_secp256k1_pubkey_from_hex(entry) {
                Some(bytes) => allow.insert(bytes),
                None => tracing::warn!("Ignoring invalid pubkey in {env_var}"),
            }
        }
    }
    allow
}

fn canonical_ed25519_pubkey_from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let bytes = decode_hex_fixed::<ED25519_PUBLIC_KEY_LEN>(hex_str, "public key").ok()?;
    let verifying_key = Ed25519VerifyingKey::from_bytes(&bytes).ok()?;
    Some(verifying_key.to_bytes().to_vec())
}

fn canonical_secp256k1_pubkey_from_hex(hex_str: &str) -> Option<Vec<u8>> {
    let bytes = visualsign::encodings::decode_hex(hex_str).ok()?;
    let encoded_point = EncodedPoint::from_bytes(&bytes).ok()?;
    let verifying_key = Secp256k1VerifyingKey::from_encoded_point(&encoded_point).ok()?;
    Some(verifying_key.to_encoded_point(false).as_bytes().to_vec())
}

/// Validate a token-metadata entry's signature over `value`'s raw bytes,
/// dispatching curve and allowlist by `origin_chain`.
/// `TokenOriginChain::Unspecified` is treated as `Near`.
fn validate_token_metadata_signature(
    asset_id: &str,
    value: &str,
    origin_chain: TokenOriginChain,
    signature: &SignatureMetadata,
    allowlists: &TokenMetadataSignerAllowlists,
) -> Result<(), TokenMetadataSignatureError> {
    match origin_chain {
        TokenOriginChain::Unspecified | TokenOriginChain::Near => validate_ed25519(
            asset_id,
            value,
            signature,
            &allowlists.near,
            visualsign::signing::near_token_metadata_prehash,
        ),
        TokenOriginChain::Ethereum => {
            validate_secp256k1(asset_id, value, signature, &allowlists.ethereum)
        }
        TokenOriginChain::Solana => validate_ed25519(
            asset_id,
            value,
            signature,
            &allowlists.solana,
            visualsign::signing::solana_token_metadata_prehash,
        ),
    }
}

fn validate_ed25519(
    asset_id: &str,
    value: &str,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
    prehash: fn(&str, &[u8]) -> [u8; 32],
) -> Result<(), TokenMetadataSignatureError> {
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing algorithm".to_string()))?;
    if algorithm != ED25519_ALGORITHM {
        return Err(TokenMetadataSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {ED25519_ALGORITHM} is supported for this origin chain."
        )));
    }
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing public_key".to_string()))?;

    let hash = prehash(asset_id, value.as_bytes());
    let sig_bytes = decode_hex_fixed::<ED25519_SIGNATURE_LEN>(&signature.value, "signature")?;
    let sig = Ed25519Signature::from_bytes(&sig_bytes);
    let pubkey_bytes = decode_hex_fixed::<ED25519_PUBLIC_KEY_LEN>(public_key_hex, "public key")?;
    let verifying_key = Ed25519VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| TokenMetadataSignatureError::Validation(format!("Invalid public key: {e}")))?;

    verifying_key.verify_strict(&hash, &sig).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    if !allowlist.contains(&verifying_key.to_bytes()) {
        return Err(TokenMetadataSignatureError::Validation(
            "signer not in allowlist".to_string(),
        ));
    }
    Ok(())
}

fn validate_secp256k1(
    asset_id: &str,
    value: &str,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<(), TokenMetadataSignatureError> {
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing algorithm".to_string()))?;
    if algorithm != SECP256K1_ALGORITHM {
        return Err(TokenMetadataSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {SECP256K1_ALGORITHM} is supported for this origin chain."
        )));
    }
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing public_key".to_string()))?;

    let hash = visualsign::signing::ethereum_token_metadata_prehash(asset_id, value.as_bytes());
    let sig_bytes = visualsign::encodings::decode_hex(&signature.value).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Invalid signature hex: {e}"))
    })?;
    let sig = Secp256k1Signature::from_der(&sig_bytes).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Invalid DER signature: {e}"))
    })?;
    let pubkey_bytes = visualsign::encodings::decode_hex(public_key_hex).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Invalid public key hex: {e}"))
    })?;
    let encoded_point = EncodedPoint::from_bytes(&pubkey_bytes).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Invalid public key point: {e}"))
    })?;
    let verifying_key = Secp256k1VerifyingKey::from_encoded_point(&encoded_point).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Invalid verifying key: {e}"))
    })?;

    verifying_key.verify_prehash(&hash, &sig).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    let signer_pubkey = verifying_key.to_encoded_point(false);
    if !allowlist.contains(signer_pubkey.as_bytes()) {
        return Err(TokenMetadataSignatureError::Validation(
            "signer not in allowlist".to_string(),
        ));
    }
    Ok(())
}

/// Extract and validate token-metadata entries from `ChainMetadata`, if
/// present.
///
/// Navigates `ChainMetadata -> Near -> token_mappings`. Returns `None` if the
/// metadata contains no NEAR token mappings (or no metadata at all), matching
/// the Ethereum ABI / Solana IDL extraction functions' convention so callers
/// can plug the result straight into a
/// [`visualsign::registry::LayeredRegistry`] request layer.
///
/// `trust_policy` gates only whether an entry with no signature at all is
/// accepted ([`MetadataTrustPolicy::accepts_unsigned`]); a present signature
/// is always checked against the relevant origin-chain allowlist in
/// `allowlists`, regardless of posture. Unlike the Ethereum ABI path (a
/// single allowlist), NEAR already dispatches identity checks per origin
/// chain, so the allowlist a `MetadataTrustPolicy::RequireAllowlistedSigner`
/// carries is not itself consulted here -- only the posture it selects is.
#[must_use]
pub fn try_extract_from_chain_metadata(
    chain_metadata: Option<&ChainMetadata>,
    allowlists: &TokenMetadataSignerAllowlists,
    trust_policy: &MetadataTrustPolicy,
) -> Option<NearTokenRegistry> {
    let chain_metadata = chain_metadata?;
    let chain_metadata::Metadata::Near(near) = chain_metadata.metadata.as_ref()? else {
        return None;
    };
    if near.token_mappings.is_empty() {
        return None;
    }

    let mut registry = NearTokenRegistry::default();
    let mut unsigned_count: usize = 0;
    for (asset_id, entry) in &near.token_mappings {
        if entry.value.len() > MAX_TOKEN_METADATA_VALUE_BYTES {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': exceeds size limit ({} bytes > {MAX_TOKEN_METADATA_VALUE_BYTES})",
                entry.value.len()
            );
            continue;
        }

        // An unrecognized discriminant and an omitted field both land on
        // Unspecified (NEAR's ed25519 curve), so each is logged before it gets
        // there. Otherwise a wrong-curve entry fails later as "Unsupported
        // algorithm", which points at the algorithm rather than at the
        // origin_chain that was silently substituted.
        let origin_chain = match entry.origin_chain {
            Some(v) => TokenOriginChain::try_from(v).unwrap_or_else(|_| {
                tracing::warn!(
                    "Token metadata for '{asset_id}': unrecognized origin_chain {v}, treating as \
                     Unspecified (NEAR ed25519); a signature for another chain's curve will not \
                     verify"
                );
                TokenOriginChain::Unspecified
            }),
            None => {
                if entry.signature.is_some() {
                    tracing::debug!(
                        "Token metadata for '{asset_id}': signed entry omits origin_chain, \
                         defaulting to NEAR ed25519; a secp256k1 signature needs origin_chain set \
                         explicitly"
                    );
                }
                TokenOriginChain::Unspecified
            }
        };

        // Signatures aren't required to register an entry (not every caller
        // signs yet), but one that IS present must validate: a present-but-
        // invalid signature signals tampering rather than simply an unsigned
        // source, so it is rejected outright rather than downgraded.
        let is_unsigned = entry.signature.is_none();

        // Whether an unsigned entry is acceptable at all is fixed by the
        // deployment's posture, not by the request: under
        // RequireAllowlistedSigner, a missing signature is always a
        // rejection, regardless of the asset id.
        if is_unsigned && !trust_policy.accepts_unsigned() {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': this deployment requires signed entries"
            );
            continue;
        }

        // An unsigned entry may fill a gap for an asset the compiled-in table
        // doesn't cover, but must never override an already-curated one:
        // tokens::resolve checks this registry before SEEDS unconditionally,
        // so an unsigned override would let an unauthenticated caller shadow
        // verified data with no signature at all -- turning, e.g., 1 wNEAR
        // into 1000000 wNEAR by claiming the wrong decimals. A signed entry
        // from an allowlisted signer is still a trusted correction and may
        // override normally.
        if is_unsigned && tokens::is_seeded(asset_id) {
            tracing::warn!(
                "Skipping unsigned token metadata for '{asset_id}': would override a curated seed"
            );
            continue;
        }

        if let Some(proto_sig) = entry.signature.as_ref() {
            let signature = convert_proto_signature(proto_sig);
            if let Err(e) = validate_token_metadata_signature(
                asset_id,
                &entry.value,
                origin_chain,
                &signature,
                allowlists,
            ) {
                tracing::warn!("Skipping token metadata for '{asset_id}': {e}");
                continue;
            }
        }

        let parsed: TokenMetadataValue = match serde_json::from_str(&entry.value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Skipping token metadata for '{asset_id}': invalid value JSON: {e}");
                continue;
            }
        };
        if parsed.decimals > MAX_TOKEN_DECIMALS {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': decimals {} out of range",
                parsed.decimals
            );
            continue;
        }
        if parsed.symbol.is_empty() || parsed.symbol.len() > MAX_TOKEN_SYMBOL_LEN {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': symbol length {} out of range",
                parsed.symbol.len()
            );
            continue;
        }
        // The symbol is embedded verbatim in an amount's abbreviation and
        // fallback text, so its character content decides what a signer reads.
        // Restricting it to printable ASCII keeps a bidi override (U+202E), a
        // zero-width character or a control byte from reordering or hiding the
        // asset name on the signing screen. Rejected rather than filtered: a
        // symbol is short and operator-supplied, so silently rewriting it
        // would show an asset name nobody chose.
        if !parsed
            .symbol
            .chars()
            .all(|c| c.is_ascii_graphic() || c == ' ')
        {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': symbol contains characters outside \
                 printable ASCII"
            );
            continue;
        }

        registry.by_asset_id.insert(
            asset_id.clone(),
            TokenMeta {
                symbol: parsed.symbol,
                decimals: parsed.decimals,
                verified: !is_unsigned,
            },
        );
        if is_unsigned {
            unsigned_count += 1;
        }
    }
    if unsigned_count > 0 {
        tracing::warn!(
            "Accepted {unsigned_count} unsigned token metadata entr(y/ies): integrity/provenance \
             unverified -- each also carries an `unverified-token-metadata` diagnostic on its \
             rendered amount, so this log is a count, not the only place this surfaces"
        );
    }
    if registry.by_asset_id.is_empty() {
        return None;
    }
    Some(registry)
}

/// Deterministic 32-byte seeds used to sign token metadata in local dev
/// tooling and this module's own tests. Not production keys.
#[cfg(any(test, feature = "dev-signing"))]
pub const DEV_NEAR_SIGNING_KEY_SEED: [u8; 32] = [0x51u8; 32];
#[cfg(any(test, feature = "dev-signing"))]
pub const DEV_ETHEREUM_SIGNING_KEY_SEED: [u8; 32] = [0x52u8; 32];
#[cfg(any(test, feature = "dev-signing"))]
pub const DEV_SOLANA_SIGNING_KEY_SEED: [u8; 32] = [0x53u8; 32];

/// Sign `value` (the exact `TokenMetadataEntry.value` bytes) with an ed25519
/// seed (Near or Solana origin) and return a proto `SignatureMetadata` ready
/// to drop into `TokenMetadataEntry.signature`.
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_token_metadata_ed25519(
    asset_id: &str,
    value: &str,
    seed: &[u8; 32],
    prehash: fn(&str, &[u8]) -> [u8; 32],
) -> generated::parser::SignatureMetadata {
    use ed25519_dalek::{Signer, SigningKey};
    let signing_key = SigningKey::from_bytes(seed);
    let verifying_key = signing_key.verifying_key();
    let hash = prehash(asset_id, value.as_bytes());
    let signature = signing_key.sign(&hash);
    generated::parser::SignatureMetadata {
        value: hex::encode(signature.to_bytes()),
        metadata: vec![
            generated::parser::Metadata {
                key: "algorithm".to_string(),
                value: ED25519_ALGORITHM.to_string(),
            },
            generated::parser::Metadata {
                key: "public_key".to_string(),
                value: hex::encode(verifying_key.to_bytes()),
            },
        ],
    }
}

/// Sign `value` (the exact `TokenMetadataEntry.value` bytes) with a secp256k1
/// seed (Ethereum origin) and return a proto `SignatureMetadata` ready to drop
/// into `TokenMetadataEntry.signature`.
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_token_metadata_secp256k1(
    asset_id: &str,
    value: &str,
    seed: &[u8; 32],
) -> Result<generated::parser::SignatureMetadata, String> {
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    let signing_key = Secp256k1SigningKey::from_bytes(seed.into())
        .map_err(|e| format!("invalid secp256k1 signing key seed: {e}"))?;
    let verifying_key = Secp256k1VerifyingKey::from(&signing_key);
    let hash = visualsign::signing::ethereum_token_metadata_prehash(asset_id, value.as_bytes());
    let signature: Secp256k1Signature = signing_key
        .sign_prehash(&hash)
        .map_err(|e| format!("failed to sign token metadata hash: {e}"))?;
    Ok(generated::parser::SignatureMetadata {
        value: hex::encode(signature.to_der().as_bytes()),
        metadata: vec![
            generated::parser::Metadata {
                key: "algorithm".to_string(),
                value: SECP256K1_ALGORITHM.to_string(),
            },
            generated::parser::Metadata {
                key: "public_key".to_string(),
                value: hex::encode(verifying_key.to_encoded_point(false).as_bytes()),
            },
        ],
    })
}

/// CLI entry point for token-metadata signing, decoupled from the
/// `dev-signing` cargo feature so the `cli_plugin` module compiles regardless
/// of whether `dev-signing` is enabled.
///
/// `cli_plugin` is a default feature of `visualsign-near` (like every other
/// chain crate's), so it is compiled into every consumer -- including the
/// production `parser_app`, which enables `cli-plugin` transitively but NOT
/// `dev-signing`. The underlying [`sign_token_metadata_ed25519`] and
/// [`DEV_NEAR_SIGNING_KEY_SEED`] are `dev-signing`-gated, so calling them
/// directly from `cli_plugin` breaks any `cli-plugin`-without-`dev-signing`
/// build. This wrapper is always present: under `dev-signing` (or tests) it
/// signs with the dev key; otherwise it returns an error.
///
/// Always signs as NEAR-origin (ed25519, the NEAR-only curator dev key): the
/// CLI has no per-mapping `origin_chain` input yet, so every CLI-supplied
/// entry defaults to the origin the verifier itself treats as the default
/// (`Unspecified`/`Near`). Ethereum/Solana-origin CLI signing is not wired up
/// yet.
///
/// # Errors
/// Returns `Err` if the binary was built without `dev-signing` (in which case
/// token-metadata signing is unavailable by design).
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_token_metadata_for_cli(
    asset_id: &str,
    value: &str,
) -> Result<generated::parser::SignatureMetadata, String> {
    Ok(sign_token_metadata_ed25519(
        asset_id,
        value,
        &DEV_NEAR_SIGNING_KEY_SEED,
        visualsign::signing::near_token_metadata_prehash,
    ))
}

/// See the `dev-signing`-enabled variant above. Without `dev-signing` the dev
/// key is not linked, so token-metadata signing is unavailable and this
/// returns an error.
#[cfg(not(any(test, feature = "dev-signing")))]
pub fn sign_token_metadata_for_cli(
    _asset_id: &str,
    _value: &str,
) -> Result<generated::parser::SignatureMetadata, String> {
    Err(
        "token metadata signing is unavailable: this binary was built without the \
         dev-signing feature"
            .to_string(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{NearMetadata, TokenMetadataEntry};

    const ASSET_ID: &str = "nep141:a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near";
    const VALUE: &str = r#"{"symbol":"USDC.e","decimals":6}"#;
    /// Deliberately NOT in `tokens::SEEDS`. Tests exercising an unsigned
    /// entry's own validation (JSON shape, decimals bound) use this instead
    /// of `ASSET_ID` so the gap-fill-only guard doesn't intercept first and
    /// mask what they're actually testing.
    const UNSEEDED_ASSET_ID: &str = "nep141:new-unlisted-token.near";

    fn accept_unsigned_policy() -> MetadataTrustPolicy {
        MetadataTrustPolicy::AcceptUnsigned
    }

    fn require_signed_policy() -> MetadataTrustPolicy {
        MetadataTrustPolicy::RequireAllowlistedSigner(SignerAllowlist::new())
    }

    fn near_allowlist() -> TokenMetadataSignerAllowlists {
        let mut near = SignerAllowlist::new();
        near.insert(
            ed25519_dalek::SigningKey::from_bytes(&DEV_NEAR_SIGNING_KEY_SEED)
                .verifying_key()
                .to_bytes()
                .to_vec(),
        );
        TokenMetadataSignerAllowlists {
            near,
            ethereum: SignerAllowlist::new(),
            solana: SignerAllowlist::new(),
        }
    }

    fn ethereum_allowlist() -> TokenMetadataSignerAllowlists {
        let signing_key =
            Secp256k1SigningKey::from_bytes((&DEV_ETHEREUM_SIGNING_KEY_SEED).into()).unwrap();
        let verifying_key = Secp256k1VerifyingKey::from(&signing_key);
        let mut ethereum = SignerAllowlist::new();
        ethereum.insert(verifying_key.to_encoded_point(false).as_bytes().to_vec());
        TokenMetadataSignerAllowlists {
            near: SignerAllowlist::new(),
            ethereum,
            solana: SignerAllowlist::new(),
        }
    }

    fn solana_allowlist() -> TokenMetadataSignerAllowlists {
        let mut solana = SignerAllowlist::new();
        solana.insert(
            ed25519_dalek::SigningKey::from_bytes(&DEV_SOLANA_SIGNING_KEY_SEED)
                .verifying_key()
                .to_bytes()
                .to_vec(),
        );
        TokenMetadataSignerAllowlists {
            near: SignerAllowlist::new(),
            ethereum: SignerAllowlist::new(),
            solana,
        }
    }

    fn empty_allowlists() -> TokenMetadataSignerAllowlists {
        TokenMetadataSignerAllowlists {
            near: SignerAllowlist::new(),
            ethereum: SignerAllowlist::new(),
            solana: SignerAllowlist::new(),
        }
    }

    #[test]
    fn near_origin_valid_signature_verifies() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Near,
                &sig,
                &near_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn unspecified_origin_defaults_to_near_curve() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Unspecified,
                &sig,
                &near_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn ethereum_origin_valid_signature_verifies() {
        let sig_meta =
            sign_token_metadata_secp256k1(ASSET_ID, VALUE, &DEV_ETHEREUM_SIGNING_KEY_SEED).unwrap();
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Ethereum,
                &sig,
                &ethereum_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn solana_origin_valid_signature_verifies() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_SOLANA_SIGNING_KEY_SEED,
            visualsign::signing::solana_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Solana,
                &sig,
                &solana_allowlist()
            )
            .is_ok()
        );
    }

    /// A Near-origin signature must not verify under a different origin
    /// chain's tag.
    #[test]
    fn signature_does_not_cross_origin_chains() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Solana,
                &sig,
                &near_allowlist()
            )
            .is_err(),
            "a Near-tagged signature must not verify under the Solana tag"
        );
    }

    #[test]
    fn tampered_value_rejected() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        let tampered = r#"{"symbol":"PHISH","decimals":6}"#;
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                tampered,
                TokenOriginChain::Near,
                &sig,
                &near_allowlist()
            )
            .is_err()
        );
    }

    /// A signature is valid only for the exact asset id it was produced for.
    #[test]
    fn signature_bound_to_asset_id_rejects_replay() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                "nep141:a-different-token.near",
                VALUE,
                TokenOriginChain::Near,
                &sig,
                &near_allowlist()
            )
            .is_err()
        );
    }

    #[test]
    fn unlisted_signer_rejected() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        let result = validate_token_metadata_signature(
            ASSET_ID,
            VALUE,
            TokenOriginChain::Near,
            &sig,
            &empty_allowlists(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in allowlist"));
    }

    fn make_mappings(
        entries: Vec<(&str, TokenMetadataEntry)>,
    ) -> std::collections::BTreeMap<String, TokenMetadataEntry> {
        entries
            .into_iter()
            .map(|(id, entry)| (id.to_string(), entry))
            .collect()
    }

    #[test]
    fn extract_no_metadata_is_none() {
        assert!(
            try_extract_from_chain_metadata(None, &near_allowlist(), &accept_unsigned_policy())
                .is_none()
        );
    }

    #[test]
    fn extract_non_near_metadata_is_none() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(
                generated::parser::EthereumMetadata {
                    network_id: None,
                    abi_mappings: Default::default(),
                },
            )),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_unsigned_entry_accepted() {
        // Not seeded: unsigned entries fill gaps for assets SEEDS doesn't
        // cover. See extract_unsigned_entry_for_seeded_asset_rejected for the
        // (disallowed) other case.
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            &near_allowlist(),
            &accept_unsigned_policy(),
        )
        .expect("unsigned entry must still be registered");
        let meta = registry
            .by_asset_id
            .get(UNSEEDED_ASSET_ID)
            .expect("present");
        assert_eq!(meta.symbol, "USDC.e");
        assert_eq!(meta.decimals, 6);
        assert!(
            !meta.verified,
            "an unsigned gap-fill entry must not be marked verified"
        );
    }

    // Regression coverage for the shadowing finding: an unsigned entry must
    // not override a curated SEEDS value, since tokens::resolve checks this
    // registry before SEEDS unconditionally -- an unsigned override would let
    // an unauthenticated caller turn 1 wNEAR into 1000000 wNEAR by claiming
    // the wrong decimals, with no signature required at all.
    #[test]
    fn extract_unsigned_entry_for_seeded_asset_rejected() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID, // seeded as USDC.e/6 in tokens::SEEDS
                    TokenMetadataEntry {
                        value: r#"{"symbol":"USDC.e","decimals":30}"#.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    // Regression coverage for adopting MetadataTrustPolicy: an unsigned entry
    // for an asset SEEDS doesn't cover -- normally accepted (see
    // extract_unsigned_entry_accepted) -- must be rejected outright once the
    // deployment's posture is RequireAllowlistedSigner, regardless of asset id.
    #[test]
    fn extract_unsigned_entry_rejected_under_require_signed_policy() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &require_signed_policy()
            )
            .is_none()
        );
    }

    // A validly signed, allowlisted entry still registers under the strict
    // posture: MetadataTrustPolicy gates only whether a MISSING signature is
    // acceptable, not the per-origin-chain allowlist check a present one
    // already goes through unconditionally.
    #[test]
    fn extract_signed_entry_still_registers_under_require_signed_policy() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: Some(TokenOriginChain::Near as i32),
                    },
                )]),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            &near_allowlist(),
            &require_signed_policy(),
        )
        .expect("signed entry must still be registered under the strict posture");
        let meta = registry.by_asset_id.get(ASSET_ID).expect("present");
        assert_eq!(meta.symbol, "USDC.e");
        assert!(
            meta.verified,
            "a signed, allowlisted entry must be marked verified"
        );
    }

    #[test]
    fn extract_rejects_a_symbol_with_a_bidi_override() {
        // U+202E RIGHT-TO-LEFT OVERRIDE would reorder the asset name where the
        // symbol is rendered, so the entry is refused rather than displayed.
        let value = format!("{{\"symbol\":\"BTC{}\",\"decimals\":8}}", '\u{202E}');
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    "nep141:spoofed.near",
                    TokenMetadataEntry {
                        value: value.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none(),
            "a symbol carrying a bidi override must not register"
        );
    }

    #[test]
    fn extract_treats_an_unrecognized_origin_chain_as_unspecified() {
        // An out-of-range discriminant falls back to Unspecified (NEAR's
        // ed25519 curve), so an ed25519 signature over the value still
        // validates and the entry registers.
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: Some(9999),
                    },
                )]),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            &near_allowlist(),
            &require_signed_policy(),
        )
        .expect("an unrecognized origin_chain falls back to the NEAR curve");
        assert_eq!(
            registry.by_asset_id.get(ASSET_ID).expect("present").symbol,
            "USDC.e"
        );
    }

    #[test]
    fn extract_signed_entry_verifies_and_registers() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: Some(TokenOriginChain::Near as i32),
                    },
                )]),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            &near_allowlist(),
            &accept_unsigned_policy(),
        )
        .expect("signed entry must verify and register");
        assert!(registry.by_asset_id.contains_key(ASSET_ID));
    }

    /// An entry that carries a signature that fails validation (unauthorized
    /// signer) must be rejected outright, not silently downgraded to unsigned.
    #[test]
    fn extract_rejects_entry_with_unauthorized_signature() {
        let sig_meta = sign_token_metadata_ed25519(
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: Some(TokenOriginChain::Near as i32),
                    },
                )]),
            })),
        };
        // No allowlist authorizes the dev seed used above.
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &empty_allowlists(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_invalid_value_json_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: "not valid json".to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_decimals_out_of_range_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: r#"{"symbol":"BROKEN","decimals":999}"#.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    // Regression coverage for a remote panic: 999 above is caught by u8
    // deserialization failing outright, but 39 is a perfectly valid u8 that
    // still overflows `10u128.pow(decimals)` in `tokens::format_units` --
    // reachable without a signature, from an unauthenticated request field.
    #[test]
    fn extract_decimals_within_u8_but_over_format_bound_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: r#"{"symbol":"BROKEN","decimals":39}"#.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_empty_symbol_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: r#"{"symbol":"","decimals":6}"#.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_oversized_symbol_skipped() {
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: format!(
                            r#"{{"symbol":"{}","decimals":6}}"#,
                            "A".repeat(MAX_TOKEN_SYMBOL_LEN + 1)
                        ),
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    #[test]
    fn extract_oversized_value_skipped() {
        let oversized = format!(
            r#"{{"symbol":"{}","decimals":6}}"#,
            "A".repeat(MAX_TOKEN_METADATA_VALUE_BYTES)
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: oversized,
                        signature: None,
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    /// A CLI-signed entry must verify under `authorized_token_metadata_signers`
    /// itself, the allowlist the decode path consults.
    ///
    /// The test module's own `near_allowlist()` helper enrolls the dev key
    /// directly, so asserting against it proves only that the signature is
    /// well-formed. Without the dev-signing carve-out in the real function, a
    /// CLI-signed entry signs fine and is then dropped as an untrusted signer
    /// when the CLI decodes it.
    #[test]
    fn authorized_token_metadata_signers_allowlists_the_cli_dev_key() {
        let sig = sign_token_metadata_for_cli(ASSET_ID, VALUE).expect("cli signing must succeed");
        let local_sig = convert_proto_signature(&sig);
        assert!(
            validate_token_metadata_signature(
                ASSET_ID,
                VALUE,
                TokenOriginChain::Near,
                &local_sig,
                authorized_token_metadata_signers()
            )
            .is_ok(),
            "a CLI-signed entry must verify under the allowlist the decode path consults"
        );
    }
}
