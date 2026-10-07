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

/// A real Turnkey stamp is ~250 bytes; generous headroom over that without
/// leaving the header effectively unbounded (see `verify`'s size check).
const MAX_STAMP_HEADER_BYTES: usize = 1024;

#[derive(Debug)]
#[allow(dead_code)]
pub enum StampError {
    Missing,
    Malformed(String),
    UnsupportedScheme(String),
    UnknownKey,
    BadSignature,
}

impl StampError {
    /// Bounded, caller-uncontrolled discriminant for logging. `Malformed` and
    /// `UnsupportedScheme` carry attacker-supplied strings (an unbounded
    /// `scheme`, a `serde_json` error `Display`), so logging `self` with
    /// `{:?}` would let an unauthenticated caller amplify enclave logs; this
    /// gives callers something safe to log instead.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Malformed(_) => "malformed",
            Self::UnsupportedScheme(_) => "unsupported_scheme",
            Self::UnknownKey => "unknown_key",
            Self::BadSignature => "bad_signature",
        }
    }
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

/// The curve an allowlist entry is bound to.
///
/// A compressed SEC1 point is the same 33 bytes on both curves, and ~half of
/// the valid x-coordinates on one curve are also valid on the other, so the
/// bytes alone cannot say which curve a key belongs to. The entry carries the
/// answer instead (see `CURVE_TAG_*`), and `verify` requires the request's
/// `scheme` to match it, rather than letting the caller choose which curve
/// their key is checked against.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Curve {
    P256,
    Secp256k1,
}

const CURVE_TAG_P256: &str = "p256";
const CURVE_TAG_SECP256K1: &str = "secp256k1";

/// Compressed SEC1 pubkeys permitted to call the parse routes.
///
/// Deliberately not `visualsign::signing::SignerAllowlist`: that type's
/// `BTreeSet` lookup isn't constant-time (see `contains_constant_time`
/// below, needed here because this allowlist gates authentication, not
/// metadata trust) and it stores keys in the Ethereum ABI path's uncompressed
/// SEC1 encoding rather than the compressed form Turnkey's stamper uses.
pub struct Allowlist {
    keys: Vec<(Curve, [u8; 33])>,
}

