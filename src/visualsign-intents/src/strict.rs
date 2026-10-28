//! Strict key sets for the JSON a signer approves.
//!
//! The defuse types decode with serde's default, which ignores keys a type does
//! not declare. For a signing screen that is wrong: an undeclared key is part of
//! the bytes the signature covers but never renders, and a misspelled key (a
//! snake_case `callback_url` for NEP-413's `callbackUrl`) silently drops a field
//! the wallet does sign. Every chain that renders these payloads decodes the
//! typed value first, so serde still reports malformed input and repeated keys,
//! then refuses any top-level key outside the type's declared set and, for an
//! intents payload, any key an intent's own kind does not declare.

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

/// The `intent` tag, which every intent object carries alongside its fields.
const INTENT_TAG: &str = "intent";

/// Optional fields every withdraw kind declares beyond its token and amount.
const WITHDRAW_OPTIONAL_KEYS: [&str; 4] = ["memo", "msg", "storage_deposit", "min_gas"];

/// The keys an intent of kind `kind` declares, besides its `intent` tag, as
/// `defuse-core` (pinned in the workspace) decodes it: each struct's own
/// fields, flattened ones included (a `transfer`'s `NotifyOnTransfer`), and
/// serde aliases (`storage_deposit`'s `account_id`). `None` for a kind this
/// build does not decode.
///
/// Keys nested inside a declared field are not listed: `tokens` and `diff`
/// are maps keyed by asset id, and every entry renders; `state_init` renders
/// only as "not fully decoded", so its contents are never shown as if read.
fn intent_keys(kind: &str) -> Option<Vec<&'static str>> {
    let withdraw = |own: &[&'static str]| {
        let mut keys = own.to_vec();
        keys.extend(WITHDRAW_OPTIONAL_KEYS);
        keys
    };
    Some(match kind {
        "add_public_key" | "remove_public_key" => vec!["public_key"],
        "transfer" => vec![
            "receiver_id",
            "tokens",
            "memo",
            "state_init",
            "msg",
            "min_gas",
        ],
        "ft_withdraw" => withdraw(&["token", "receiver_id", "amount"]),
        "nft_withdraw" => withdraw(&["token", "receiver_id", "token_id"]),
        "mt_withdraw" => withdraw(&["token", "receiver_id", "token_ids", "amounts"]),
        "native_withdraw" => vec!["receiver_id", "amount"],
        "storage_deposit" => vec![
            "contract_id",
            "deposit_for_account_id",
            "account_id",
            "amount",
        ],
        "token_diff" => vec!["diff", "memo", "referral"],
        "set_auth_by_predecessor_id" => vec!["enabled"],
        "auth_call" => vec![
            "contract_id",
            "state_init",
            "msg",
            "attached_deposit",
            "min_gas",
        ],
        _ => return None,
    })
}

