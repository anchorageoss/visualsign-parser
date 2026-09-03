//! **Demo-only response-signature verification.** The gateway pins one
//! `qos_p256::P256Public` value at boot via env var and rejects any parse
//! response whose `Signature.public_key` doesn't match.
//!
//! This is NOT a production attestation flow. It does not parse or validate
//! an AWS Nitro / Intel TDX attestation document; it does not check PCRs; it
//! does not verify Turnkey operator signatures over the deploy manifest. It
//! assumes someone else (a TVC operator, an ops engineer, the demo playbook)
//! put a trustworthy pubkey hex in the env var.
//!
//! ## Production replacement
//!
//! In a real Turnkey TVC deployment, replace this with the real attestation
//! chain. The Turnkey Rust SDK already exposes the validator:
//!
//! - `tkhq/rust-sdk` -> `proofs::parse_and_verify_aws_nitro_attestation`
//!   <https://github.com/tkhq/rust-sdk/blob/373fed6/proofs/src/lib.rs#L298>
//!
//! Sketch of the production path:
//! 1. parser_app exposes its Nitro attestation document via a new
//!    `GetAttestation` gRPC method (it already holds one from QOS boot).
//! 2. Gateway at startup fetches the doc, calls
//!    `parse_and_verify_aws_nitro_attestation(doc, expected_pcrs)`, and
//!    extracts the embedded ephemeral pubkey from the returned struct.
//! 3. That extracted pubkey is what gets used for per-response P256 verify
//!    -- same wire path as today, just sourced from attestation instead of
//!    an env var.
//!
//! Until that lands, this module's `from_env()` is the demo crutch.
//!
//! ## Env vars (demo only)
//!
//! - `TVC_DEMO_PINNED_PUBKEY_HEX` -- hex of `P256Public::to_bytes()`.
//! - `TVC_DEMO_PINNED_PUBKEY_FILE` -- file containing the same hex.
//!
//! The hex is the qos_p256 compound key (encrypt half || sign half, each
//! SEC1 uncompressed) -- 130 bytes / 260 hex chars. This is NOT a Solana
//! base58 address; the two share the word "pubkey" but live in different
//! namespaces.

use borsh::BorshSerialize;
use generated::parser::{ParsedTransactionPayload, Signature, SignatureScheme};
use qos_crypto::sha_256;
use qos_p256::P256Public;

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("unsupported signature scheme: {0}")]
    UnsupportedScheme(String),
    #[error("public key mismatch: response key does not match pinned TVC verifier key")]
    PubkeyMismatch,
    #[error("hex decode error in {field}: {message}")]
    Hex {
        field: &'static str,
        message: String,
    },
    #[error("invalid pinned TVC public key: {0}")]
    InvalidPinnedKey(String),
    #[error("signature verification failed")]
    Verify,
    #[error("failed to read TVC pubkey file {path}: {message}")]
    PubkeyFile { path: String, message: String },
    #[error("TVC pubkey file {path} exceeds maximum size ({max} bytes)")]
    PubkeyFileTooLarge { path: String, max: u64 },
    #[error("failed to serialize payload for digest recomputation: {0}")]
    DigestSerialization(String),
    #[error("both TVC_DEMO_PINNED_PUBKEY_HEX and TVC_DEMO_PINNED_PUBKEY_FILE are set; choose one")]
    BothSet,
    #[error("{var} contains invalid (non-UTF-8) bytes")]
    NotUnicode { var: &'static str },
}

/// Maximum allowed size for the pinned-pubkey file (bytes). The hex payload
/// is a fixed 260 characters (130-byte qos_p256 compound key); this leaves
/// generous room for an optional prefix/surrounding whitespace while still
/// bounding the read against a mistaken path to a very large file or
/// character device.
const MAX_PUBKEY_FILE_SIZE: u64 = 4096;

