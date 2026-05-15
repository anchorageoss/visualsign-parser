//! Verifies that a parse response was signed by a pinned TVC (Turnkey Verifiable
//! Compute) ephemeral key.
//!
//! The gateway sits between an HTTP client (which may be paying via x402) and the
//! parser_app gRPC service. parser_app signs every response with an ephemeral
//! P256 keypair provisioned into the enclave by Turnkey. The gateway must refuse
//! to release the response to the client (and skip x402 settlement) unless the
//! signature verifies against a TVC pubkey that was pinned at the gateway's
//! launch time. The pubkey is provided via env var (`X402_TVC_VERIFIER_PUBKEY_HEX`,
//! or a file path via `X402_TVC_VERIFIER_PUBKEY_FILE`) and matches the same
//! `qos_hex::encode(P256Public::to_bytes())` format parser_app emits in the wire
//! signature.

use generated::parser::{Signature, SignatureScheme};
use qos_p256::P256Public;
use subtle::ConstantTimeEq;

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
}

pub struct AttestationVerifier {
    pinned_public: P256Public,
    pinned_bytes: Vec<u8>,
}

impl AttestationVerifier {
    /// Production entrypoint — reads from the real process environment.
    ///
    /// Returns `Ok(None)` if neither `X402_TVC_VERIFIER_PUBKEY_HEX` nor
    /// `X402_TVC_VERIFIER_PUBKEY_FILE` is set. Callers decide whether absence
    /// is fatal based on profile (production deployments fail closed; local
    /// dev runs without a pinned verifier).
    pub fn from_env() -> Result<Option<Self>, AttestationError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Test-friendly core — takes a closure that resolves env-var lookups so
    /// tests can inject values without mutating process state.
    pub fn from_lookup<F>(get: F) -> Result<Option<Self>, AttestationError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let hex_value = match (
            get("X402_TVC_VERIFIER_PUBKEY_HEX"),
            get("X402_TVC_VERIFIER_PUBKEY_FILE"),
        ) {
            (Some(s), _) => s,
            (None, Some(path)) => std::fs::read_to_string(&path)
                .map_err(|e| AttestationError::PubkeyFile {
                    path: path.clone(),
                    message: e.to_string(),
                })?
                .trim()
                .to_string(),
            (None, None) => return Ok(None),
        };

        Self::from_hex(&hex_value).map(Some)
    }

    pub fn from_hex(hex_value: &str) -> Result<Self, AttestationError> {
        let pinned_bytes =
            qos_hex::decode(hex_value.trim()).map_err(|e| AttestationError::Hex {
                field: "X402_TVC_VERIFIER_PUBKEY_HEX",
                message: format!("{e:?}"),
            })?;
        let pinned_public = P256Public::from_bytes(&pinned_bytes)
            .map_err(|e| AttestationError::InvalidPinnedKey(format!("{e:?}")))?;
        Ok(Self {
            pinned_public,
            pinned_bytes,
        })
    }

    /// Verify that the proto `Signature` on a parse response was produced by the
    /// pinned TVC key.
    pub fn verify(&self, sig: &Signature) -> Result<(), AttestationError> {
        if sig.scheme != SignatureScheme::TurnkeyP256EphemeralKey as i32 {
            let scheme_name = SignatureScheme::from_i32(sig.scheme)
                .map(|s| s.as_str_name().to_string())
                .unwrap_or_else(|| format!("UNKNOWN({})", sig.scheme));
            return Err(AttestationError::UnsupportedScheme(scheme_name));
        }

        let response_bytes =
            qos_hex::decode(&sig.public_key).map_err(|e| AttestationError::Hex {
                field: "signature.public_key",
                message: format!("{e:?}"),
            })?;
        if response_bytes.len() != self.pinned_bytes.len()
            || response_bytes
                .ct_eq(self.pinned_bytes.as_slice())
                .unwrap_u8()
                != 1
        {
            return Err(AttestationError::PubkeyMismatch);
        }

        let digest = qos_hex::decode(&sig.message).map_err(|e| AttestationError::Hex {
            field: "signature.message",
            message: format!("{e:?}"),
        })?;
        let signature_bytes =
            qos_hex::decode(&sig.signature).map_err(|e| AttestationError::Hex {
                field: "signature.signature",
                message: format!("{e:?}"),
            })?;

        self.pinned_public
            .verify(&digest, &signature_bytes)
            .map_err(|_| AttestationError::Verify)
    }

    /// Hex representation of the pinned key. Useful for log/error messages.
    pub fn pinned_hex(&self) -> String {
        qos_hex::encode(&self.pinned_bytes)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use generated::parser::{ParsedTransactionPayload, Signature, SignatureScheme};
    use qos_crypto::sha_256;
    use qos_p256::P256Pair;

    fn make_signed_response(pair: &P256Pair) -> Signature {
        let payload = ParsedTransactionPayload {
            parsed_payload: "{}".to_string(),
            input_payload_digest: String::new(),
            metadata_digest: String::new(),
            signable_payload: "{}".to_string(),
        };
        let body = borsh::to_vec(&payload).unwrap();
        let digest = sha_256(&body);
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
    fn round_trip_verifies_real_signature() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let sig = make_signed_response(&pair);
        verifier
            .verify(&sig)
            .expect("legitimate signature must verify");
    }

    #[test]
    fn rejects_mismatched_pubkey() {
        let pair_a = P256Pair::generate().unwrap();
        let pair_b = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair_a.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let sig = make_signed_response(&pair_b);
        assert!(matches!(
            verifier.verify(&sig).unwrap_err(),
            AttestationError::PubkeyMismatch
        ));
    }

    #[test]
    fn rejects_tampered_signature_bytes() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let mut sig = make_signed_response(&pair);
        let mut chars: Vec<char> = sig.signature.chars().collect();
        let last_idx = chars.len() - 1;
        chars[last_idx] = if chars[last_idx] == '0' { '1' } else { '0' };
        sig.signature = chars.into_iter().collect();
        assert!(matches!(
            verifier.verify(&sig).unwrap_err(),
            AttestationError::Verify
        ));
    }

    #[test]
    fn rejects_unsupported_scheme() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex).unwrap();
        let mut sig = make_signed_response(&pair);
        sig.scheme = SignatureScheme::Unspecified as i32;
        assert!(matches!(
            verifier.verify(&sig).unwrap_err(),
            AttestationError::UnsupportedScheme(_)
        ));
    }

    #[test]
    fn pubkey_compare_is_case_insensitive() {
        let pair = P256Pair::generate().unwrap();
        let pinned_hex = qos_hex::encode(&pair.public_key().to_bytes());
        let verifier = AttestationVerifier::from_hex(&pinned_hex.to_uppercase()).unwrap();
        let sig = make_signed_response(&pair);
        verifier.verify(&sig).expect("hex case must not matter");
    }
}
