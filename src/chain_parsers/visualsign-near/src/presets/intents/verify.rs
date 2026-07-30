//! Signature verification + payload extraction. Defuse types stay internal;
//! results are normalized to `String`/`visualsign` shapes by `render.rs`.

use defuse_core::intents::DefuseIntents;
use defuse_core::payload::multi::MultiPayload;
use defuse_core::payload::{DefusePayload, ExtractDefusePayload};
use defuse_crypto::SignedPayload;

/// Outcome of verifying one payload's signature.
#[derive(Debug)]
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
    /// Cryptographically well-formed but encodes a recovery-id convention
    /// this wire format doesn't accept (e.g. Ethereum's v=27/28 instead of
    /// the 0-3 this format expects). Not a proof of tampering -- rendered
    /// with its own diagnostic rather than folded into `Invalid`, whose
    /// wording implies the cryptography itself failed.
    MalformedEncoding(String),
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
    let check = match invalid_secp256k1_recovery_id_reason(payload) {
        Some(reason) => SignatureCheck::MalformedEncoding(reason),
        None => match payload.verify() {
            Some(key) => SignatureCheck::Valid {
                recovered_key: key.to_string(),
                implied_account_id: key.to_implicit_account_id().to_string(),
            },
            None => SignatureCheck::Invalid,
        },
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
///
/// Returns the human-readable reason when rejected, `None` when the
/// signature's recovery id is in range (or the standard has none). The most
/// common out-of-range case is Ethereum's v=27/28 convention (`recovery_id +
/// 27`, per MetaMask/`personal_sign`) landing on a wire format that expects
/// the raw 0-3 recovery id -- flagged with a specific hint, since that's a
/// wrong encoding, not a broken signature.
fn invalid_secp256k1_recovery_id_reason(payload: &MultiPayload) -> Option<String> {
    let signature = match payload {
        MultiPayload::Erc191(signed) => &signed.signature,
        MultiPayload::Tip191(signed) => &signed.signature,
        // Ed25519/P256 standards have no recovery id, so ecrecover's
        // out-of-range panic path cannot be reached for these variants.
        MultiPayload::Nep413(_)
        | MultiPayload::RawEd25519(_)
        | MultiPayload::WebAuthn(_)
        | MultiPayload::TonConnect(_)
        | MultiPayload::Sep53(_) => return None,
    };
    let recovery_id = signature[64];
    if recovery_id < 4 {
        return None;
    }
    let hint = if recovery_id == 27 || recovery_id == 28 {
        " (Ethereum v=27/28 must be normalized)"
    } else {
        ""
    };
    Some(format!(
        "malformed signature encoding: recovery id {recovery_id}, expected 0-3{hint}"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::presets::intents::args::decode_args;

    const VECTOR: &[u8] = include_bytes!("../../../tests/fixtures/_vector_raw_ed25519.input");

    #[test]
    fn valid_signature_verifies_natively() {
        // Proves defuse's canonical RawEd25519 verification runs off-chain
        // (pure-Rust crypto via near-sdk's non-contract-usage mode).
        let payloads = decode_args(VECTOR).expect("decode");
        let (check, _) = verify_and_extract(&payloads[0]);
        match check {
            SignatureCheck::Valid { recovered_key } => assert_eq!(
                recovered_key,
                "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"
            ),
            SignatureCheck::Invalid => panic!("expected a valid signature"),
        }
    }

    #[test]
    fn tampered_signature_is_invalid() {
        // Flip the embedded signature: verification must fail (not panic).
        let good = String::from_utf8(VECTOR.to_vec()).expect("utf8");
        let bad = good.replace(
            "3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj",
            "3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVk",
        );
        let payloads = decode_args(bad.as_bytes()).expect("decode");
        let (check, _) = verify_and_extract(&payloads[0]);
        assert!(matches!(check, SignatureCheck::Invalid));
    }

    // ---------------------------------------------------------------
    // ERC-191 (secp256k1 / EVM wallets)
    //
    // The vector is GENERATED deterministically (fixed key, RFC6979
    // signing, fixed nonce/deadline) and pinned as a fixture file;
    // regenerate with:
    //   UPDATE_TESTDATA=1 cargo test -p visualsign-near erc191
    // ---------------------------------------------------------------

    /// Throwaway test-only signing key (any fixed valid scalar).
    const ERC191_TEST_KEY: [u8; 32] = [0x42; 32];

    fn erc191_vector_path() -> String {
        format!(
            "{}/tests/fixtures/_vector_erc191.input",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// The signer's uncompressed public key, SEC1 minus the 0x04 prefix.
    fn erc191_test_pubkey64() -> [u8; 64] {
        let sk = k256::ecdsa::SigningKey::from_bytes((&ERC191_TEST_KEY).into()).unwrap();
        let point = sk.verifying_key().to_encoded_point(false);
        let mut pk = [0u8; 64];
        pk.copy_from_slice(&point.as_bytes()[1..65]);
        pk
    }

    /// Builds the signed ERC-191 `execute_intents` args deterministically.
    fn build_erc191_vector() -> String {
        let pk64 = erc191_test_pubkey64();
        // Implicit EVM-style account id: keccak(pubkey)[12..], hex.
        let addr_hash = near_sdk::env::keccak256_array(pk64);
        let signer_id = format!("0x{}", hex::encode(&addr_hash[12..]));

        let inner = serde_json::json!({
            "signer_id": signer_id,
            "verifying_contract": "intents.near",
            "deadline": "2100-01-01T00:00:00Z",
            "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
            "intents": [{
                "intent": "token_diff",
                "diff": {
                    "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near": "-1000",
                    "nep141:eth-0xdac17f958d2ee523a2206206994597c13d831ec7.omft.near": "998",
                },
            }],
        })
        .to_string();

        // ERC-191 personal_sign: keccak256("\x19Ethereum Signed Message:\n"
        // + len + message), then a recoverable secp256k1 signature (r||s||v,
        // v in {0,1}). RFC6979 makes the signature deterministic.
        let prehash = [
            format!("\x19Ethereum Signed Message:\n{}", inner.len()).into_bytes(),
            inner.clone().into_bytes(),
        ]
        .concat();
        let hash = near_sdk::env::keccak256_array(&prehash);
        let sk = k256::ecdsa::SigningKey::from_bytes((&ERC191_TEST_KEY).into()).unwrap();
        let (sig, recovery_id) = sk.sign_prehash_recoverable(&hash).unwrap();
        let mut sig65 = [0u8; 65];
        sig65[..64].copy_from_slice(&sig.to_bytes());
        sig65[64] = recovery_id.to_byte();

        serde_json::json!({
            "signed": [{
                "standard": "erc191",
                "payload": inner,
                "signature": format!("secp256k1:{}", bs58::encode(sig65).into_string()),
            }],
        })
        .to_string()
    }

    #[test]
    fn erc191_vector_matches_generator() {
        let built = build_erc191_vector();
        let path = erc191_vector_path();
        if std::env::var("UPDATE_TESTDATA").is_ok() {
            std::fs::write(&path, format!("{built}\n")).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing {path} (UPDATE_TESTDATA=1 regenerates): {e}"));
        assert_eq!(committed.trim(), built, "vector drifted from its generator");
    }

    #[test]
    fn erc191_valid_signature_recovers_the_signer_key() {
        // Proves defuse's canonical ERC-191 verification (keccak prehash +
        // ecrecover) runs off-chain, and recovers exactly the signing key.
        let payloads = decode_args(build_erc191_vector().as_bytes()).expect("decode");
        let (check, extracted) = verify_and_extract(&payloads[0]);
        let expected = format!(
            "secp256k1:{}",
            bs58::encode(erc191_test_pubkey64()).into_string()
        );
        match check {
            SignatureCheck::Valid { recovered_key } => assert_eq!(recovered_key, expected),
            SignatureCheck::Invalid => panic!("expected a valid signature"),
        }
        assert!(extracted.is_some(), "inner payload must extract");
    }

    #[test]
    fn erc191_tampered_payload_recovers_a_different_key() {
        // ERC-191 "verification" is RECOVERY: any well-formed signature
        // recovers SOME key, so tampering with the message does not fail
        // verification -- it recovers a different key than the signer's.
        // Account binding is therefore the comparison against the claimed
        // signer, not the recovery itself.
        let tampered = build_erc191_vector().replace("\\\"998\\\"", "\\\"999\\\"");
        let payloads = decode_args(tampered.as_bytes()).expect("decode");
        let (check, _) = verify_and_extract(&payloads[0]);
        let signer = format!(
            "secp256k1:{}",
            bs58::encode(erc191_test_pubkey64()).into_string()
        );
        match check {
            SignatureCheck::Valid { recovered_key } => assert_ne!(
                recovered_key, signer,
                "a tampered message must not recover the signer's key"
            ),
            // Some tampers land on an unrecoverable point; also acceptable.
            SignatureCheck::Invalid => {}
        }
    }

    #[test]
    fn erc191_invalid_recovery_byte_fails_verification() {
        // v outside {0..3} makes ecrecover fail outright.
        let good = build_erc191_vector();
        let payloads = decode_args(good.as_bytes()).expect("decode");
        let MultiPayload::Erc191(signed) = &payloads[0] else {
            panic!("expected erc191 payload");
        };
        let mut sig = signed.signature;
        sig[64] = 29; // invalid recovery id
        let bad = serde_json::json!({
            "signed": [{
                "standard": "erc191",
                "payload": signed.payload.0,
                "signature": format!("secp256k1:{}", bs58::encode(sig).into_string()),
            }],
        })
        .to_string();
        let payloads = decode_args(bad.as_bytes()).expect("decode");
        let (check, _) = verify_and_extract(&payloads[0]);
        assert!(matches!(check, SignatureCheck::Invalid));
    }

    // ---------------------------------------------------------------
    // MetaMask provenance pin: an independent check that our ERC-191
    // generator produces byte-for-byte what a real wallet would. The key
    // and expected signature are copied verbatim from near/intents' own
    // `erc191` crate test suite (a signature MetaMask actually produced);
    // reproducing it here means our generated vector above carries the
    // same provenance without a wallet in the loop.
    // ---------------------------------------------------------------

    #[test]
    fn erc191_generator_reproduces_metamask_reference_signature() {
        const KEY_HEX: &str = "a4b319a82adfc43584e4537fec97a80516e16673db382cd91eba97abbab8ca56";
        const MESSAGE: &str = "Hello world!";
        const EXPECTED_SIGNATURE_HEX: &str = "7800a70d05cde2c49ed546a6ce887ce6027c2c268c0285f6efef0cdfc4366b23643790f67a86468ee8301ed12cfffcb07c6530f90a9327ec057800fabd332e471c";

        let key: [u8; 32] = hex::decode(KEY_HEX)
            .expect("valid hex")
            .try_into()
            .expect("32 bytes");
        let expected: [u8; 65] = hex::decode(EXPECTED_SIGNATURE_HEX)
            .expect("valid hex")
            .try_into()
            .expect("65 bytes");

        let prehash = [
            format!("\x19Ethereum Signed Message:\n{}", MESSAGE.len()).into_bytes(),
            MESSAGE.as_bytes().to_vec(),
        ]
        .concat();
        let hash = near_sdk::env::keccak256_array(&prehash);
        let sk = k256::ecdsa::SigningKey::from_bytes((&key).into()).expect("valid key");
        let (sig, recovery_id) = sk.sign_prehash_recoverable(&hash).expect("sign");

        let mut sig65 = [0u8; 65];
        sig65[..64].copy_from_slice(&sig.to_bytes());
        // MetaMask/Ethereum's v convention is recovery_id + 27, not the raw
        // 0/1 this crate uses on the wire elsewhere in this module.
        sig65[64] = recovery_id.to_byte() + 27;

        assert_eq!(
            sig65, expected,
            "generator must reproduce the MetaMask-produced reference signature byte-for-byte"
        );
    }
}
