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
//! Each origin is a [`TokenMetadataDomain`], which names one domain-separated
//! prehash and one set of curator keys. The two are the same value, so a
//! signature can only ever be checked against the keys enrolled for the domain
//! it was signed under.
//!
//! Curator keys live in a single [`SignerAllowlist`], with each entry bound to
//! its domain by [`scoped_signer_key`]. The same physical key enrolled for two
//! domains is two distinct entries, so revoking "trusted to vouch for
//! Ethereum-origin assets" does not silently also revoke (or, worse, leave
//! standing) "trusted to vouch for NEAR-native ones". The Ethereum ABI and
//! Solana IDL paths keep their own allowlists and are untouched by this.
//!
//! A signature that fails to verify is rejected outright: it means the bytes or
//! the key were altered. A signature that verifies under a key this deployment
//! has not enrolled is not tampering -- it is an assertion from a party the
//! parser cannot attribute, so it is worth what an unsigned entry is worth and
//! is treated identically. Either may still be accepted (not every caller signs
//! yet), but only when the deployment's [`MetadataTrustPolicy`] allows it and
//! the asset isn't already covered by the compiled-in `tokens::SEEDS` table --
//! unattributable metadata fills a gap, it never overrides a curated value.

use std::sync::OnceLock;

use ed25519_dalek::{Signature as Ed25519Signature, VerifyingKey as Ed25519VerifyingKey};
#[cfg(any(test, feature = "dev-signing"))]
use ed25519_dalek::{Signer as Ed25519Signer, SigningKey as Ed25519SigningKey};
use generated::parser::{ChainMetadata, TokenOriginChain, chain_metadata};
use k256::EncodedPoint;
#[cfg(any(test, feature = "dev-signing"))]
use k256::ecdsa::SigningKey as Secp256k1SigningKey;
#[cfg(any(test, feature = "dev-signing"))]
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature as Secp256k1Signature, VerifyingKey as Secp256k1VerifyingKey};
use serde::Deserialize;
use visualsign::signing::{MetadataTrustPolicy, SignerAllowlist};

use super::{NearTokenRegistry, TokenMeta, tokens};
use crate::networks::NearNetwork;

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

/// Maximum number of entries accepted in one request's `token_mappings`. Each
/// entry that survives the cheap structural checks costs an elliptic-curve
/// verification, so the map's length is bounded before any of that work
/// starts. A request carrying more entries than this is rejected whole rather
/// than truncated: dropping the tail would silently ignore caller data that
/// the caller has no way to see was discarded.
const MAX_TOKEN_METADATA_ENTRIES: usize = 256;

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
    visualsign::encodings::decode_hex_array::<N>(value)
        .map_err(|e| TokenMetadataSignatureError::Validation(format!("Invalid {what} {e}")))
}

/// The signing domain a token-metadata entry is authorized under: which curve
/// verifies it, which prehash it is bound to, and which curator keys may sign
/// it. Selected per entry from [`TokenOriginChain`].
///
/// Every fact about a domain is derived here, from one exhaustive match each.
/// Adding an origin chain therefore has to name its tag, its algorithm, its
/// prehash and its env var, rather than only adding a dispatch arm that could
/// silently reuse another chain's curator identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenMetadataDomain {
    /// NEAR-native assets, and the default for an omitted `origin_chain`.
    Near,
    /// Assets bridged from Ethereum and its EVM twins.
    Ethereum,
    /// Assets bridged from Solana and its SVM twin.
    Solana,
}

impl TokenMetadataDomain {
    /// `TokenOriginChain::Unspecified` resolves to [`Self::Near`]; every other
    /// variant maps to its own domain.
    #[must_use]
    fn from_origin_chain(origin_chain: TokenOriginChain) -> Self {
        match origin_chain {
            TokenOriginChain::Unspecified | TokenOriginChain::Near => Self::Near,
            TokenOriginChain::Ethereum => Self::Ethereum,
            TokenOriginChain::Solana => Self::Solana,
        }
    }

