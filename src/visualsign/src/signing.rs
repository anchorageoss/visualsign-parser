//! Domain-separated prehash for caller-supplied metadata signatures.
//!
//! Caller-supplied ABI (Ethereum) and IDL (Solana) metadata can carry an optional
//! signature (secp256k1 ECDSA for Ethereum ABI, ed25519 for Solana IDL).
//! Historically the prehash was `SHA-256(body)` over the
//! metadata JSON alone. That proves the body was not altered after signing, but it
//! binds the signature to nothing else: a signature minted for one (chain, on-chain
//! identity) is byte-for-byte valid when replayed under any other (chain, identity).
//! An attacker could lift a legitimately signed body and rebind it to a different
//! contract / program to steer the human-readable rendering of unrelated calldata.
//!
//! This module defines a shared, versioned, domain-separated prehash that closes the
//! replay window by committing to the chain and the on-chain identity the body
//! describes alongside the body itself.
//!
//! # v1 byte layout
//!
//! The prehash is computed over a length-prefixed encoding of four fields:
//!
//! ```text
//! prehash = SHA-256(
//!     le_u64(DOMAIN.len())    || DOMAIN    ||
//!     le_u64(chain_tag.len()) || chain_tag ||
//!     le_u64(scope.len())     || scope     ||
//!     le_u64(body.len())      || body
//! )
//! ```
//!
//! where:
//!
//! - `DOMAIN` is the constant ASCII string [`METADATA_SIGNING_DOMAIN_V1`]
//!   (`b"visualsign-metadata-v1"`). It version-stamps the construction so a future
//!   v2 layout cannot collide with a v1 prehash.
//! - `chain_tag` is a short ASCII tag identifying the chain family
//!   ([`CHAIN_TAG_ETHEREUM`] or [`CHAIN_TAG_SOLANA`]).
//! - `scope` is the chain-specific on-chain identity bytes (see below).
//! - `body` is the metadata JSON bytes verbatim (the ABI/IDL string as supplied).
//! - `le_u64(n)` is the 8-byte little-endian length of the field that immediately
//!   follows it. Prefixing every field with its length makes the encoding injective:
//!   distinct `(chain_tag, scope, body)` triples can never produce the same preimage,
//!   so no shifting of bytes between adjacent fields (including empty fields or fields
//!   that happen to contain length-prefix bytes) can forge an equivalent digest. This
//!   holds for arbitrary field contents, not just the fixed-width scopes used today.
//!
//! # Per-chain scope
//!
//! - **Ethereum** ([`ethereum_metadata_prehash`]): the scope is 28 bytes, the
//!   8-byte big-endian `chain_id` followed by the 20-byte contract address.
//! - **Solana** ([`solana_metadata_prehash`]): the scope is the 32-byte program id
//!   (pubkey).
//!
//! External signers reproduce a valid signature by computing the SHA-256 over this
//! exact byte sequence and signing the resulting 32-byte digest. The signing
//! algorithm is chosen per chain by the verifier: the Ethereum ABI path verifies
//! a secp256k1 ECDSA signature over the digest (prehash signing), and the Solana
//! IDL path verifies an ed25519 signature over the digest as its message. The
//! prehash construction itself is curve-agnostic. Note that the format is
//! intentionally explicit so it can be re-implemented in any language.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

/// Allowlist of authorized signer public keys, compared by their canonical
/// byte representation. The exact encoding depends on the curve:
/// - **secp256k1 (Ethereum ABI path)**: uncompressed SEC1 encoding (65 bytes).
///   The chain crates canonicalize before insert/lookup, so compressed vs
///   uncompressed forms of the same key match.
/// - **ed25519 (Solana IDL path)**: raw 32-byte public key (no SEC1 variants).
///
/// An EMPTY allowlist authorizes nothing: signed metadata is rejected
/// (fail-closed).
///
/// This type is deliberately crypto-free: it stores already-decoded key bytes and
/// never parses or validates them, so the core crate keeps no dependency on any
/// specific crypto implementation. Callers (the chain crates) decode and
/// canonicalize the keys before inserting or looking them up. It is shared so the
/// Ethereum ABI and Solana IDL signature paths can enforce the same mechanism.
#[derive(Debug, Clone, Default)]
pub struct SignerAllowlist {
    keys: BTreeSet<Vec<u8>>,
}

