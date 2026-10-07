//! Strict key sets for the JSON a signer approves.
//!
//! The defuse types decode with serde's default, which ignores keys a type does
//! not declare. For a signing screen that is wrong: an undeclared key is part of
//! the bytes the signature covers but never renders, and a misspelled key (a
//! snake_case `callback_url` for NEP-413's `callbackUrl`) silently drops a field
//! the wallet does sign. Every chain that renders these payloads decodes the
//! typed value first, so serde still reports malformed input and repeated keys,
//! then refuses any top-level key outside the type's declared set.

/// Top-level keys `DefusePayload<DefuseIntents>` reads: its own fields plus the
/// flattened `DefuseIntents`.
pub const DEFUSE_PAYLOAD_KEYS: &[&str] = &[
    "signer_id",
    "verifying_contract",
    "deadline",
    "nonce",
    "intents",
];

/// Top-level keys `Nep413Payload` reads, in its camelCase JSON form.
pub const NEP413_KEYS: &[&str] = &["message", "nonce", "recipient", "callbackUrl"];

/// Keys of the intents request a NEP-413 `message` carries
/// (`Nep413DefuseMessage<DefuseIntents>`): `verifying_contract` and `nonce` come
/// from the envelope instead.
pub const NEP413_INTENTS_MESSAGE_KEYS: &[&str] = &["signer_id", "deadline", "intents"];

/// Top-level keys of the JSON object in `json` that are not in `allowed`, each
/// quoted so a refusal can name them unambiguously.
///
/// # Errors
///
/// When `json` is not valid JSON or not a JSON object.
pub fn undeclared_keys(json: &[u8], allowed: &[&str]) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let object = value.as_object().ok_or("not a JSON object")?;
    Ok(object
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()))
        .map(|k| format!("{k:?}"))
        .collect())
}

/// `Ok(())` when every top-level key of `json` is in `allowed`, otherwise a
/// refusal naming the undeclared keys.
///
/// # Errors
///
/// When `json` is not a JSON object, or carries a key outside `allowed`.
pub fn require_declared_keys(json: &[u8], allowed: &[&str]) -> Result<(), String> {
    let unknown = undeclared_keys(json, allowed)?;
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "fields this payload does not declare: {}",
            unknown.join(", ")
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn declared_keys_pass_and_others_are_named() {
        let ok = br#"{"signer_id":"a.near","verifying_contract":"intents.near","deadline":"x","nonce":"y","intents":[]}"#;
        assert!(require_declared_keys(ok, DEFUSE_PAYLOAD_KEYS).is_ok());

        let extra = br#"{"signer_id":"a.near","message":"hi","recipient":"intents.near"}"#;
        let err = require_declared_keys(extra, DEFUSE_PAYLOAD_KEYS).unwrap_err();
        assert!(
            err.contains("\"message\"") && err.contains("\"recipient\""),
            "{err}"
        );
    }

    #[test]
    fn a_snake_case_callback_url_is_not_a_nep413_key() {
        let err = require_declared_keys(
            br#"{"message":"m","nonce":"n","recipient":"r","callback_url":"u"}"#,
            NEP413_KEYS,
        )
        .unwrap_err();
        assert!(err.contains("\"callback_url\""), "{err}");
    }

    #[test]
    fn non_objects_are_refused() {
        assert!(require_declared_keys(b"[]", DEFUSE_PAYLOAD_KEYS).is_err());
        assert!(require_declared_keys(b"not json", DEFUSE_PAYLOAD_KEYS).is_err());
    }
}
