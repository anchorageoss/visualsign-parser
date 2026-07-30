//! Signature verification + payload extraction. Defuse types stay internal;
//! results are normalized to `String`/`visualsign` shapes by `render.rs`.

use defuse_core::intents::DefuseIntents;
use defuse_core::payload::multi::MultiPayload;
use defuse_core::payload::{DefusePayload, ExtractDefusePayload};
use defuse_crypto::SignedPayload;

/// Outcome of verifying one payload's signature.
pub(crate) enum SignatureCheck {
    /// Signature is cryptographically valid; `recovered_key` is the public key
    /// the signature recovers to (NOT a proof of account binding).
    Valid { recovered_key: String },
    /// Signature did not verify.
    Invalid,
}

/// Verify the signature and extract the inner intents payload.
///
/// `verify()` and `extract_defuse_payload()` are independent: a payload with an
/// invalid signature or an unparseable inner body can still yield the other
/// half, so both are attempted and reported separately.
pub(crate) fn verify_and_extract(
    payload: &MultiPayload,
) -> (SignatureCheck, Option<DefusePayload<DefuseIntents>>) {
    let check = if has_invalid_secp256k1_recovery_id(payload) {
        SignatureCheck::Invalid
    } else {
        match payload.verify() {
            Some(key) => SignatureCheck::Valid {
                recovered_key: key.to_string(),
            },
            None => SignatureCheck::Invalid,
        }
    };
    let extracted = payload.clone().extract_defuse_payload().ok();
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
        _ => return false,
    };
    signature[64] >= 4
}