impl SignerAllowlist {
    /// Creates an empty allowlist. An empty allowlist authorizes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a signer's canonical public-key bytes to the allowlist.
    pub fn insert(&mut self, canonical_pubkey: Vec<u8>) {
        self.keys.insert(canonical_pubkey);
    }

    /// Returns `true` if the canonical public-key bytes are authorized.
    #[must_use]
    pub fn contains(&self, canonical_pubkey: &[u8]) -> bool {
        self.keys.contains(canonical_pubkey)
    }

    /// Returns `true` if the allowlist holds no keys (authorizes nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Returns the number of distinct authorized keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

impl FromIterator<Vec<u8>> for SignerAllowlist {
    fn from_iter<I: IntoIterator<Item = Vec<u8>>>(iter: I) -> Self {
        Self {
            keys: iter.into_iter().collect(),
        }
    }
}

/// Deploy-time trust posture for caller-supplied metadata (Ethereum ABI
/// mappings today; the Solana IDL path still uses a bare [`SignerAllowlist`]).
///
/// The posture is picked once by whoever deploys the parser and is fixed for the
/// life of the process. Nothing in a request can move the parser between the two
/// variants: a caller cannot opt itself into leniency by omitting a signature,
/// and cannot opt itself into strictness either. That is the point of the type.
/// Because the choice lives on the parser's cmdline (for the enclave binary, in
/// the `pivotArgs` of the signed TVC manifest), a signer can verify out of band
/// which posture the deployment they are signing against actually runs, instead
/// of trusting a per-request signal or a log line that no one sees.
///
/// The two variants are mutually exclusive by construction:
///
/// - [`Self::AcceptUnsigned`]: metadata with no signature is accepted. A
///   signature that IS present must still verify against the public key carried
///   alongside it in the same untrusted metadata (so a mutated payload under an
///   unchanged signature/key pair is still rejected), but that key's identity is
///   not checked against an allowlist, because a deployment that accepts
///   unsigned metadata gains nothing from an identity check an attacker can
///   sidestep by simply dropping the signature (or re-signing with a key of
///   their own choosing).
/// - [`Self::RequireAllowlistedSigner`]: every entry must carry a signature that
///   verifies AND whose key is in the allowlist. Missing, malformed, and
///   unauthorized signatures are all rejected. An empty allowlist rejects
///   everything (fail-closed).
///
/// There is deliberately no `Default`: a deployment has to say which posture it
/// runs.
#[derive(Debug, Clone)]
pub enum MetadataTrustPolicy {
    /// Accept unsigned metadata; verify integrity only when a signature is present.
    AcceptUnsigned,
    /// Reject any metadata not signed by one of the allowlisted keys.
    RequireAllowlistedSigner(SignerAllowlist),
}

impl MetadataTrustPolicy {
    /// Returns `true` if metadata with no signature at all is accepted.
    #[must_use]
    pub fn accepts_unsigned(&self) -> bool {
        matches!(self, Self::AcceptUnsigned)
    }

    /// The allowlist a present signature's signer must appear in, or `None` when
    /// the posture only checks signature integrity (see [`Self::AcceptUnsigned`]).
    #[must_use]
    pub fn signer_allowlist(&self) -> Option<&SignerAllowlist> {
        match self {
            Self::AcceptUnsigned => None,
            Self::RequireAllowlistedSigner(allow) => Some(allow),
        }
    }
}

impl std::fmt::Display for MetadataTrustPolicy {
    /// Human-readable posture, for the one-line startup log that tells an
    /// operator which mode the process came up in.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AcceptUnsigned => write!(f, "accept-unsigned"),
            Self::RequireAllowlistedSigner(allow) => {
                write!(f, "require-signed ({} authorized signer(s))", allow.len())
            }
        }
    }
}

/// Domain separation tag for the v1 metadata signing prehash.
///
/// Version-stamps the construction so prehashes from different format versions
/// cannot collide.
pub const METADATA_SIGNING_DOMAIN_V1: &[u8] = b"visualsign-metadata-v1";

/// Chain tag for Ethereum (and EVM-compatible) metadata signatures.
pub const CHAIN_TAG_ETHEREUM: &str = "ethereum";