/// Recompute the bytes the TVC ephemeral key signs over for a
/// `ParsedTransactionPayload`.
///
/// This mirrors `parser/app/src/routes/parse.rs::signing_digest_bytes`
/// byte-for-byte; keep the two in sync. `verify()` uses this to recompute the
/// signed digest from the payload actually being forwarded, rather than
/// trusting the wire-carried `Signature.message` verbatim -- otherwise a
/// stale-but-validly-signed `(message, signature, public_key)` tuple could be
/// replayed against an attacker-substituted payload.
fn signing_digest_bytes(payload: &ParsedTransactionPayload) -> Result<Vec<u8>, AttestationError> {
    let mut bytes = Vec::new();
    payload
        .serialize(&mut bytes)
        .map_err(|e| AttestationError::DigestSerialization(e.to_string()))?;
    if !payload.intermediate_output.is_empty() {
        payload
            .intermediate_output
            .serialize(&mut bytes)
            .map_err(|e| AttestationError::DigestSerialization(e.to_string()))?;
    }
    Ok(bytes)
}

pub struct AttestationVerifier {
    pinned_public: P256Public,
}

impl AttestationVerifier {
    /// Production entrypoint -- reads from the real process environment.
    ///
    /// Returns `Ok(None)` if neither `TVC_DEMO_PINNED_PUBKEY_HEX` nor
    /// `TVC_DEMO_PINNED_PUBKEY_FILE` is set. Callers decide whether absence
    /// is fatal based on profile (production deployments fail closed; local
    /// dev runs without a pinned verifier).
    pub fn from_env() -> Result<Option<Self>, AttestationError> {
        // Distinguish "unset" from "set but not valid UTF-8" (see
        // `crate::env_util::checked_env_var`): `std::env::var(..).ok()`
        // collapses both into `None`, which would silently disable
        // verification for a malformed pinned-key env var instead of
        // reporting invalid configuration.
        let hex_value = Self::checked_env_var("TVC_DEMO_PINNED_PUBKEY_HEX")?;
        let file_path = Self::checked_env_var("TVC_DEMO_PINNED_PUBKEY_FILE")?;
        Self::from_lookup(|key| match key {
            "TVC_DEMO_PINNED_PUBKEY_HEX" => hex_value.clone(),
            "TVC_DEMO_PINNED_PUBKEY_FILE" => file_path.clone(),
            _ => None,
        })
    }

    /// Reads an env var, distinguishing "unset" from "set but not valid
    /// UTF-8". See `crate::env_util::checked_env_var`.
    fn checked_env_var(key: &'static str) -> Result<Option<String>, AttestationError> {
        crate::env_util::checked_env_var(key, |var| AttestationError::NotUnicode { var })
    }

    /// Test-friendly core -- takes a closure that resolves env-var lookups so
    /// tests can inject values without mutating process state.
    pub fn from_lookup<F>(get: F) -> Result<Option<Self>, AttestationError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let (hex_value, source) = match (
            get("TVC_DEMO_PINNED_PUBKEY_HEX"),
            get("TVC_DEMO_PINNED_PUBKEY_FILE"),
        ) {
            (Some(_), Some(_)) => return Err(AttestationError::BothSet),
            (Some(s), None) => (s, "TVC_DEMO_PINNED_PUBKEY_HEX"),
            (None, Some(path)) => (
                Self::read_pubkey_file(&path)?.trim().to_string(),
                "TVC_DEMO_PINNED_PUBKEY_FILE",
            ),
            (None, None) => return Ok(None),
        };

