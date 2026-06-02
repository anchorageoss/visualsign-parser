//! Domain-separated prehash for caller-supplied metadata signatures.
//!
//! Caller-supplied ABI (Ethereum) and IDL (Solana) metadata can carry an optional
//! secp256k1 signature. Historically the prehash was `SHA-256(body)` over the
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
//! The prehash is computed as:
//!
//! ```text
//! prehash = SHA-256( DOMAIN || 0x00 || chain_tag || 0x00 || scope || 0x00 || body )
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
//! - `0x00` is a single zero-byte separator placed between each field so the
//!   concatenation is unambiguous and an attacker cannot shift bytes from one field
//!   into an adjacent one to forge an equivalent preimage.
//!
//! # Per-chain scope
//!
//! - **Ethereum** ([`ethereum_metadata_prehash`]): the scope is 28 bytes, the
//!   8-byte big-endian `chain_id` followed by the 20-byte contract address.
//! - **Solana** ([`solana_metadata_prehash`]): the scope is the 32-byte program id
//!   (pubkey).
//!
//! External signers reproduce a valid signature by computing the SHA-256 over this
//! exact byte sequence and signing the resulting 32-byte digest with secp256k1
//! (prehash signing). Note that the format is intentionally explicit so it can be
//! re-implemented in any language.

use sha2::{Digest, Sha256};

/// Domain separation tag for the v1 metadata signing prehash.
///
/// Version-stamps the construction so prehashes from different format versions
/// cannot collide.
pub const METADATA_SIGNING_DOMAIN_V1: &[u8] = b"visualsign-metadata-v1";

/// Chain tag for Ethereum (and EVM-compatible) metadata signatures.
pub const CHAIN_TAG_ETHEREUM: &str = "ethereum";

/// Chain tag for Solana metadata signatures.
pub const CHAIN_TAG_SOLANA: &str = "solana";

/// Single-byte separator placed between each field of the prehash preimage.
const FIELD_SEPARATOR: u8 = 0x00;

/// Core constructor for the v1 domain-separated metadata prehash.
///
/// Computes `SHA-256(DOMAIN || 0x00 || chain_tag || 0x00 || scope || 0x00 || body)`.
/// See the module documentation for the precise byte layout and the per-chain
/// definition of `scope`.
#[must_use]
pub fn metadata_signing_prehash_v1(chain_tag: &str, scope: &[u8], body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(METADATA_SIGNING_DOMAIN_V1);
    hasher.update([FIELD_SEPARATOR]);
    hasher.update(chain_tag.as_bytes());
    hasher.update([FIELD_SEPARATOR]);
    hasher.update(scope);
    hasher.update([FIELD_SEPARATOR]);
    hasher.update(body);
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
    fn test_separator_prevents_field_boundary_ambiguity() {
        // Without the 0x00 separators, ("ab", "c") and ("a", "bc") would collide.
        // The separators keep field boundaries unambiguous, so these must differ.
        let a = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"ab", b"c");
        let b = metadata_signing_prehash_v1(CHAIN_TAG_ETHEREUM, b"a", b"bc");
        assert_ne!(a, b, "field boundaries must be unambiguous");
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