/// Chain tag for Solana metadata signatures.
pub const CHAIN_TAG_SOLANA: &str = "solana";

/// Core constructor for the v1 domain-separated metadata prehash.
///
/// Computes the SHA-256 over the length-prefixed encoding of
/// `(DOMAIN, chain_tag, scope, body)`. See the module documentation for the precise
/// byte layout and the per-chain definition of `scope`.
#[must_use]
pub fn metadata_signing_prehash_v1(chain_tag: &str, scope: &[u8], body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    // Prefix every field with its little-endian u64 length so the concatenation is
    // injective for arbitrary field contents (a field's bytes can never be reread as
    // part of an adjacent field). usize -> u64 is a lossless widening on supported
    // (<= 64-bit) targets.
    for field in [
        METADATA_SIGNING_DOMAIN_V1,
        chain_tag.as_bytes(),
        scope,
        body,
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

/// Ethereum scope = 8-byte big-endian chain_id followed by the 20-byte address.
#[must_use]
pub fn ethereum_metadata_prehash(chain_id: u64, address: &[u8; 20], abi_json: &[u8]) -> [u8; 32] {
    let mut scope = [0u8; 28];
    scope[..8].copy_from_slice(&chain_id.to_be_bytes());
    scope[8..].copy_from_slice(address);
    metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, &scope, abi_json)
}

/// Solana scope = the 32-byte program id (pubkey).
#[must_use]
pub fn solana_metadata_prehash(program_id: &[u8; 32], idl_json: &[u8]) -> [u8; 32] {
    metadata_signing_prehash_v1(CHAIN_TAG_SOLANA, program_id, idl_json)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_prehash_is_deterministic() {
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope", b"body");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope", b"body");
        assert_eq!(a, b, "same inputs must produce the same prehash");
    }

    #[test]
    fn test_changing_chain_tag_changes_prehash() {
        let eth = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope", b"body");
        let sol = metadata_signing_prehash_v1(CHAIN_TAG_SOLANA, b"scope", b"body");
        assert_ne!(eth, sol, "different chain_tag must change the prehash");
    }

    #[test]
    fn test_changing_scope_changes_prehash() {
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope-a", b"body");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope-b", b"body");
        assert_ne!(a, b, "different scope must change the prehash");
    }

    #[test]
    fn test_changing_body_changes_prehash() {
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope", b"body-a");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"scope", b"body-b");
        assert_ne!(a, b, "different body must change the prehash");
    }

    #[test]
    fn test_length_prefix_prevents_field_boundary_ambiguity() {
        // A naive concatenation would let ("ab", "c") and ("a", "bc") collide.
        // Length-prefixing keeps field boundaries unambiguous, so these must differ.
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"ab", b"c");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"a", b"bc");
        assert_ne!(a, b, "field boundaries must be unambiguous");
    }

    #[test]
    fn test_length_prefix_resists_separator_byte_collision() {
        // Regression for the single-delimiter weakness: with a lone 0x00 separator,
        // (scope = [0x00, b'y'], body = [b'z']) and (scope = [], body = [b'y', 0x00,
        // b'z']) hash the same naive preimage. Length-prefixing must keep them
        // distinct even when a field contains the separator byte or is empty.
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, &[0x00, b'y'], b"z");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, &[], &[b'y', 0x00, b'z']);
        assert_ne!(a, b, "embedded separator bytes must not create a collision");
    }

    #[test]
    fn test_ethereum_wrapper_matches_hand_computed_prehash() {
        let chain_id: u64 = 1;
        let address: [u8; 20] = [0x11u8; 20];
        let abi_json = b"[]";

        let mut scope = [0u8; 28];
        scope[..8].copy_from_slice(&chain_id.to_be_bytes());
        scope[8..].copy_from_slice(&address);
        let expected = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, &scope, abi_json);

        let actual = ethereum_metadata_prehash(chain_id, &address, abi_json);
        assert_eq!(
            actual, expected,
            "ethereum wrapper must match the hand-computed core call"
        );
    }

    #[test]
    fn test_ethereum_prehash_binds_chain_and_address() {
        let address_a: [u8; 20] = [0x11u8; 20];
        let address_b: [u8; 20] = [0x22u8; 20];
        let abi_json = b"[]";

        let base = ethereum_metadata_prehash(1, &address_a, abi_json);
        // Different address, same chain.
        assert_ne!(base, ethereum_metadata_prehash(1, &address_b, abi_json));
        // Different chain, same address.
        assert_ne!(base, ethereum_metadata_prehash(137, &address_a, abi_json));
    }

    #[test]
    fn test_allowlist_empty_by_default() {
        let allow = SignerAllowlist::new();
        assert!(allow.is_empty(), "a fresh allowlist must be empty");
        assert_eq!(allow.len(), 0);
        assert!(
            !allow.contains(b"any-key"),
            "an empty allowlist must authorize nothing (fail-closed)"
        );
    }

    #[test]
    fn test_allowlist_insert_and_contains() {
        let mut allow = SignerAllowlist::new();
        allow.insert(b"key-a".to_vec());
        assert!(!allow.is_empty());
        assert_eq!(allow.len(), 1);
        assert!(allow.contains(b"key-a"), "inserted key must be authorized");
        assert!(
            !allow.contains(b"key-b"),
            "a key that was never inserted must not be authorized"
        );
    }

    #[test]
    fn test_allowlist_insert_is_idempotent() {
        let mut allow = SignerAllowlist::new();
        allow.insert(b"key-a".to_vec());
        allow.insert(b"key-a".to_vec());
        assert_eq!(
            allow.len(),
            1,
            "inserting the same key twice keeps one entry"
        );
    }

    #[test]
    fn test_allowlist_from_iter() {
        let allow: SignerAllowlist = [b"key-a".to_vec(), b"key-b".to_vec(), b"key-a".to_vec()]
            .into_iter()
            .collect();
        // Duplicate "key-a" collapses to a single entry.
        assert_eq!(allow.len(), 2);
        assert!(allow.contains(b"key-a"));
        assert!(allow.contains(b"key-b"));
        assert!(!allow.contains(b"key-c"));
    }

    #[test]
    fn test_accept_unsigned_policy_skips_signer_identity() {
        let policy = MetadataTrustPolicy::AcceptUnsigned;
        assert!(policy.accepts_unsigned());
        assert!(
            policy.signer_allowlist().is_none(),
            "accept-unsigned must not enforce signer identity: an attacker can \
             always drop the signature instead"
        );
    }

    #[test]
    fn test_require_signed_policy_exposes_allowlist() {
        let mut allow = SignerAllowlist::new();
        allow.insert(b"key-a".to_vec());
        let policy = MetadataTrustPolicy::RequireAllowlistedSigner(allow);

        assert!(
            !policy.accepts_unsigned(),
            "require-signed must reject unsigned metadata"
        );
        let enforced = policy.signer_allowlist().expect("allowlist is enforced");
        assert!(enforced.contains(b"key-a"));
        assert!(!enforced.contains(b"key-b"));
    }

    #[test]
    fn test_policy_display_names_the_posture() {
        assert_eq!(
            MetadataTrustPolicy::AcceptUnsigned.to_string(),
            "accept-unsigned"
        );

        let mut allow = SignerAllowlist::new();
        allow.insert(b"key-a".to_vec());
        allow.insert(b"key-b".to_vec());
        assert_eq!(
            MetadataTrustPolicy::RequireAllowlistedSigner(allow).to_string(),
            "require-signed (2 authorized signer(s))"
        );
    }

    /// An empty allowlist under require-signed authorizes nothing, so the posture
    /// rejects every entry (fail-closed) rather than degrading to accept-unsigned.
    #[test]
    fn test_require_signed_with_empty_allowlist_authorizes_nothing() {
        let policy = MetadataTrustPolicy::RequireAllowlistedSigner(SignerAllowlist::new());
        assert!(!policy.accepts_unsigned());
        assert!(policy
            .signer_allowlist()
            .expect("allowlist is enforced")
            .is_empty());
    }

    #[test]
    fn test_solana_wrapper_matches_hand_computed_prehash() {
        let program_id: [u8; 32] = [0x33u8; 32];
        let idl_json = b"{}";

        let expected = metadata_signing_prehash_v1(CHAIN_TAG_SOLANA, &program_id, idl_json);
        let actual = solana_metadata_prehash(&program_id, idl_json);
        assert_eq!(
            actual, expected,
            "solana wrapper must match the hand-computed core call"
        );
    }
}