        Self::from_hex_with_source(&hex_value, source).map(Some)
    }

    /// Reads `TVC_DEMO_PINNED_PUBKEY_FILE` with a bounded reader, matching
    /// the repository's file-input convention (`mapping_parser.rs`,
    /// `tx_input.rs`): a mistaken path to a very large file or character
    /// device must not exhaust memory or hang startup. See
    /// `crate::env_util::read_bounded_file`.
    fn read_pubkey_file(path: &str) -> Result<String, AttestationError> {
        crate::env_util::read_bounded_file(
            path,
            MAX_PUBKEY_FILE_SIZE,
            |path, message| AttestationError::PubkeyFile { path, message },
            |path, max| AttestationError::PubkeyFileTooLarge { path, max },
        )
    }

    pub fn from_hex(hex_value: &str) -> Result<Self, AttestationError> {
        Self::from_hex_with_source(hex_value, "TVC_DEMO_PINNED_PUBKEY_HEX")
    }

    /// Core hex-decode path, parameterized by which env var the hex actually
    /// came from so a decode failure names the input the operator set (not
    /// always `_HEX`, e.g. when the value was read out of `_FILE`).
    fn from_hex_with_source(
        hex_value: &str,
        source: &'static str,
    ) -> Result<Self, AttestationError> {
        // Operator-supplied hex (env var or file), not the wire-carried
        // signature fields verify() decodes below -- route it through the
        // repository's unified decoder so an uppercase `0X` prefix isn't
        // silently rejected (qos_hex::decode only strips lowercase `0x`).
        let pinned_bytes = visualsign::encodings::decode_hex(hex_value.trim()).map_err(|e| {
            AttestationError::Hex {
                field: source,
                message: format!("{e:?}"),
            }
        })?;
        let pinned_public = P256Public::from_bytes(&pinned_bytes)
            .map_err(|e| AttestationError::InvalidPinnedKey(format!("{e:?}")))?;
        Ok(Self { pinned_public })
    }

    /// Verify that the proto `Signature` on a parse response was produced by
    /// the pinned TVC key over exactly this `payload` -- the one actually
    /// being forwarded to the caller.
    ///
    /// The digest is recomputed from `payload` (see `signing_digest_bytes`)
    /// rather than trusted from `sig.message`, so a signature captured from a
    /// prior legitimate response can't be paired with a different,
    /// attacker-controlled payload and still verify.
    ///
    /// On success, returns the recomputed digest that was actually
    /// authenticated. Callers MUST forward this value (not `sig.message`) to
    /// clients: `sig.message` is wire-carried and unverified, so a
    /// compromised gRPC hop could tamper with it alone -- verification here
    /// would still pass (it never reads `sig.message`), but a client that
    /// checks the signature against the tampered `message` field would
    /// reject a response it already paid for.
    pub fn verify(
        &self,
        sig: &Signature,
        payload: &ParsedTransactionPayload,
    ) -> Result<[u8; 32], AttestationError> {
        if sig.scheme != SignatureScheme::TurnkeyP256EphemeralKey as i32 {
            // The generated `SignatureScheme` (prost 0.11-style enum) has no
            // `from_i32` helper on main, unlike newer prost releases. Only
            // two variants exist, so name the known one and fall back to the
            // raw int for anything else.
            let scheme_name = if sig.scheme == SignatureScheme::Unspecified as i32 {
                SignatureScheme::Unspecified.as_str_name().to_string()
            } else {
                format!("UNKNOWN({})", sig.scheme)
            };
            return Err(AttestationError::UnsupportedScheme(scheme_name));
        }

        let response_bytes =
            qos_hex::decode(&sig.public_key).map_err(|e| AttestationError::Hex {
                field: "signature.public_key",
                message: format!("{e:?}"),
            })?;
        let response_public = P256Public::from_bytes(&response_bytes)
            .map_err(|_| AttestationError::PubkeyMismatch)?;
        if response_public != self.pinned_public {
            return Err(AttestationError::PubkeyMismatch);
        }

        let expected_digest = sha_256(&signing_digest_bytes(payload)?);
        let signature_bytes =
            qos_hex::decode(&sig.signature).map_err(|e| AttestationError::Hex {
                field: "signature.signature",
                message: format!("{e:?}"),
            })?;

        self.pinned_public
            .verify(&expected_digest, &signature_bytes)
            .map_err(|_| AttestationError::Verify)?;

        Ok(expected_digest)
    }

    /// Hex representation of the pinned key. Useful for log/error messages.
    pub fn pinned_hex(&self) -> String {
        qos_hex::encode(&self.pinned_public.to_bytes())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::SignatureScheme;
    use qos_p256::P256Pair;

    fn sample_payload(parsed_payload: &str) -> ParsedTransactionPayload {
        ParsedTransactionPayload {
            parsed_payload: parsed_payload.to_string(),
            input_payload_digest: String::new(),
            metadata_digest: String::new(),
            signable_payload: "{}".to_string(),
            intermediate_output: Vec::new(),
        }
    }

    fn make_signed_response(pair: &P256Pair, payload: &ParsedTransactionPayload) -> Signature {
        let digest = sha_256(&signing_digest_bytes(payload).unwrap());
        let sig_bytes = pair.sign(&digest).unwrap();
        Signature {
            public_key: qos_hex::encode(&pair.public_key().to_bytes()),
            signature: qos_hex::encode(&sig_bytes),
            message: qos_hex::encode(&digest),
            scheme: SignatureScheme::TurnkeyP256EphemeralKey as i32,
        }
    }

    #[test]
    fn from_lookup_absent_returns_none() {
        let v = AttestationVerifier::from_lookup(|_| None).unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn from_lookup_both_set_errors() {
        let res = AttestationVerifier::from_lookup(|key| match key {
            "TVC_DEMO_PINNED_PUBKEY_HEX" => Some("aa".to_string()),
            "TVC_DEMO_PINNED_PUBKEY_FILE" => Some("/nonexistent".to_string()),
            _ => None,
        });
        assert!(matches!(res, Err(AttestationError::BothSet)));
    }

    #[test]
    fn from_lookup_reads_file_source() {
        let pair = P256Pair::generate().unwrap();
        let hex = qos_hex::encode(&pair.public_key().to_bytes());
        let path = std::env::temp_dir().join(format!(
            "attestation-test-pubkey-{}-{}.hex",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, &hex).unwrap();
        let path_str = path.to_string_lossy().to_string();

        let v = AttestationVerifier::from_lookup(move |key| {
            if key == "TVC_DEMO_PINNED_PUBKEY_FILE" {
                Some(path_str.clone())
            } else {
                None
            }
        })
        .unwrap();
        std::fs::remove_file(&path).ok();
        assert!(v.is_some());
    }

    #[test]
    fn from_lookup_file_source_names_file_var_on_bad_hex() {
        let path = std::env::temp_dir().join(format!(
            "attestation-test-badhex-{}-{}.hex",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, "not-hex!!").unwrap();
        let path_str = path.to_string_lossy().to_string();

        let res = AttestationVerifier::from_lookup(move |key| {
            if key == "TVC_DEMO_PINNED_PUBKEY_FILE" {
                Some(path_str.clone())
            } else {
                None
            }
        });
        std::fs::remove_file(&path).ok();
        assert!(matches!(
            res,
            Err(AttestationError::Hex {
                field: "TVC_DEMO_PINNED_PUBKEY_FILE",
                ..
            })
        ));
    }

    #[test]
    fn from_lookup_file_too_large_rejected() {
        // A pinned-key file larger than MAX_PUBKEY_FILE_SIZE must be
        // rejected rather than read in full, regardless of its contents.
        let path = std::env::temp_dir().join(format!(
            "attestation-test-oversized-{}-{}.hex",
            std::process::id(),
            line!()
        ));
        let oversized = "a".repeat((MAX_PUBKEY_FILE_SIZE + 1) as usize);
        std::fs::write(&path, &oversized).unwrap();
        let path_str = path.to_string_lossy().to_string();

        let res = AttestationVerifier::from_lookup(move |key| {
            if key == "TVC_DEMO_PINNED_PUBKEY_FILE" {
                Some(path_str.clone())
            } else {
                None
            }
        });
        std::fs::remove_file(&path).ok();
        assert!(matches!(
            res,
            Err(AttestationError::PubkeyFileTooLarge { .. })
        ));
    }

    #[test]
    fn from_hex_invalid_key_bytes_rejected() {
        // Valid hex, but too short to be a qos_p256 compound key.
        let res = AttestationVerifier::from_hex("00112233");
        assert!(matches!(res, Err(AttestationError::InvalidPinnedKey(_))));
    }

    #[test]
    fn from_hex_accepts_uppercase_0x_prefix() {
        let pair = P256Pair::generate().unwrap();
        let hex = qos_hex::encode(&pair.public_key().to_bytes());
        let uppercase_prefixed = format!("0X{hex}");
        let verifier = AttestationVerifier::from_hex(&uppercase_prefixed)
            .expect("uppercase 0X prefix must be tolerated, same as the lowercase 0x form");
        assert_eq!(verifier.pinned_hex(), hex);
    }

    #[test]
    fn round_trip_verifies_real_signature() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let payload = sample_payload("{}");
        let sig = make_signed_response(&pair, &payload);
        verifier
            .verify(&sig, &payload)
            .expect("legitimate signature must verify");
    }

    #[test]
    fn rejects_mismatched_pubkey() {
        let pair_a = P256Pair::generate().unwrap();
        let pair_b = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair_a.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let payload = sample_payload("{}");
        let sig = make_signed_response(&pair_b, &payload);
        assert!(matches!(
            verifier.verify(&sig, &payload).unwrap_err(),
            AttestationError::PubkeyMismatch
        ));
    }

    #[test]
    fn rejects_tampered_signature_bytes() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let payload = sample_payload("{}");
        let mut sig = make_signed_response(&pair, &payload);
        let mut chars: Vec<char> = sig.signature.chars().collect();
        let last_idx = chars.len() - 1;
        chars[last_idx] = if chars[last_idx] == '0' { '1' } else { '0' };
        sig.signature = chars.into_iter().collect();
        assert!(matches!(
            verifier.verify(&sig, &payload).unwrap_err(),
            AttestationError::Verify
        ));
    }

    #[test]
    fn rejects_unsupported_scheme() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let payload = sample_payload("{}");
        let mut sig = make_signed_response(&pair, &payload);
        sig.scheme = SignatureScheme::Unspecified as i32;
        assert!(matches!(
            verifier.verify(&sig, &payload).unwrap_err(),
            AttestationError::UnsupportedScheme(_)
        ));
    }

    #[test]
    fn pubkey_compare_is_case_insensitive() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex.to_uppercase()).unwrap();
        let payload = sample_payload("{}");
        let sig = make_signed_response(&pair, &payload);
        verifier
            .verify(&sig, &payload)
            .expect("hex case must not matter");
    }

    #[test]
    fn verify_returns_authenticated_digest_ignoring_tampered_wire_message() {
        // Regression for the gap where `parse_handler` forwarded
        // `sig.message` verbatim: `verify()` never reads `sig.message`, so a
        // compromised hop could tamper with only that field and verification
        // would still pass. Callers must use the digest `verify()` returns
        // (the value actually authenticated), not the untrusted wire field.
        // This test proves that returned digest matches the real signed
        // digest and diverges from a tampered `sig.message`, i.e. a caller
        // that forwards the returned digest (as `parse_handler` now does)
        // never forwards the tampered value unchanged.
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let payload = sample_payload("{\"amount\":\"1\"}");
        let mut sig = make_signed_response(&pair, &payload);

        // Tamper with only `message`; signature and public_key are untouched.
        let real_digest = sha_256(&signing_digest_bytes(&payload).unwrap());
        sig.message = qos_hex::encode(b"attacker-controlled-message");

        let authenticated_digest = verifier
            .verify(&sig, &payload)
            .expect("verification does not read sig.message, so it still passes");
        assert_eq!(
            authenticated_digest, real_digest,
            "returned digest must be the value actually signed over"
        );
        assert_ne!(
            qos_hex::encode(&authenticated_digest),
            sig.message,
            "returned digest must diverge from the tampered wire message"
        );
    }

    #[test]
    fn rejects_stale_signature_replayed_against_substituted_payload() {
        // The core fix under test: a validly-signed (message, signature,
        // public_key) tuple for one payload must NOT verify against a
        // different payload, even though the tuple was produced by the
        // pinned key. Regression test for the digest-binding gap.
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let original_payload = sample_payload("{\"amount\":\"1\"}");
        let sig = make_signed_response(&pair, &original_payload);
        let substituted_payload = sample_payload("{\"amount\":\"1000000\"}");
        assert!(matches!(
            verifier.verify(&sig, &substituted_payload).unwrap_err(),
            AttestationError::Verify
        ));
    }
}
