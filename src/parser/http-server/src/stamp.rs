//! Validates the `X-Stamp` header Turnkey's stamper attaches to a request.
//!
//! The stamp covers the raw request body only: no timestamp, no nonce, no
//! method or path. Verification must run against the exact bytes the client
//! sent, never a re-serialized form (see `handle_parse` in `main.rs` and the
//! `signature_is_checked_against_raw_bytes_not_reserialized_json` test below).

use axum::http::HeaderMap;
use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// Header and scheme names are fixed by Turnkey's stamper
/// (turnkey_api_key_stamper 0.10: API_KEY_STAMP_HEADER_NAME,
/// SIGNATURE_SCHEME_P256, SIGNATURE_SCHEME_SECP256K1).
const STAMP_HEADER: &str = "X-Stamp";
const SCHEME_P256: &str = "SIGNATURE_SCHEME_TK_API_P256";
const SCHEME_SECP256K1: &str = "SIGNATURE_SCHEME_TK_API_SECP256K1";

/// `Malformed` and `UnsupportedScheme` carry context read only via `{e:?}`
/// in `main.rs`'s deliberately coarse client-facing error (see `verify`'s
/// callsite); same pattern as `boot_proof::BootProofError`.
#[derive(Debug)]
#[allow(dead_code)]
pub enum StampError {
    Missing,
    Malformed(String),
    UnsupportedScheme(String),
    UnknownKey,
    BadSignature,
}

/// Wire form of the header value: base64url-no-pad JSON.
///
/// Deliberately NOT `deny_unknown_fields`: the producer is Turnkey's stamper,
/// and a field added on their side would otherwise fail every request. The
/// three fields we read are the ones the signature scheme is defined over.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiStamp {
    public_key: String,
    signature: String,
    scheme: String,
}

/// Compressed SEC1 pubkeys permitted to call the parse routes. Delivered via
/// `pivotArgs` at deploy time, the same mechanism that pins the gateway signing
/// key, so rotation costs a redeploy but no rebuild. A signed allowlist
/// document (option c in PRS-581) is the follow-up if that hurts.
pub struct Allowlist {
    keys: Vec<Vec<u8>>,
}

impl Allowlist {
    pub fn from_hex_list(csv: &str) -> Result<Self, StampError> {
        let mut keys = Vec::new();
        for entry in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let bytes = visualsign::encodings::decode_hex(entry)
                .map_err(|e| StampError::Malformed(format!("allowlist entry {entry}: {e}")))?;
            if bytes.len() != 33 {
                return Err(StampError::Malformed(format!(
                    "allowlist entry {entry} is {} bytes, expected 33 (compressed SEC1)",
                    bytes.len()
                )));
            }
            keys.push(bytes);
        }
        if keys.is_empty() {
            return Err(StampError::Malformed("allowlist is empty".to_string()));
        }
        Ok(Self { keys })
    }

    /// Constant-time membership: a timing signal here would leak which keys are
    /// allowlisted. Same posture as parser_gateway's auth/attestation compares.
    fn contains(&self, candidate: &[u8]) -> bool {
        let mut found = subtle::Choice::from(0u8);
        for key in &self.keys {
            found |= key.as_slice().ct_eq(candidate);
        }
        found.into()
    }
}