    /// The domain-separation tag, shared by the prehash this domain signs under
    /// and the allowlist scope its curator keys are enrolled in. One value for
    /// both is what keeps them from diverging.
    #[must_use]
    fn tag(self) -> &'static str {
        match self {
            Self::Near => visualsign::signing::CHAIN_TAG_NEAR_TOKEN_METADATA,
            Self::Ethereum => visualsign::signing::CHAIN_TAG_ETHEREUM_TOKEN_METADATA,
            Self::Solana => visualsign::signing::CHAIN_TAG_SOLANA_TOKEN_METADATA,
        }
    }

    /// The only signature algorithm this domain accepts.
    #[must_use]
    fn algorithm(self) -> &'static str {
        match self {
            Self::Near | Self::Solana => ED25519_ALGORITHM,
            Self::Ethereum => SECP256K1_ALGORITHM,
        }
    }

    /// The 32-byte digest a signature for this domain is produced over.
    #[must_use]
    fn prehash(self, network_id: &str, asset_id: &str, body: &[u8]) -> [u8; 32] {
        match self {
            Self::Near => {
                visualsign::signing::near_token_metadata_prehash(network_id, asset_id, body)
            }
            Self::Ethereum => {
                visualsign::signing::ethereum_token_metadata_prehash(network_id, asset_id, body)
            }
            Self::Solana => {
                visualsign::signing::solana_token_metadata_prehash(network_id, asset_id, body)
            }
        }
    }

    /// The env var naming this domain's authorized curator keys.
    #[must_use]
    fn env_var(self) -> &'static str {
        match self {
            Self::Near => "VISUALSIGN_NEAR_TOKEN_SIGNERS",
            Self::Ethereum => "VISUALSIGN_ETH_TOKEN_SIGNERS",
            Self::Solana => "VISUALSIGN_SOL_TOKEN_SIGNERS",
        }
    }

    /// Every domain, for construction and for tests asserting each is distinct.
    const ALL: [Self; 3] = [Self::Near, Self::Ethereum, Self::Solana];
}

/// Whether a signature that verified belongs to a curator this deployment
/// recognizes for the entry's domain.
///
/// Kept distinct from the error case on purpose. A signature that fails to
/// verify means the bytes or the key were altered, and the entry is refused. A
/// signature that verifies under a key nobody enrolled is not tampering -- it
/// is an assertion by an unrecognized party, which is exactly as trustworthy as
/// an unsigned entry and is treated as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignerIdentity {
    /// Signed by a key enrolled for this domain.
    Recognized,
    /// The signature verifies, but the key is not enrolled for this domain.
    Unrecognized,
}

impl SignerIdentity {
    fn of(
        allowlist: &SignerAllowlist,
        domain: TokenMetadataDomain,
        canonical_pubkey: &[u8],
    ) -> Self {
        if allowlist.contains(&scoped_signer_key(domain, canonical_pubkey)) {
            Self::Recognized
        } else {
            Self::Unrecognized
        }
    }
}

/// Bind a curator key to the domain it is authorized for, producing the bytes
/// a [`SignerAllowlist`] entry is keyed under.
///
/// The shared allowlist stores opaque bytes and compares them, so scoping is
/// ours to encode. Both halves are length-prefixed, exactly as
/// [`visualsign::signing::metadata_signing_prehash_v1`] prefixes its own fields
/// and for the same reason: plain concatenation is not injective while the tag
/// is variable-length text, so `("near", K)` could collide with
/// `("nea", "r" || K)`.
///
/// The same physical key enrolled for two domains is two distinct entries, so
/// revoking one leaves the other standing.
fn scoped_signer_key(domain: TokenMetadataDomain, canonical_pubkey: &[u8]) -> Vec<u8> {
    let tag = domain.tag().as_bytes();
    let mut scoped = Vec::with_capacity(16 + tag.len() + canonical_pubkey.len());
    for field in [tag, canonical_pubkey] {
        scoped.extend_from_slice(&(field.len() as u64).to_le_bytes());
        scoped.extend_from_slice(field);
    }
    scoped
}

