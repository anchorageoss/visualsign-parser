//! Gateway P256 signing identity for VerifiedPaymentMarkers.
//!
//! The gateway holds a P256 keypair (its identity in the TVC trust pair) and
//! signs every VPM it hands to parser_app. parser_app verifies against the
//! pinned `GATEWAY_SIGNING_PUBKEY_HEX`.
//!
//! Key source: `GATEWAY_SIGNING_KEY_FILE` — JSON `{"private": "<hex>", "public": "<hex>"}`.
//! Single file = single secret mount, the same shape as the demo's
//! `TVC_API_KEY_FILE` was designed around (Cloud Run / k8s Secret volume
//! friendly).
//!
//! The private hex must decode to a 32-byte P256 scalar; the public hex must
//! match the corresponding SEC1-uncompressed encoding (65 bytes, `0x04 || X || Y`).
//! The startup loader checks both consistency conditions.

use host_primitives::payment_marker::{SignedVerifiedPaymentMarker, VerifiedPaymentMarker};
use qos_p256::sign::{P256SignPair, P256SignPublic};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("GATEWAY_SIGNING_KEY_FILE not set")]
    MissingEnv,
    #[error("failed to read {path}: {message}")]
    ReadFile { path: String, message: String },
    #[error("failed to parse signing key file: {0}")]
    Parse(String),
    #[error("hex decode error in {field}: {message}")]
    Hex {
        field: &'static str,
        message: String,
    },
    #[error("invalid private scalar: {0}")]
    Private(String),
    #[error("invalid public point: {0}")]
    Public(String),
    #[error("public/private mismatch: derived pubkey != provided pubkey")]
    Mismatch,
    #[error("sign error: {0}")]
    Sign(String),
}

#[derive(Deserialize)]
struct KeyFile {
    private: String,
    public: String,
}

pub struct GatewaySigner {
    pair: P256SignPair,
    public_hex_lower: String,
}

impl GatewaySigner {
    pub fn from_env() -> Result<Self, SigningError> {
        let path =
            std::env::var("GATEWAY_SIGNING_KEY_FILE").map_err(|_| SigningError::MissingEnv)?;
        Self::from_file(Path::new(&path))
    }

    pub fn from_file(path: &Path) -> Result<Self, SigningError> {
        let raw = std::fs::read_to_string(path).map_err(|e| SigningError::ReadFile {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        let parsed: KeyFile =
            serde_json::from_str(&raw).map_err(|e| SigningError::Parse(e.to_string()))?;
        Self::from_hex(&parsed.private, &parsed.public)
    }

    pub fn from_hex(private_hex: &str, public_hex: &str) -> Result<Self, SigningError> {
        let priv_bytes = qos_hex::decode(private_hex.trim()).map_err(|e| SigningError::Hex {
            field: "private",
            message: format!("{e:?}"),
        })?;
        let pub_bytes = qos_hex::decode(public_hex.trim()).map_err(|e| SigningError::Hex {
            field: "public",
            message: format!("{e:?}"),
        })?;

        let pair = P256SignPair::from_bytes(&priv_bytes)
            .map_err(|e| SigningError::Private(format!("{e:?}")))?;
        let derived = pair.public_key();
        let provided = P256SignPublic::from_bytes(&pub_bytes)
            .map_err(|e| SigningError::Public(format!("{e:?}")))?;

        if derived.to_bytes() != provided.to_bytes() {
            return Err(SigningError::Mismatch);
        }

        Ok(Self {
            pair,
            public_hex_lower: public_hex.trim().to_ascii_lowercase(),
        })
    }

    /// Hex of the SEC1-uncompressed P256 sign public key. Lower-cased.
    /// This is what parser_app pins via `GATEWAY_SIGNING_PUBKEY_HEX`.
    pub fn public_hex(&self) -> &str {
        &self.public_hex_lower
    }

    /// Sign a VPM, returning the borsh-serializable bundle parser_app
    /// expects in `ParseRequest.payment_marker`.
    pub fn sign(
        &self,
        vpm: VerifiedPaymentMarker,
    ) -> Result<SignedVerifiedPaymentMarker, SigningError> {
        let digest = vpm.signing_digest();
        let signature = self
            .pair
            .sign(&digest)
            .map_err(|e| SigningError::Sign(format!("{e:?}")))?;
        Ok(SignedVerifiedPaymentMarker { vpm, signature })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use host_primitives::payment_marker::VPM_VERSION;

    fn sample_vpm() -> VerifiedPaymentMarker {
        VerifiedPaymentMarker {
            version: VPM_VERSION,
            request_hash: [3u8; 32],
            txid: "txsig".into(),
            payer: "Pay".into(),
            pay_to: "Recv".into(),
            amount: "1000".into(),
            mint: "Mint".into(),
            x_payment_hash: [4u8; 32],
            network: "solana:test".into(),
            settled_at_ms: 1,
            gateway_pubkey_hex: String::new(),
        }
    }

    fn fresh_signer() -> GatewaySigner {
        let pair = P256SignPair::generate();
        let priv_hex = qos_hex::encode(&pair.to_bytes());
        let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());
        GatewaySigner::from_hex(&priv_hex, &pub_hex).unwrap()
    }

    #[test]
    fn round_trip_sign_then_verify() {
        let signer = fresh_signer();
        let pub_bytes = qos_hex::decode(signer.public_hex()).unwrap();
        let pubkey = P256SignPublic::from_bytes(&pub_bytes).unwrap();

        let signed = signer.sign(sample_vpm()).unwrap();
        let digest = signed.vpm.signing_digest();
        pubkey.verify(&digest, &signed.signature).unwrap();
    }

    #[test]
    fn rejects_pub_priv_mismatch() {
        let pair_a = P256SignPair::generate();
        let pair_b = P256SignPair::generate();
        let priv_hex = qos_hex::encode(&pair_a.to_bytes());
        let pub_hex = qos_hex::encode(&pair_b.public_key().to_bytes());
        let res = GatewaySigner::from_hex(&priv_hex, &pub_hex);
        match res {
            Err(SigningError::Mismatch) => {}
            Err(e) => panic!("expected Mismatch, got {e:?}"),
            Ok(_) => panic!("expected Mismatch, got Ok"),
        }
    }
}