impl Allowlist {
    /// Parses `[<curve>:]<hex>` entries, comma-separated. `<curve>` is
    /// `p256` (the default, matching Turnkey's default stamping scheme) or
    /// `secp256k1`.
    pub fn from_hex_list(csv: &str) -> Result<Self, StampError> {
        let mut keys = Vec::new();
        for entry in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (curve, hex) = match entry.split_once(':') {
                Some((CURVE_TAG_P256, hex)) => (Curve::P256, hex.trim()),
                Some((CURVE_TAG_SECP256K1, hex)) => (Curve::Secp256k1, hex.trim()),
                Some((tag, _)) => {
                    return Err(StampError::Malformed(format!(
                        "allowlist entry {entry}: unknown curve tag {tag} (expected \
                         {CURVE_TAG_P256} or {CURVE_TAG_SECP256K1})"
                    )));
                }
                None => (Curve::P256, entry),
            };
            let bytes = visualsign::encodings::decode_hex_array::<33>(hex).map_err(|e| {
                StampError::Malformed(format!(
                    "allowlist entry {entry} (compressed SEC1 hex): {e}"
                ))
            })?;
            // decode_hex_array only checks hex syntax and length; without this,
            // a syntactically-valid but non-curve-point entry would start the
            // deployment with a silently-dead allowlist slot that can never
            // authenticate (see PR review).
            let on_curve = match curve {
                Curve::P256 => p256::ecdsa::VerifyingKey::from_sec1_bytes(&bytes).is_ok(),
                Curve::Secp256k1 => k256::ecdsa::VerifyingKey::from_sec1_bytes(&bytes).is_ok(),
            };
            if !on_curve {
                return Err(StampError::Malformed(format!(
                    "allowlist entry {entry}: not a valid compressed SEC1 point on {curve:?}"
                )));
            }
            keys.push((curve, bytes));
        }
        if keys.is_empty() {
            return Err(StampError::Malformed("allowlist is empty".to_string()));
        }
        keys.sort_unstable();
        keys.dedup();
        Ok(Self { keys })
    }

    /// Constant-time over the key bytes. The curve comparison is not, and
    /// need not be: it is fixed by the request's own `scheme`.
    fn contains_constant_time(&self, candidate: &[u8], curve: Curve) -> bool {
        let mut found = subtle::Choice::from(0u8);
        for (entry_curve, key) in &self.keys {
            let curve_ok = subtle::Choice::from(u8::from(*entry_curve == curve));
            found |= key.as_slice().ct_eq(candidate) & curve_ok;
        }
        found.into()
    }

    /// Number of distinct allowlisted keys, for the startup log line only -
    /// never the keys themselves outside the constant-time compare above.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
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
    // Before any authentication decision, an anonymous caller could otherwise
    // force base64/JSON decoding and hex parsing on an unbounded header
    // (hyper's own head-buffer cap is the only thing limiting it), the same
    // CPU-amplification shape `PIVOT_BODY_LIMIT_BYTES` exists to prevent on
    // the body.
    if raw.as_bytes().len() > MAX_STAMP_HEADER_BYTES {
        return Err(StampError::Malformed("stamp header too large".to_string()));
    }
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .map_err(|e| StampError::Malformed(format!("base64url: {e}")))?;
    let stamp: ApiStamp = serde_json::from_slice(&decoded)
        .map_err(|e| StampError::Malformed(format!("stamp json: {e}")))?;

    let pubkey = visualsign::encodings::decode_hex(&stamp.public_key)
        .map_err(|e| StampError::Malformed(format!("publicKey hex: {e}")))?;
    let sig_der = visualsign::encodings::decode_hex(&stamp.signature)
        .map_err(|e| StampError::Malformed(format!("signature hex: {e}")))?;

    // Run the full parse/verify path unconditionally, before deciding
    // allowlist membership, so a listed and an unlisted candidate cost the
    // same DER parsing, curve validation and ECDSA verification. The
    // allowlist lives in `pivotArgs`, which `bootProof.qosManifestB64` only
    // carries on a successful parse, and a caller can only reach that after
    // passing this check.
    let (curve, sig_ok) = match stamp.scheme.as_str() {
        SCHEME_P256 => {
            use p256::ecdsa::{DerSignature, VerifyingKey, signature::Verifier};
            let key = VerifyingKey::from_sec1_bytes(&pubkey)
                .map_err(|e| StampError::Malformed(format!("p256 pubkey: {e}")))?;
            let sig = DerSignature::from_bytes(&sig_der)
                .map_err(|e| StampError::Malformed(format!("p256 der: {e}")))?;
            (Curve::P256, key.verify(body, &sig).is_ok())
        }
        SCHEME_SECP256K1 => {
            use k256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
            let key = VerifyingKey::from_sec1_bytes(&pubkey)
                .map_err(|e| StampError::Malformed(format!("k256 pubkey: {e}")))?;
            let sig = Signature::from_der(&sig_der)
                .map_err(|e| StampError::Malformed(format!("k256 der: {e}")))?;
            // k256 rejects high-S signatures while p256 accepts both, so a
            // stamper that doesn't normalize S would fail about half the time.
            // Malleability is moot here since stamps are replayable by design.
            let sig = sig.normalize_s().unwrap_or(sig);
            (Curve::Secp256k1, key.verify(body, &sig).is_ok())
        }
        other => return Err(StampError::UnsupportedScheme(other.to_string())),
    };

    // Curve-bound: a key listed for one curve does not authorize a signature
    // under the other. The same 33 bytes are a valid point on both curves
    // about half the time, so without this the caller's `scheme` would pick
    // which curve their own allowlisted key is checked against.
    if !allowlist.contains_constant_time(&pubkey, curve) {
        return Err(StampError::UnknownKey);
    }
    if !sig_ok {
        return Err(StampError::BadSignature);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use k256::ecdsa::Signature;
    use turnkey_api_key_stamper::{Stamp, TurnkeyP256ApiKey, TurnkeySecp256k1ApiKey};

    fn headers_for(key: &impl Stamp, body: &[u8]) -> HeaderMap {
        let stamp = key.stamp(body).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-Stamp", HeaderValue::from_str(&stamp.value).unwrap());
        headers
    }

    fn allowlist_of(key: &TurnkeyP256ApiKey) -> Allowlist {
        Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap()
    }

    /// Shared by tests that only need a key and an allowlist containing it,
    /// not a specific key/allowlist relationship.
    fn single_key_allowlist() -> (TurnkeyP256ApiKey, Allowlist) {
        let key = TurnkeyP256ApiKey::generate();
        let allowlist = allowlist_of(&key);
        (key, allowlist)
    }

    #[test]
    fn accepts_a_stamp_from_an_allowlisted_key() {
        let (key, allowlist) = single_key_allowlist();
        let body = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02"}}"#;
        verify(&headers_for(&key, body), body, &allowlist).unwrap();
    }

    #[test]
    fn rejects_a_stamp_from_an_unlisted_key() {
        let signer = TurnkeyP256ApiKey::generate();
        let other = TurnkeyP256ApiKey::generate();
        let allowlist = allowlist_of(&other);
        let body = br#"{"request":{}}"#;
        let err = verify(&headers_for(&signer, body), body, &allowlist).unwrap_err();
        assert!(matches!(err, StampError::UnknownKey));
    }

    #[test]
    fn signature_is_checked_against_raw_bytes_not_reserialized_json() {
        let (key, allowlist) = single_key_allowlist();
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
        let (_key, allowlist) = single_key_allowlist();
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
    fn accepts_a_stamp_from_an_allowlisted_secp256k1_key() {
        let key = TurnkeySecp256k1ApiKey::generate();
        let allowlist = Allowlist::from_hex_list(&format!(
            "secp256k1:{}",
            hex::encode(key.compressed_public_key())
        ))
        .unwrap();
        let body = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02"}}"#;
        verify(&headers_for(&key, body), body, &allowlist).unwrap();
    }

    #[test]
    fn accepts_a_high_s_secp256k1_stamp() {
        let key = TurnkeySecp256k1ApiKey::generate();
        let allowlist = Allowlist::from_hex_list(&format!(
            "secp256k1:{}",
            hex::encode(key.compressed_public_key())
        ))
        .unwrap();
        let body = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02"}}"#;

        // The stamper emits low-S; flip it to the equivalent high-S form.
        let stamp = key.stamp(body).unwrap();
        let decoded = BASE64_URL_SAFE_NO_PAD.decode(&stamp.value).unwrap();
        let mut json: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        let der = hex::decode(json["signature"].as_str().unwrap()).unwrap();
        let low = Signature::from_der(&der).unwrap();
        let high = Signature::from_scalars(low.r().to_bytes(), (-*low.s()).to_bytes()).unwrap();
        assert!(high.normalize_s().is_some(), "fixture must be high-S");
        json["signature"] = hex::encode(high.to_der().as_bytes()).into();

        let mut headers = HeaderMap::new();
        let value = BASE64_URL_SAFE_NO_PAD.encode(json.to_string());
        headers.insert("X-Stamp", HeaderValue::from_str(&value).unwrap());
        verify(&headers, body, &allowlist).unwrap();
    }

    #[test]
    fn an_untagged_entry_defaults_to_p256_and_rejects_a_secp256k1_stamp() {
        // The key-confusion case the curve tag exists to close: a secp256k1
        // key whose compressed bytes also parse as a P256 point (~half of
        // them do). Listed untagged, it is a P256 entry, and a secp256k1
        // stamp from the very same bytes must not satisfy it.
        let key = std::iter::repeat_with(TurnkeySecp256k1ApiKey::generate)
            .take(64)
            .find(|k| {
                p256::ecdsa::VerifyingKey::from_sec1_bytes(&k.compressed_public_key()).is_ok()
            })
            .expect("no secp256k1 key in 64 tries was also a valid P256 point");
        let allowlist =
            Allowlist::from_hex_list(&hex::encode(key.compressed_public_key())).unwrap();
        let body = br#"{"request":{}}"#;
        let err = verify(&headers_for(&key, body), body, &allowlist).unwrap_err();
        assert!(matches!(err, StampError::UnknownKey));
    }

    #[test]
    fn rejects_an_unknown_curve_tag() {
        let key = TurnkeyP256ApiKey::generate();
        let entry = format!("ed25519:{}", hex::encode(key.compressed_public_key()));
        assert!(matches!(
            Allowlist::from_hex_list(&entry),
            Err(StampError::Malformed(_))
        ));
    }

    #[test]
    fn repeated_entries_are_counted_once() {
        let key = TurnkeyP256ApiKey::generate();
        let hex_key = hex::encode(key.compressed_public_key());
        // Same key three ways: bare, `0x`-prefixed, and explicitly tagged.
        let csv = format!("{hex_key}, 0x{hex_key}, p256:{hex_key}");
        let allowlist = Allowlist::from_hex_list(&csv).unwrap();
        assert_eq!(allowlist.len(), 1);
    }

    #[test]
    fn accepts_a_stamp_from_the_second_key_in_a_multi_entry_allowlist() {
        let first = TurnkeyP256ApiKey::generate();
        let second = TurnkeyP256ApiKey::generate();
        // Whitespace around entries and a `0x`-prefixed entry must both be
        // accepted, and membership must not be limited to the first entry.
        let csv = format!(
            " {}, 0x{} ",
            hex::encode(first.compressed_public_key()),
            hex::encode(second.compressed_public_key())
        );
        let allowlist = Allowlist::from_hex_list(&csv).unwrap();
        let body = br#"{"request":{"chain":"CHAIN_ETHEREUM","unsigned_payload":"0x02"}}"#;
        verify(&headers_for(&second, body), body, &allowlist).unwrap();
    }

    #[test]
    fn rejects_an_allowlist_entry_that_is_not_a_valid_curve_point() {
        // 33 bytes, valid hex, wrong leading byte for a compressed SEC1 point
        // (0x04 is the uncompressed-point prefix, invalid at this length).
        // Regression test for the PR-review fix: `decode_hex_array` only
        // checks hex syntax and length, so without the SEC1 parse in
        // `from_hex_list` this entry would silently join the allowlist as a
        // dead key that can never authenticate.
        let bad_entry = format!("04{}", "00".repeat(32));
        assert!(matches!(
            Allowlist::from_hex_list(&bad_entry),
            Err(StampError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_an_unsupported_scheme() {
        let (key, allowlist) = single_key_allowlist();
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