/// Verify the `X-Stamp` header against the **raw** request bytes.
///
/// The stamp covers the body only: no timestamp, no nonce, no method or path.
/// It is replayable by design, which is acceptable for a stateless read-only
/// parse. Combined with x402, a replayed body plus its original VPM is a free
/// re-parse of the same transaction; the VPM commits to request_hash, so it
/// cannot be redirected at a different one.
pub fn verify(headers: &HeaderMap, body: &[u8], allowlist: &Allowlist) -> Result<(), StampError> {
    let raw = headers.get(STAMP_HEADER).ok_or(StampError::Missing)?;
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .map_err(|e| StampError::Malformed(format!("base64url: {e}")))?;
    let stamp: ApiStamp = serde_json::from_slice(&decoded)
        .map_err(|e| StampError::Malformed(format!("stamp json: {e}")))?;

    let pubkey = visualsign::encodings::decode_hex(&stamp.public_key)
        .map_err(|e| StampError::Malformed(format!("publicKey hex: {e}")))?;
    if !allowlist.contains(&pubkey) {
        return Err(StampError::UnknownKey);
    }
    let sig_der = visualsign::encodings::decode_hex(&stamp.signature)
        .map_err(|e| StampError::Malformed(format!("signature hex: {e}")))?;

    match stamp.scheme.as_str() {
        SCHEME_P256 => {
            use p256::ecdsa::{DerSignature, VerifyingKey, signature::Verifier};
            let key = VerifyingKey::from_sec1_bytes(&pubkey)
                .map_err(|e| StampError::Malformed(format!("p256 pubkey: {e}")))?;
            let sig = DerSignature::from_bytes(&sig_der)
                .map_err(|e| StampError::Malformed(format!("p256 der: {e}")))?;
            key.verify(body, &sig).map_err(|_| StampError::BadSignature)
        }
        SCHEME_SECP256K1 => {
            use k256::ecdsa::{DerSignature, VerifyingKey, signature::Verifier};
            let key = VerifyingKey::from_sec1_bytes(&pubkey)
                .map_err(|e| StampError::Malformed(format!("k256 pubkey: {e}")))?;
            let sig = DerSignature::from_bytes(&sig_der)
                .map_err(|e| StampError::Malformed(format!("k256 der: {e}")))?;
            key.verify(body, &sig).map_err(|_| StampError::BadSignature)
        }
        other => Err(StampError::UnsupportedScheme(other.to_string())),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use turnkey_api_key_stamper::{Stamp, TurnkeyP256ApiKey};

    fn headers_for(key: &TurnkeyP256ApiKey, body: &[u8]) -> HeaderMap {
        let stamp = key.stamp(body).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-Stamp", HeaderValue::from_str(&stamp.value).unwrap());
        headers
    }

    #[test]
    fn accepts_a_stamp_from_an_allowlisted_key() {
        let key = TurnkeyP256ApiKey::generate();
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap();
        let body = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02"}}"#;
        verify(&headers_for(&key, body), body, &allowlist).unwrap();
    }

    #[test]
    fn rejects_a_stamp_from_an_unlisted_key() {
        let signer = TurnkeyP256ApiKey::generate();
        let other = TurnkeyP256ApiKey::generate();
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(other.compressed_public_key())).unwrap();
        let body = br#"{"request":{}}"#;
        let err = verify(&headers_for(&signer, body), body, &allowlist).unwrap_err();
        assert!(matches!(err, StampError::UnknownKey));
    }

    #[test]
    fn signature_is_checked_against_raw_bytes_not_reserialized_json() {
        // The acceptance criterion for this PR. The stamp is signed over the
        // exact bytes; serde's output differs in key order and whitespace, so
        // verifying the re-serialized form must fail.
        let key = TurnkeyP256ApiKey::generate();
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap();
        // `serde_json::Value` preserves key insertion order in this workspace
        // (some dependency turns on serde_json's `preserve_order` feature,
        // and Cargo unifies it for every user of the crate), so a re-ordered
        // fixture round-trips byte-identical and would make this test pass
        // for the wrong reason. The whitespace this fixture adds is the part
        // `serde_json::to_vec`'s compact output always drops, so the
        // round-trip is guaranteed to differ regardless of that feature.
        let raw = br#"{"request": {"chain":"CHAIN_ETHEREUM", "unsigned_payload": "0x02"}}"#;
        let headers = headers_for(&key, raw);

        verify(&headers, raw, &allowlist).unwrap();

        let value: serde_json::Value = serde_json::from_slice(raw).unwrap();
        let reserialized = serde_json::to_vec(&value).unwrap();
        assert_ne!(
            reserialized.as_slice(),
            raw.as_slice(),
            "fixture must actually differ"
        );
        let err = verify(&headers, &reserialized, &allowlist).unwrap_err();
        assert!(matches!(err, StampError::BadSignature));
    }

    #[test]
    fn rejects_missing_header_and_malformed_encodings() {
        let key = TurnkeyP256ApiKey::generate();
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap();
        let body = br#"{}"#;
        assert!(matches!(
            verify(&HeaderMap::new(), body, &allowlist),
            Err(StampError::Missing)
        ));

        let mut bad_b64 = HeaderMap::new();
        bad_b64.insert("X-Stamp", HeaderValue::from_static("!!!not-base64url!!!"));
        assert!(matches!(
            verify(&bad_b64, body, &allowlist),
            Err(StampError::Malformed(_))
        ));

        let mut bad_json = HeaderMap::new();
        let payload = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(b"{\"nope\":1}");
        bad_json.insert("X-Stamp", HeaderValue::from_str(&payload).unwrap());
        assert!(matches!(
            verify(&bad_json, body, &allowlist),
            Err(StampError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_an_unsupported_scheme() {
        let key = TurnkeyP256ApiKey::generate();
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap();
        let stamp = serde_json::json!({
            "publicKey": hex::encode(key.compressed_public_key()),
            "signature": "3006020100020100",
            "scheme": "SIGNATURE_SCHEME_TK_API_ED25519",
        });
        let mut headers = HeaderMap::new();
        let value = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(stamp.to_string());
        headers.insert("X-Stamp", HeaderValue::from_str(&value).unwrap());
        assert!(matches!(
            verify(&headers, b"{}", &allowlist),
            Err(StampError::UnsupportedScheme(_))
        ));
    }
}