/// Authorized token-metadata curator keys, built once and cached, as a single
/// allowlist whose entries are scoped by [`TokenMetadataDomain`].
///
/// - `VISUALSIGN_NEAR_TOKEN_SIGNERS`: comma-separated hex ed25519 public keys.
/// - `VISUALSIGN_ETH_TOKEN_SIGNERS`: comma-separated hex secp256k1 public keys
///   (any SEC1 encoding).
/// - `VISUALSIGN_SOL_TOKEN_SIGNERS`: comma-separated hex ed25519 public keys.
///
/// Each var populates only its own domain, so a key trusted to vouch for
/// Ethereum-origin assets does not thereby vouch for NEAR-native ones. An unset
/// (or entirely invalid) var leaves that domain unpopulated, and a signature
/// under it is then never recognized (fail-closed).
///
/// This list decides whether a present signature is *recognized*, not whether an
/// unrecognized entry is *accepted* -- that is the deployment's
/// [`MetadataTrustPolicy`], together with the gap-fill-only rule in
/// [`try_extract_from_chain_metadata`].
#[must_use]
pub fn authorized_token_metadata_signers() -> &'static SignerAllowlist {
    static ALLOW: OnceLock<SignerAllowlist> = OnceLock::new();
    ALLOW.get_or_init(|| {
        let mut allow = SignerAllowlist::new();
        for domain in TokenMetadataDomain::ALL {
            insert_env_signers(&mut allow, domain);
        }
        allow
    })
}

/// Enrol one domain's env-configured keys, canonicalized for its curve.
fn insert_env_signers(allow: &mut SignerAllowlist, domain: TokenMetadataDomain) {
    let env_var = domain.env_var();
    let Ok(list) = std::env::var(env_var) else {
        return;
    };
    for entry in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let canonical = match domain.algorithm() {
            SECP256K1_ALGORITHM => canonical_secp256k1_pubkey_from_hex(entry),
            _ => canonical_ed25519_pubkey_from_hex(entry),
        };
        match canonical {
            Some(bytes) => allow.insert(scoped_signer_key(domain, &bytes)),
            None => tracing::warn!("Ignoring invalid pubkey in {env_var}"),
        }
    }
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
///
/// `network_id` is the NEAR network the metadata applies to. It is part of the
/// signed scope, so a signature minted for one network does not verify on
/// another.
fn validate_token_metadata_signature(
    network_id: &str,
    asset_id: &str,
    value: &str,
    domain: TokenMetadataDomain,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<SignerIdentity, TokenMetadataSignatureError> {
    match domain {
        TokenMetadataDomain::Near | TokenMetadataDomain::Solana => {
            validate_ed25519(network_id, asset_id, value, domain, signature, allowlist)
        }
        TokenMetadataDomain::Ethereum => {
            validate_secp256k1(network_id, asset_id, value, domain, signature, allowlist)
        }
    }
}

fn validate_ed25519(
    network_id: &str,
    asset_id: &str,
    value: &str,
    domain: TokenMetadataDomain,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<SignerIdentity, TokenMetadataSignatureError> {
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing algorithm".to_string()))?;
    if algorithm != domain.algorithm() {
        return Err(TokenMetadataSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {} is supported for this origin chain.",
            domain.algorithm()
        )));
    }
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing public_key".to_string()))?;

    let hash = domain.prehash(network_id, asset_id, value.as_bytes());
    let sig_bytes = decode_hex_fixed::<ED25519_SIGNATURE_LEN>(&signature.value, "signature")?;
    let sig = Ed25519Signature::from_bytes(&sig_bytes);
    let pubkey_bytes = decode_hex_fixed::<ED25519_PUBLIC_KEY_LEN>(public_key_hex, "public key")?;
    let verifying_key = Ed25519VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| TokenMetadataSignatureError::Validation(format!("Invalid public key: {e}")))?;

    verifying_key.verify_strict(&hash, &sig).map_err(|e| {
        TokenMetadataSignatureError::Validation(format!("Signature verification failed: {e}"))
    })?;

    Ok(SignerIdentity::of(
        allowlist,
        domain,
        &verifying_key.to_bytes(),
    ))
}

