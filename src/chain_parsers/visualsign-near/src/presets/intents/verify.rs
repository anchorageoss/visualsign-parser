//! Signature verification + payload extraction. Defuse types stay internal;
//! results are normalized to `String`/`visualsign` shapes by `render.rs`.

use defuse_core::intents::DefuseIntents;
use defuse_core::payload::multi::MultiPayload;
use defuse_core::payload::{DefusePayload, ExtractDefusePayload};
use defuse_crypto::SignedPayload;

/// Outcome of verifying one payload's signature.
pub(crate) enum SignatureCheck {
    /// Signature is cryptographically valid.
    Valid {
        /// The public key the signature recovers to.
        recovered_key: String,
        /// The account id `recovered_key` implies under NEAR's (or defuse's
        /// EVM-style) implicit-account convention. Comparable against a
        /// payload's `signer_id` when that id has the same shape -- see
        /// `render.rs::looks_like_implicit_account`. For a named account
        /// (e.g. `alice.near`), this comparison says nothing: named accounts'
        /// access keys are registered on-chain, decoupled from any implicit
        /// derivation.
        implied_account_id: String,
    },
    /// Signature did not verify.
    Invalid,
}

/// Verify the signature and extract the inner intents payload.
///
/// `verify()` and `extract_defuse_payload()` are independent: a payload with an
/// invalid signature or an unparseable inner body can still yield the other
/// half, so both are attempted and reported separately. Extraction failure
/// keeps its error message (rather than collapsing to `None`) so the caller
/// can surface *why* no envelope/intents rendered instead of silently
/// omitting them.
pub(crate) fn verify_and_extract(
    payload: &MultiPayload,
) -> (SignatureCheck, Result<DefusePayload<DefuseIntents>, String>) {
    let check = if has_invalid_secp256k1_recovery_id(payload) {
        SignatureCheck::Invalid
    } else {
        match payload.verify() {
            Some(key) => SignatureCheck::Valid {
                recovered_key: key.to_string(),
                implied_account_id: key.to_implicit_account_id().to_string(),
            },
            None => SignatureCheck::Invalid,
        }
    };
    let extracted = payload
        .clone()
        .extract_defuse_payload()
        .map_err(|e| e.to_string());
    (check, extracted)
}

/// Rejects secp256k1 signatures whose recovery byte is out of range before
/// they reach `verify()`: near-crypto's native `ecrecover` backend panics on
/// an invalid recovery id rather than returning `None`, and a signing service
/// must treat a malformed payload as invalid input, never as a crash.
fn has_invalid_secp256k1_recovery_id(payload: &MultiPayload) -> bool {
    let signature = match payload {
        MultiPayload::Erc191(signed) => &signed.signature,
        MultiPayload::Tip191(signed) => &signed.signature,
        // Ed25519/P256 standards have no recovery id, so ecrecover's
        // out-of-range panic path cannot be reached for these variants.
        MultiPayload::Nep413(_)
        | MultiPayload::RawEd25519(_)
        | MultiPayload::WebAuthn(_)
        | MultiPayload::TonConnect(_)
        | MultiPayload::Sep53(_) => return false,
    };
    signature[64] >= 4
}