/// Like [`require_declared_keys`] for an intents payload: the envelope's keys
/// must be in `envelope`, and every intent in its `intents` array may carry
/// only the keys its kind declares.
///
/// The typed decode ignores an undeclared key inside an intent exactly as it
/// does one on the envelope, so a `native_withdraw` carrying `"memo"` (a field
/// that kind does not have) would be signed and never shown.
///
/// # Errors
///
/// When `json` is not a JSON object, carries an undeclared envelope key, or an
/// intent that is not an object, names no kind this build decodes, or carries
/// an undeclared key.
pub fn require_declared_payload_keys(json: &[u8], envelope: &[&str]) -> Result<(), String> {
    require_declared_keys(json, envelope)?;
    let value: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let Some(intents) = value.get("intents").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, intent) in intents.iter().enumerate() {
        let object = intent
            .as_object()
            .ok_or_else(|| format!("intent {index} is not a JSON object"))?;
        let kind = object
            .get(INTENT_TAG)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("intent {index} names no kind"))?;
        let allowed =
            intent_keys(kind).ok_or_else(|| format!("intent {index} has unknown kind {kind:?}"))?;
        let unknown: Vec<String> = object
            .keys()
            .filter(|k| k.as_str() != INTENT_TAG && !allowed.contains(&k.as_str()))
            .map(|k| format!("{k:?}"))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "fields intent {index} ({kind}) does not declare: {}",
                unknown.join(", ")
            ));
        }
    }
    Ok(())
}

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

    /// An intent may carry only the keys its kind declares. `memo` is a real
    /// field on a withdraw of a token, but not on `native_withdraw`, so there
    /// it would be signed and never rendered.
    #[test]
    fn an_undeclared_key_inside_an_intent_is_refused() {
        let payload = |intent: &str| {
            format!(
                r#"{{"signer_id":"a.near","verifying_contract":"intents.near","deadline":"x","nonce":"y","intents":[{intent}]}}"#
            )
        };
        let hidden = payload(
            r#"{"intent":"native_withdraw","receiver_id":"bob.near","amount":"1","memo":"hidden"}"#,
        );
        let err =
            require_declared_payload_keys(hidden.as_bytes(), DEFUSE_PAYLOAD_KEYS).unwrap_err();
        assert!(
            err.contains("\"memo\"") && err.contains("native_withdraw"),
            "{err}"
        );

        let declared = payload(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1","memo":"shown"}"#,
        );
        assert!(require_declared_payload_keys(declared.as_bytes(), DEFUSE_PAYLOAD_KEYS).is_ok());

        let unknown_kind = payload(r#"{"intent":"future_unknown"}"#);
        assert!(
            require_declared_payload_keys(unknown_kind.as_bytes(), DEFUSE_PAYLOAD_KEYS).is_err()
        );
    }

    /// Each kind's declared set matches the type: an intent carrying every
    /// declared key (optional ones set to non-default values, so none is
    /// skipped on serialization) re-encodes with exactly those keys. A listed
    /// key the type does not have, or a field the list misses, fails here.
    /// `account_id`, a deserialize-only alias, is checked separately.
    #[test]
    fn every_declared_intent_key_is_a_field_of_its_kind() {
        const STATE_INIT: &str =
            r#"{"V1":{"code":{"hash":"11111111111111111111111111111111"},"data":{}}}"#;
        let withdraw_tail = r#","memo":"m","msg":"x","storage_deposit":"1","min_gas":"1""#;
        let cases = [
            r#"{"intent":"add_public_key","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"}"#.to_string(),
            r#"{"intent":"remove_public_key","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"}"#.to_string(),
            format!(
                r#"{{"intent":"transfer","receiver_id":"bob.near","tokens":{{"nep141:wrap.near":"1"}},"memo":"m","state_init":{STATE_INIT},"msg":"x","min_gas":"1"}}"#
            ),
            format!(r#"{{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1"{withdraw_tail}}}"#),
            format!(r#"{{"intent":"nft_withdraw","token":"nft.near","receiver_id":"bob.near","token_id":"1"{withdraw_tail}}}"#),
            format!(r#"{{"intent":"mt_withdraw","token":"mt.near","receiver_id":"bob.near","token_ids":["a"],"amounts":["1"]{withdraw_tail}}}"#),
            r#"{"intent":"native_withdraw","receiver_id":"bob.near","amount":"1"}"#.to_string(),
            r#"{"intent":"storage_deposit","contract_id":"wrap.near","deposit_for_account_id":"bob.near","amount":"1"}"#.to_string(),
            r#"{"intent":"token_diff","diff":{"nep141:wrap.near":"-1","nep141:usdc.near":"1"},"memo":"m","referral":"ref.near"}"#.to_string(),
            r#"{"intent":"set_auth_by_predecessor_id","enabled":true}"#.to_string(),
            format!(
                r#"{{"intent":"auth_call","contract_id":"app.near","state_init":{STATE_INIT},"msg":"x","attached_deposit":"1","min_gas":"1"}}"#
            ),
        ];
        for case in &cases {
            let input: serde_json::Value = serde_json::from_str(case).unwrap();
            let kind = input["intent"].as_str().unwrap();
            let intent: defuse_core::intents::Intent = serde_json::from_str(case)
                .unwrap_or_else(|e| panic!("{kind} does not decode: {e}"));
            let encoded = serde_json::to_value(&intent).unwrap();
            let mut encoded_keys: Vec<&str> = encoded
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .filter(|k| *k != INTENT_TAG)
                .collect();
            let mut declared: Vec<&str> = intent_keys(kind)
                .unwrap()
                .into_iter()
                .filter(|k| *k != "account_id")
                .collect();
            encoded_keys.sort_unstable();
            declared.sort_unstable();
            assert_eq!(encoded_keys, declared, "{kind}");
        }
        let aliased: defuse_core::intents::Intent = serde_json::from_str(
            r#"{"intent":"storage_deposit","contract_id":"wrap.near","account_id":"bob.near","amount":"1"}"#,
        )
        .expect("`account_id` is an alias of `deposit_for_account_id`");
        assert!(matches!(
            aliased,
            defuse_core::intents::Intent::StorageDeposit(_)
        ));
    }

    #[test]
    fn non_objects_are_refused() {
        assert!(require_declared_keys(b"[]", DEFUSE_PAYLOAD_KEYS).is_err());
        assert!(require_declared_keys(b"not json", DEFUSE_PAYLOAD_KEYS).is_err());
    }
}