fn validate_secp256k1(
    network_id: &str,
    asset_id: &str,
    value: &str,
    domain: TokenMetadataDomain,
    signature: &SignatureMetadata,
    allowlist: &SignerAllowlist,
) -> Result<SignerIdentity, TokenMetadataSignatureError> {
    let algorithm = signature
        .algorithm
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing algorithm".to_string()))?;
    if algorithm != domain.algorithm() {
        return Err(TokenMetadataSignatureError::Validation(format!(
            "Unsupported algorithm: {algorithm}. Only {} is supported for this origin chain.",
            domain.algorithm()
        )));
    }
    let public_key_hex = signature
        .public_key
        .as_deref()
        .ok_or_else(|| TokenMetadataSignatureError::Validation("Missing public_key".to_string()))?;

    let hash = domain.prehash(network_id, asset_id, value.as_bytes());
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
    Ok(SignerIdentity::of(
        allowlist,
        domain,
        signer_pubkey.as_bytes(),
    ))
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
/// `network` is the NEAR network this request resolved to, taken as a parameter
/// rather than re-derived from `chain_metadata`: the caller already resolves it
/// (a request that omits `network_id` falls back to the network the converter
/// was built for), and it is part of every signed scope, so deriving it a
/// second time here could scope a signature check to a different network than
/// the one being rendered.
///
/// `trust_policy` gates only whether an entry with no signature at all is
/// `allowlist` is what decides whether a present signature is attributable to
/// a curator; `trust_policy` decides only whether an entry that is not
/// attributable may be used at all. Identity is checked under either posture,
/// because it labels the entry rather than gating it -- an unrecognized signer
/// lands exactly where an unsigned entry does.
#[must_use]
pub fn try_extract_from_chain_metadata(
    chain_metadata: Option<&ChainMetadata>,
    network: NearNetwork,
    allowlist: &SignerAllowlist,
    trust_policy: &MetadataTrustPolicy,
) -> Option<NearTokenRegistry> {
    let chain_metadata = chain_metadata?;
    let chain_metadata::Metadata::Near(near) = chain_metadata.metadata.as_ref()? else {
        return None;
    };
    if near.token_mappings.is_empty() {
        return None;
    }
    // The canonical id, not whatever spelling the request used, so a signature
    // does not depend on the casing a caller happened to send.
    let network_id = network.network_id();

    if near.token_mappings.len() > MAX_TOKEN_METADATA_ENTRIES {
        tracing::warn!(
            "Ignoring all NEAR token metadata: {} entries exceeds the limit of \
             {MAX_TOKEN_METADATA_ENTRIES}",
            near.token_mappings.len()
        );
        return None;
    }

    let mut registry = NearTokenRegistry::default();
    let mut unverified_count: usize = 0;
    for (asset_id, entry) in &near.token_mappings {
        if entry.value.len() > MAX_TOKEN_METADATA_VALUE_BYTES {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': exceeds size limit ({} bytes > {MAX_TOKEN_METADATA_VALUE_BYTES})",
                entry.value.len()
            );
            continue;
        }

        // Only an omitted field and an explicit Unspecified default to NEAR's
        // ed25519 curve and curator allowlist. An unrecognized discriminant
        // names an origin this build cannot verify, so it is rejected rather
        // than substituted: checking it under the NEAR curator would mark
        // metadata claiming an unsupported origin as verified.
        let origin_chain = match entry.origin_chain {
            Some(v) => match TokenOriginChain::try_from(v) {
                Ok(chain) => chain,
                Err(_) => {
                    tracing::warn!(
                        "Skipping token metadata for '{asset_id}': unrecognized origin_chain {v}"
                    );
                    continue;
                }
            },
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
        let domain = TokenMetadataDomain::from_origin_chain(origin_chain);

        // Signatures aren't required to register an entry (not every caller
        // signs yet); what an absent one costs the entry is decided below.
        let is_unsigned = entry.signature.is_none();

        // Whether an entry the parser cannot attribute to a curator is
        // acceptable at all is fixed by the deployment's posture, not by the
        // request: under RequireAllowlistedSigner a missing signature is always
        // a rejection, regardless of the asset id.
        if is_unsigned && !trust_policy.accepts_unsigned() {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': this deployment requires signed entries"
            );
            continue;
        }

        // Unattributable metadata may fill a gap for an asset the compiled-in
        // table doesn't cover, but must never override an already-curated one:
        // tokens::resolve checks this registry before SEEDS unconditionally, so
        // allowing it would let an unauthenticated caller shadow verified data
        // -- turning, e.g., 1 wNEAR into 1000000 wNEAR by claiming the wrong
        // decimals. Checked cheaply here for an entry carrying no signature at
        // all; one whose signature proves to be from a key this deployment does
        // not recognize is caught after verification, below.
        if is_unsigned && tokens::is_seeded(asset_id) {
            tracing::warn!(
                "Skipping unsigned token metadata for '{asset_id}': would override a curated seed"
            );
            continue;
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

        // A present signature must verify: one that does not match its own
        // bytes and key signals tampering, so the entry is refused outright.
        //
        // A signature that verifies under a key this deployment has not
        // enrolled is a different fact. It is not tampering -- it is an
        // assertion from a party the parser cannot attribute, worth exactly
        // what an unsigned entry is worth and treated the same. So an attacker
        // gains nothing by signing with a key of their own over omitting the
        // signature, which is what makes checking identity here safe to do
        // under either posture.
        //
        // This runs last, once the entry is known to be structurally sound. An
        // elliptic-curve verification is the most expensive step here by orders
        // of magnitude, and every check above it is a length comparison or a
        // parse of an already size-capped string, so an entry that a cheap
        // check would reject anyway never costs one.
        let verified = match entry.signature.as_ref() {
            None => false,
            Some(proto_sig) => {
                let signature = convert_proto_signature(proto_sig);
                match validate_token_metadata_signature(
                    network_id,
                    asset_id,
                    &entry.value,
                    domain,
                    &signature,
                    allowlist,
                ) {
                    Err(e) => {
                        tracing::warn!("Skipping token metadata for '{asset_id}': {e}");
                        continue;
                    }
                    Ok(SignerIdentity::Recognized) => true,
                    Ok(SignerIdentity::Unrecognized) => {
                        if !trust_policy.accepts_unsigned() {
                            tracing::warn!(
                                "Skipping token metadata for '{asset_id}': signer is not an \
                                 authorized curator for this origin chain"
                            );
                            continue;
                        }
                        tracing::warn!(
                            "Token metadata for '{asset_id}': signed by a key this deployment \
                             does not recognize; accepted as unverified"
                        );
                        false
                    }
                }
            }
        };

        // The gap-fill rule again, now that identity is settled: a signature
        // from an unrecognized key must not buy an override the same caller
        // could not have had by omitting it.
        if !verified && tokens::is_seeded(asset_id) {
            tracing::warn!(
                "Skipping token metadata for '{asset_id}': unverified metadata would override a \
                 curated seed"
            );
            continue;
        }

        registry.by_asset_id.insert(
            asset_id.clone(),
            TokenMeta {
                symbol: parsed.symbol,
                decimals: parsed.decimals,
                verified,
            },
        );
        if !verified {
            unverified_count += 1;
        }
    }
    if unverified_count > 0 {
        tracing::warn!(
            "Accepted {unverified_count} unverified token metadata entr(y/ies): provenance \
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
/// to drop into `TokenMetadataEntry.signature`. `network_id` must be the NEAR
/// network the entry is destined for; it is part of the signed scope.
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_token_metadata_ed25519(
    network_id: &str,
    asset_id: &str,
    value: &str,
    seed: &[u8; 32],
    prehash: fn(&str, &str, &[u8]) -> [u8; 32],
) -> generated::parser::SignatureMetadata {
    let signing_key = Ed25519SigningKey::from_bytes(seed);
    let verifying_key = signing_key.verifying_key();
    let hash = prehash(network_id, asset_id, value.as_bytes());
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
/// into `TokenMetadataEntry.signature`. `network_id` must be the NEAR network
/// the entry is destined for; it is part of the signed scope.
#[cfg(any(test, feature = "dev-signing"))]
pub fn sign_token_metadata_secp256k1(
    network_id: &str,
    asset_id: &str,
    value: &str,
    seed: &[u8; 32],
) -> Result<generated::parser::SignatureMetadata, String> {
    let signing_key = Secp256k1SigningKey::from_bytes(seed.into())
        .map_err(|e| format!("invalid secp256k1 signing key seed: {e}"))?;
    let verifying_key = Secp256k1VerifyingKey::from(&signing_key);
    let hash = visualsign::signing::ethereum_token_metadata_prehash(
        network_id,
        asset_id,
        value.as_bytes(),
    );
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
    /// The network every fixture below signs and verifies against. It is part
    /// of the signed scope, so signing and verifying must agree on it. The
    /// string and the enum are the same network; `network_ids_agree` gates that.
    const NETWORK_ID: &str = "NEAR_MAINNET";
    const NETWORK: NearNetwork = NearNetwork::Mainnet;

    #[test]
    fn network_ids_agree() {
        assert_eq!(NETWORK.network_id(), NETWORK_ID);
    }

    fn accept_unsigned_policy() -> MetadataTrustPolicy {
        MetadataTrustPolicy::AcceptUnsigned
    }

    /// The strict posture, carrying the same allowlist the decode path checks
    /// against -- the shape `cli_trust_policy` builds.
    fn require_signed_policy_with(allowlist: SignerAllowlist) -> MetadataTrustPolicy {
        MetadataTrustPolicy::RequireAllowlistedSigner(allowlist)
    }

    /// The strict posture authorizing the NEAR dev curator.
    fn require_signed_policy() -> MetadataTrustPolicy {
        require_signed_policy_with(near_allowlist())
    }

    /// Enrol one key under one domain, the way `authorized_token_metadata_signers`
    /// does from its env vars.
    fn allowlist_with(domain: TokenMetadataDomain, canonical_pubkey: Vec<u8>) -> SignerAllowlist {
        let mut allow = SignerAllowlist::new();
        allow.insert(scoped_signer_key(domain, &canonical_pubkey));
        allow
    }

    fn near_allowlist() -> SignerAllowlist {
        allowlist_with(
            TokenMetadataDomain::Near,
            ed25519_dalek::SigningKey::from_bytes(&DEV_NEAR_SIGNING_KEY_SEED)
                .verifying_key()
                .to_bytes()
                .to_vec(),
        )
    }

    fn ethereum_allowlist() -> SignerAllowlist {
        let signing_key =
            Secp256k1SigningKey::from_bytes((&DEV_ETHEREUM_SIGNING_KEY_SEED).into()).unwrap();
        let verifying_key = Secp256k1VerifyingKey::from(&signing_key);
        allowlist_with(
            TokenMetadataDomain::Ethereum,
            verifying_key.to_encoded_point(false).as_bytes().to_vec(),
        )
    }

    fn solana_allowlist() -> SignerAllowlist {
        allowlist_with(
            TokenMetadataDomain::Solana,
            ed25519_dalek::SigningKey::from_bytes(&DEV_SOLANA_SIGNING_KEY_SEED)
                .verifying_key()
                .to_bytes()
                .to_vec(),
        )
    }

    fn empty_allowlists() -> SignerAllowlist {
        SignerAllowlist::new()
    }

    #[test]
    fn near_origin_valid_signature_verifies() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Near,
                &sig,
                &near_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn unspecified_origin_defaults_to_near_curve() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Near,
                &sig,
                &near_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn ethereum_origin_valid_signature_verifies() {
        let sig_meta = sign_token_metadata_secp256k1(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_ETHEREUM_SIGNING_KEY_SEED,
        )
        .unwrap();
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Ethereum,
                &sig,
                &ethereum_allowlist()
            )
            .is_ok()
        );
    }

    #[test]
    fn solana_origin_valid_signature_verifies() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_SOLANA_SIGNING_KEY_SEED,
            visualsign::signing::solana_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Solana,
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
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Solana,
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
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        let tampered = r#"{"symbol":"PHISH","decimals":6}"#;
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                tampered,
                TokenMetadataDomain::Near,
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
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert!(
            validate_token_metadata_signature(
                NETWORK_ID,
                "nep141:a-different-token.near",
                VALUE,
                TokenMetadataDomain::Near,
                &sig,
                &near_allowlist()
            )
            .is_err()
        );
    }

    /// Under the permissive posture an unrecognized signer is accepted, but
    /// only on the terms an unsigned entry gets: registered unverified, so the
    /// render carries the caveat.
    #[test]
    fn extract_accepts_an_unrecognized_signer_as_unverified() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            UNSEEDED_ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(NETWORK_ID.to_string()),
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: None,
                    },
                )]),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            NETWORK,
            &empty_allowlists(),
            &accept_unsigned_policy(),
        )
        .expect("an unrecognized signer is accepted under the permissive posture");
        assert!(
            !registry
                .by_asset_id
                .get(UNSEEDED_ASSET_ID)
                .expect("present")
                .verified,
            "an unrecognized signer must not register as verified"
        );
    }

    /// The strict posture refuses it, exactly as it refuses an unsigned entry.
    #[test]
    fn extract_rejects_an_unrecognized_signer_under_require_signed() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            UNSEEDED_ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(NETWORK_ID.to_string()),
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                NETWORK,
                &empty_allowlists(),
                &require_signed_policy_with(SignerAllowlist::new()),
            )
            .is_none_or(|r| r.by_asset_id.is_empty()),
            "the strict posture requires a recognized curator, not merely a signature"
        );
    }

    /// A signature from an unrecognized key must not buy a seed override the
    /// same caller could not have had by omitting it.
    #[test]
    fn extract_rejects_an_unrecognized_signer_overriding_a_seed() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(NETWORK_ID.to_string()),
                token_mappings: make_mappings(vec![(
                    ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: None,
                    },
                )]),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                NETWORK,
                &empty_allowlists(),
                &accept_unsigned_policy(),
            )
            .is_none_or(|r| r.by_asset_id.is_empty()),
            "an unverified entry must never shadow a curated seed"
        );
    }

    /// A signature nobody enrolled is not tampering. It verifies, and reports
    /// the signer as unrecognized -- which is what lets the caller treat it as
    /// an unsigned entry rather than refusing it outright.
    #[test]
    fn unlisted_signer_verifies_but_is_unrecognized() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        assert_eq!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Near,
                &sig,
                &empty_allowlists(),
            )
            .expect("an unenrolled key is not an integrity failure"),
            SignerIdentity::Unrecognized
        );
    }

    /// The same key enrolled for a different domain stays unrecognized here:
    /// scoping is what keeps "trusted to vouch for Solana-origin assets" from
    /// also meaning "trusted to vouch for NEAR-native ones".
    #[test]
    fn a_key_enrolled_for_another_domain_is_unrecognized() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
            ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let sig = convert_proto_signature(&sig_meta);
        let same_key_wrong_domain = allowlist_with(
            TokenMetadataDomain::Solana,
            ed25519_dalek::SigningKey::from_bytes(&DEV_NEAR_SIGNING_KEY_SEED)
                .verifying_key()
                .to_bytes()
                .to_vec(),
        );
        assert_eq!(
            validate_token_metadata_signature(
                NETWORK_ID,
                ASSET_ID,
                VALUE,
                TokenMetadataDomain::Near,
                &sig,
                &same_key_wrong_domain,
            )
            .expect("verifies"),
            SignerIdentity::Unrecognized,
            "enrolment under one domain must not authorize another"
        );
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
            try_extract_from_chain_metadata(
                None,
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy()
            )
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
                NETWORK,
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
            NETWORK,
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
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    // Regression coverage for the trust posture: an unsigned entry
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
                NETWORK,
                &near_allowlist(),
                &require_signed_policy()
            )
            .is_none()
        );
    }

    // A signature is scoped to one NEAR network, so the same signed entry
    // presented against the other network is rejected rather than accepted as
    // verified. This is the end-to-end half of
    // `test_token_metadata_prehash_is_bound_to_the_network`.
    #[test]
    fn extract_rejects_a_mainnet_signature_replayed_on_testnet() {
        let sig_meta = sign_token_metadata_ed25519(
            "NEAR_MAINNET",
            UNSEEDED_ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: Some(sig_meta),
                        origin_chain: Some(TokenOriginChain::Near as i32),
                    },
                )]),
            })),
        };

        // The control: checked against the network it was signed for, it
        // registers verified.
        let mainnet = try_extract_from_chain_metadata(
            Some(&metadata),
            NearNetwork::Mainnet,
            &near_allowlist(),
            &require_signed_policy(),
        )
        .expect("the mainnet signature verifies on mainnet");
        assert!(
            mainnet
                .by_asset_id
                .get(UNSEEDED_ASSET_ID)
                .expect("present")
                .verified,
            "signed entry registers as verified on its own network"
        );

        let testnet = try_extract_from_chain_metadata(
            Some(&metadata),
            NearNetwork::Testnet,
            &near_allowlist(),
            &require_signed_policy(),
        );
        assert!(
            testnet.is_none_or(|r| r.by_asset_id.is_empty()),
            "a mainnet-scoped signature must not verify on testnet"
        );
    }

    // The scope follows the `network` the caller resolved, not the
    // `network_id` field sitting in the request: the converter's fallback
    // network applies when a request omits the field, and the two must not
    // diverge. A testnet-scoped signature therefore verifies for a testnet
    // converter even though the request names no network at all.
    #[test]
    fn extract_scopes_to_the_resolved_network_not_the_metadata_field() {
        let sig_meta = sign_token_metadata_ed25519(
            "NEAR_TESTNET",
            UNSEEDED_ASSET_ID,
            VALUE,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: None,
                token_mappings: make_mappings(vec![(
                    UNSEEDED_ASSET_ID,
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
            NearNetwork::Testnet,
            &near_allowlist(),
            &require_signed_policy(),
        )
        .expect("a testnet-scoped signature verifies for a testnet converter");
        assert!(
            registry
                .by_asset_id
                .get(UNSEEDED_ASSET_ID)
                .expect("present")
                .verified
        );
    }

    // A validly signed, allowlisted entry still registers under the strict
    // posture: NearTokenTrustPolicy gates only whether a MISSING signature is
    // acceptable, not the per-origin-chain allowlist check a present one
    // already goes through unconditionally.
    #[test]
    fn extract_signed_entry_still_registers_under_require_signed_policy() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
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
            NETWORK,
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
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none(),
            "a symbol carrying a bidi override must not register"
        );
    }

    #[test]
    fn extract_rejects_a_mapping_over_the_entry_cap() {
        // One entry past the cap rejects the whole map, including the entries
        // that would individually have been fine.
        let entries: Vec<(String, TokenMetadataEntry)> = (0..=MAX_TOKEN_METADATA_ENTRIES)
            .map(|i| {
                (
                    format!("nep141:token-{i}.near"),
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )
            })
            .collect();
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: entries.into_iter().collect(),
            })),
        };
        assert!(
            try_extract_from_chain_metadata(
                Some(&metadata),
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy(),
            )
            .is_none(),
            "a mapping over the entry cap is rejected whole"
        );
    }

    #[test]
    fn extract_accepts_a_mapping_at_the_entry_cap() {
        let entries: Vec<(String, TokenMetadataEntry)> = (0..MAX_TOKEN_METADATA_ENTRIES)
            .map(|i| {
                (
                    format!("nep141:token-{i}.near"),
                    TokenMetadataEntry {
                        value: VALUE.to_string(),
                        signature: None,
                        origin_chain: None,
                    },
                )
            })
            .collect();
        let metadata = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some("NEAR_MAINNET".to_string()),
                token_mappings: entries.into_iter().collect(),
            })),
        };
        let registry = try_extract_from_chain_metadata(
            Some(&metadata),
            NETWORK,
            &near_allowlist(),
            &accept_unsigned_policy(),
        )
        .expect("a mapping exactly at the cap is accepted");
        assert_eq!(registry.by_asset_id.len(), MAX_TOKEN_METADATA_ENTRIES);
    }

    #[test]
    fn extract_rejects_an_unrecognized_origin_chain() {
        // An out-of-range discriminant is rejected even when the entry carries
        // an otherwise-valid NEAR curator signature: the entry names an origin
        // this build cannot verify, so it must not be accepted under the NEAR
        // curator's identity.
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
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
            NETWORK,
            &near_allowlist(),
            &require_signed_policy(),
        );
        assert!(
            registry.is_none_or(|r| r.by_asset_id.is_empty()),
            "an unrecognized origin_chain must not register under the NEAR curator"
        );
    }

    #[test]
    fn extract_signed_entry_verifies_and_registers() {
        let sig_meta = sign_token_metadata_ed25519(
            NETWORK_ID,
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
            NETWORK,
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
            NETWORK_ID,
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
                NETWORK,
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
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }

    // An entry whose `value` isn't valid JSON is rejected whether or not it
    // carries a signature. The signed case is the one the check order matters
    // for: the JSON parse runs before signature verification, so a structurally
    // broken entry is dropped without spending an elliptic-curve operation.
    // Both cases must still be rejected, which is what this pins.
    #[test]
    fn extract_malformed_value_json_skipped_signed_or_not() {
        let broken = r#"{"symbol":"BROKEN","decimals":}"#;
        let signed = sign_token_metadata_ed25519(
            NETWORK_ID,
            UNSEEDED_ASSET_ID,
            broken,
            &DEV_NEAR_SIGNING_KEY_SEED,
            visualsign::signing::near_token_metadata_prehash,
        );
        for signature in [None, Some(signed)] {
            let metadata = ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: None,
                    token_mappings: make_mappings(vec![(
                        UNSEEDED_ASSET_ID,
                        TokenMetadataEntry {
                            value: broken.to_string(),
                            signature,
                            origin_chain: None,
                        },
                    )]),
                })),
            };
            assert!(
                try_extract_from_chain_metadata(
                    Some(&metadata),
                    NETWORK,
                    &near_allowlist(),
                    &accept_unsigned_policy(),
                )
                .is_none_or(|r| r.by_asset_id.is_empty()),
                "an entry whose value is not valid JSON must never register"
            );
        }
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
                NETWORK,
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
                NETWORK,
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
                NETWORK,
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
                NETWORK,
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
                NETWORK,
                &near_allowlist(),
                &accept_unsigned_policy()
            )
            .is_none()
        );
    }
}
