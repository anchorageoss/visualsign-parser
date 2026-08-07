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

    /// The signed payload is verbatim from the pinned dependency's own test
    /// suite -- `defuse/v0.4.2`, `core/src/payload/multi.rs:120` (`fn
    /// raw_ed25519`) -- so its provenance is checkable by diffing against that
    /// file rather than by trusting this comment.
    ///
    /// Its inner body carries `"deadline":{"timestamp":...}`, the pre-v0.4.x
    /// object form, so it exercises signature verification only: v0.4.2's
    /// `Deadline` accepts an RFC-3339 string, and the signature covers the body
    /// byte-for-byte, so the format cannot be updated without invalidating the
    /// vector. Extraction of a current-format ed25519 envelope is covered by
    /// `super::tests::pipeline_decodes_and_renders_intent_section`.
    const VECTOR: &[u8] = include_bytes!("../../../tests/fixtures/_vector_raw_ed25519.input");

    #[test]
    fn valid_signature_verifies_natively() {
        // Proves defuse's canonical RawEd25519 verification runs off-chain
        // (pure-Rust crypto via near-sdk's non-contract-usage mode).
        let payloads = decode_args(VECTOR).expect("decode");
        let (check, extracted) = verify_and_extract(&payloads[0]);
        match check {
            SignatureCheck::Valid { recovered_key, .. } => assert_eq!(
                recovered_key,
                "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"
            ),
            other => panic!("expected a valid signature, got {other:?}"),
        }
        // Asserted rather than discarded: this vector's deadline predates the
        // pinned format (see VECTOR), so a silently-dropped error here would
        // let the fixture read as a fully valid intents payload.
        let err = extracted.expect_err("pre-v0.4.x deadline must not extract");
        assert!(err.contains("RFC 3339"), "{err}");
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
    // This vector is self-produced: generated deterministically (fixed key,
    // RFC6979 signing, fixed nonce/deadline) and committed as a fixture file,
    // regeneratable with:
    //   UPDATE_TESTDATA=1 cargo test -p visualsign-near erc191
    //
    // Two layers cover it, and one gap remains. `erc191_vector_matches_generator`
    // fails when the committed bytes and the generator disagree, so neither
    // drifts silently. The verification tests decode the committed bytes through
    // the production verify path, so a bug in the generator alone surfaces as a
    // signature that no longer verifies -- regenerating cannot bless it into
    // passing. What neither layer catches is a bug in logic the generator and
    // the verifier share, since regeneration moves both together; the
    // real-wallet MetaMask and TRON vectors below are the external anchor for
    // that, and no command in this repo can rewrite them.
    // ---------------------------------------------------------------

    /// Throwaway test-only signing key (any fixed valid scalar).
    const ERC191_TEST_KEY: [u8; 32] = [0x42; 32];

    /// The pinned ERC-191 fixture. Verification tests decode this directly
    /// (matching `VECTOR` for raw_ed25519 above) rather than a freshly built
    /// vector, so a generator regression shows up as a signature that no
    /// longer verifies, not as a fixture that silently rewrites itself under
    /// `UPDATE_TESTDATA=1` and still "passes". `build_erc191_vector()` is used
    /// solely by the drift check below.
    const ERC191_VECTOR: &[u8] = include_bytes!("../../../tests/fixtures/_vector_erc191.input");

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
        let payloads = decode_args(ERC191_VECTOR).expect("decode");
        let (check, extracted) = verify_and_extract(&payloads[0]);
        let expected = format!(
            "secp256k1:{}",
            bs58::encode(erc191_test_pubkey64()).into_string()
        );
        match check {
            SignatureCheck::Valid { recovered_key, .. } => assert_eq!(recovered_key, expected),
            other => panic!("expected a valid signature, got {other:?}"),
        }
        assert!(extracted.is_ok(), "inner payload must extract");
    }

    #[test]
    fn erc191_tampered_payload_recovers_a_different_key() {
        // ERC-191 "verification" is RECOVERY: any well-formed signature
        // recovers SOME key, so tampering with the message does not fail
        // verification -- it recovers a different key than the signer's.
        // Account binding is therefore the comparison against the claimed
        // signer, not the recovery itself.
        let good = String::from_utf8(ERC191_VECTOR.to_vec()).expect("utf8");
        let tampered = good.replace("\\\"998\\\"", "\\\"999\\\"");
        // Without this the test passes vacuously: a fixture reformatted or
        // re-valued so the pattern no longer matches would leave `tampered`
        // identical to `good`, and the assertion below would then be checking
        // that an untampered payload recovers the signer's own key.
        assert_ne!(
            tampered, good,
            "the tamper pattern no longer matches the fixture"
        );
        let payloads = decode_args(tampered.as_bytes()).expect("decode");
        let (check, _) = verify_and_extract(&payloads[0]);
        let signer = format!(
            "secp256k1:{}",
            bs58::encode(erc191_test_pubkey64()).into_string()
        );
        match check {
            SignatureCheck::Valid { recovered_key, .. } => assert_ne!(
                recovered_key, signer,
                "a tampered message must not recover the signer's key"
            ),
            // Some tampers land on an unrecoverable point; also acceptable.
            SignatureCheck::Invalid => {}
            other => panic!("expected Valid or Invalid, got {other:?}"),
        }
    }

    fn erc191_with_recovery_id(recovery_id: u8) -> MultiPayload {
        let payloads = decode_args(ERC191_VECTOR).expect("decode");
        let MultiPayload::Erc191(signed) = &payloads[0] else {
            panic!("expected erc191 payload");
        };
        let mut sig = signed.signature;
        sig[64] = recovery_id;
        let bad = serde_json::json!({
            "signed": [{
                "standard": "erc191",
                "payload": signed.payload.0,
                "signature": format!("secp256k1:{}", bs58::encode(sig).into_string()),
            }],
        })
        .to_string();
        decode_args(bad.as_bytes()).expect("decode").remove(0)
    }

    #[test]
    fn erc191_arbitrary_out_of_range_recovery_id_is_malformed_encoding() {
        // v outside {0..3} makes ecrecover fail outright, but this is a
        // malformed-encoding finding, not a proof of tampering -- see
        // erc191_ethereum_v27_recovery_id_is_malformed_encoding_not_invalid.
        let (check, _) = verify_and_extract(&erc191_with_recovery_id(29));
        match check {
            SignatureCheck::MalformedEncoding(reason) => {
                assert!(reason.contains("recovery id 29"), "{reason}");
            }
            other => panic!("expected MalformedEncoding, got {other:?}"),
        }
    }

    #[test]
    fn erc191_ethereum_v27_recovery_id_is_malformed_encoding_not_invalid() {
        // Real wallets (MetaMask) emit v = recovery_id + 27, not this wire
        // format's 0-3 -- see erc191_generator_reproduces_metamask_reference_signature
        // below. The cryptography is fine here; only the encoding convention
        // differs, so this must not read as "signature verification failed"
        // (which implies tampering/fraud on a signing screen).
        let (check, _) = verify_and_extract(&erc191_with_recovery_id(27));
        match check {
            SignatureCheck::MalformedEncoding(reason) => {
                assert!(reason.contains("v=27/28"), "{reason}");
            }
            other => panic!("expected MalformedEncoding, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Real-wallet reference vectors, verbatim from the pinned dependency's own
    // test suites, so provenance is checkable by diffing against those files:
    //   ERC-191: defuse/v0.4.2 erc191/src/lib.rs:72-95 -- "Signature
    //            constructed in Metamask, using private key: a4b3...ca56"
    //   TIP-191: defuse/v0.4.2 tip191/src/lib.rs:83-95 -- the same key,
    //            signed as TRON
    // Both wallets signed with the same private key, hence one shared pubkey.
    // ---------------------------------------------------------------

    const REFERENCE_KEY_HEX: &str =
        "a4b319a82adfc43584e4537fec97a80516e16673db382cd91eba97abbab8ca56";
    const REFERENCE_PUBKEY_HEX: &str = "85a66984273f338ce4ef7b85e5430b008307e8591bb7c1b980852cf6423770b801f41e9438155eb53a5e20f748640093bb42ae3aeca035f7b7fd7a1a21f22f68";

    const METAMASK_MESSAGE: &str = "Hello world!";
    const METAMASK_SIGNATURE_HEX: &str = "7800a70d05cde2c49ed546a6ce887ce6027c2c268c0285f6efef0cdfc4366b23643790f67a86468ee8301ed12cfffcb07c6530f90a9327ec057800fabd332e471c";

    const TRON_MESSAGE: &str = "Hello, TRON!";
    const TRON_SIGNATURE_HEX: &str = "eea1651a60600ec4d9c45e8ae81da1a78377f789f0ac2019de66ad943459913015ef9256809ee0e6bb76e303a0b4802e475c1d26ade5d585292b80c9fe9cb10c1c";

    fn sig65(hex_str: &str) -> [u8; 65] {
        hex::decode(hex_str)
            .expect("valid hex")
            .try_into()
            .expect("65 bytes")
    }

    /// Ethereum and TRON wallets emit `v = recovery_id + 27`; this wire format
    /// expects the raw 0-3 id.
    fn normalize_v(mut sig: [u8; 65]) -> [u8; 65] {
        sig[64] -= 27;
        sig
    }

    /// The bs58 `secp256k1:` form of the shared reference public key, in the
    /// shape `verify_and_extract` reports a recovered key.
    fn reference_recovered_key() -> String {
        let pubkey64: [u8; 64] = hex::decode(REFERENCE_PUBKEY_HEX)
            .expect("valid hex")
            .try_into()
            .expect("64 bytes");
        format!("secp256k1:{}", bs58::encode(pubkey64).into_string())
    }

    /// Wrap a raw `r||s||v` signature and the message it covers as a
    /// single-payload `execute_intents` args blob, so a reference vector runs
    /// the same `decode_args` -> `verify_and_extract` path as production input.
    fn secp256k1_reference_payload(standard: &str, message: &str, sig: [u8; 65]) -> MultiPayload {
        let args = serde_json::json!({
            "signed": [{
                "standard": standard,
                "payload": message,
                "signature": format!("secp256k1:{}", bs58::encode(sig).into_string()),
            }],
        })
        .to_string();
        decode_args(args.as_bytes()).expect("decode").remove(0)
    }

    #[test]
    fn erc191_generator_reproduces_metamask_reference_signature() {
        let key: [u8; 32] = hex::decode(REFERENCE_KEY_HEX)
            .expect("valid hex")
            .try_into()
            .expect("32 bytes");
        let expected = sig65(METAMASK_SIGNATURE_HEX);

        let prehash = [
            format!("\x19Ethereum Signed Message:\n{}", METAMASK_MESSAGE.len()).into_bytes(),
            METAMASK_MESSAGE.as_bytes().to_vec(),
        ]
        .concat();
        let hash = near_sdk::env::keccak256_array(&prehash);
        let sk = k256::ecdsa::SigningKey::from_bytes((&key).into()).expect("valid key");
        let (sig, recovery_id) = sk.sign_prehash_recoverable(&hash).expect("sign");

        let mut produced = [0u8; 65];
        produced[..64].copy_from_slice(&sig.to_bytes());
        // MetaMask/Ethereum's v convention is recovery_id + 27, not the raw
        // 0/1 this crate uses on the wire elsewhere in this module.
        produced[64] = recovery_id.to_byte() + 27;

        assert_eq!(
            produced, expected,
            "generator must reproduce the MetaMask-produced reference signature byte-for-byte"
        );
    }

    #[test]
    fn erc191_metamask_reference_signature_recovers_its_key_end_to_end() {
        // The bytes MetaMask actually produced, through the production
        // verification path -- not just the signing helper that generates the
        // fixture. This is what makes the provenance pin load-bearing.
        let payload = secp256k1_reference_payload(
            "erc191",
            METAMASK_MESSAGE,
            normalize_v(sig65(METAMASK_SIGNATURE_HEX)),
        );
        let (check, extracted) = verify_and_extract(&payload);
        match check {
            SignatureCheck::Valid { recovered_key, .. } => {
                assert_eq!(recovered_key, reference_recovered_key());
            }
            other => panic!("expected a valid signature, got {other:?}"),
        }
        // The signed message is "Hello world!", not an intents envelope.
        assert!(extracted.is_err(), "a bare message must not extract");
    }

    #[test]
    fn erc191_metamask_signature_as_emitted_is_malformed_encoding() {
        // The same real signature with the v byte left as MetaMask emits it
        // (28). Backs the synthetic v=27 case above with an actual wallet's
        // bytes: this is the encoding a real EVM wallet hands over.
        let raw = sig65(METAMASK_SIGNATURE_HEX);
        assert_eq!(raw[64], 28, "reference signature should carry v=28");
        let payload = secp256k1_reference_payload("erc191", METAMASK_MESSAGE, raw);
        match verify_and_extract(&payload).0 {
            SignatureCheck::MalformedEncoding(reason) => {
                assert!(reason.contains("v=27/28"), "{reason}");
            }
            other => panic!("expected MalformedEncoding, got {other:?}"),
        }
    }

    #[test]
    fn tip191_tron_reference_signature_recovers_its_key_end_to_end() {
        // TIP-191 shares the recovery-id guard with ERC-191 but has no other
        // coverage. Same key as the MetaMask vector, signed under TRON's
        // prefix, so a prehash regression in either standard shows up here.
        let payload = secp256k1_reference_payload(
            "tip191",
            TRON_MESSAGE,
            normalize_v(sig65(TRON_SIGNATURE_HEX)),
        );
        let (check, _) = verify_and_extract(&payload);
        match check {
            SignatureCheck::Valid { recovered_key, .. } => {
                assert_eq!(recovered_key, reference_recovered_key());
            }
            other => panic!("expected a valid signature, got {other:?}"),
        }
    }

    #[test]
    fn tip191_tampered_message_recovers_a_different_key() {
        // Recovery, not verification: a well-formed signature over a changed
        // message still recovers SOME key, just not the signer's -- so the
        // assertion is inequality, the same shape as the ERC-191 tamper test.
        // Recovery is also allowed to fail outright on the mutated prehash.
        let payload = secp256k1_reference_payload(
            "tip191",
            "Hello, TRON?",
            normalize_v(sig65(TRON_SIGNATURE_HEX)),
        );
        match verify_and_extract(&payload).0 {
            SignatureCheck::Valid { recovered_key, .. } => {
                assert_ne!(
                    recovered_key,
                    reference_recovered_key(),
                    "a tampered TIP-191 message must not recover the signer's key"
                );
            }
            SignatureCheck::Invalid => {}
            other => panic!("expected a recovery outcome, got {other:?}"),
        }
    }

    #[test]
    fn tip191_shares_the_recovery_id_guard() {
        let payload =
            secp256k1_reference_payload("tip191", TRON_MESSAGE, sig65(TRON_SIGNATURE_HEX));
        match verify_and_extract(&payload).0 {
            SignatureCheck::MalformedEncoding(reason) => {
                assert!(reason.contains("v=27/28"), "{reason}");
            }
            other => panic!("expected MalformedEncoding, got {other:?}"),
        }
    }
}
