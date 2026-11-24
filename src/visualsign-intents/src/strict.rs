//! Strict key sets for JSON a signer approves. serde ignores undeclared keys, but
//! on a signing screen they would be signed unseen (and a misspelled key drops a
//! field), so callers decode the typed value, then refuse undeclared keys.

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

/// Keys an intent of `kind` declares besides `intent`, per the pinned
/// `defuse-core` (flattened fields and serde aliases included); `None` for a kind
/// this build does not decode. Keys inside maps such as `tokens` are not listed.
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

/// [`require_declared_keys`] for an intents payload: envelope keys must be in
/// `envelope`, and each intent may carry only its kind's keys.
///
/// # Errors
///
/// When `json` is not a JSON object, has an undeclared key, or has an intent of
/// an unknown kind.
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

/// Top-level keys of `json` not in `allowed`, quoted.
///
/// # Errors
///
/// When `json` is not a JSON object.
pub fn undeclared_keys(json: &[u8], allowed: &[&str]) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let object = value.as_object().ok_or("not a JSON object")?;
    Ok(object
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()))
        .map(|k| format!("{k:?}"))
        .collect())
}

/// Refuses any top-level key of `json` outside `allowed`.
///
/// # Errors
///
/// When `json` is not a JSON object or has an undeclared key.
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
    use defuse_core::intents::Intent;
    use std::collections::BTreeSet;

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

    /// One intent of every kind, with every declared key set to a non-default value.
    fn every_kind_with_every_key() -> Vec<String> {
        const STATE_INIT: &str =
            r#"{"V1":{"code":{"hash":"11111111111111111111111111111111"},"data":{}}}"#;
        let withdraw_tail = r#","memo":"m","msg":"x","storage_deposit":"1","min_gas":"1""#;
        vec![
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
        ]
    }

    /// The tag of every `Intent` variant. No wildcard arm, so a new defuse variant
    /// fails to compile here until it is named.
    fn intent_tag(intent: &Intent) -> &'static str {
        match intent {
            Intent::AddPublicKey(_) => "add_public_key",
            Intent::RemovePublicKey(_) => "remove_public_key",
            Intent::Transfer(_) => "transfer",
            Intent::FtWithdraw(_) => "ft_withdraw",
            Intent::NftWithdraw(_) => "nft_withdraw",
            Intent::MtWithdraw(_) => "mt_withdraw",
            Intent::NativeWithdraw(_) => "native_withdraw",
            Intent::StorageDeposit(_) => "storage_deposit",
            Intent::TokenDiff(_) => "token_diff",
            Intent::SetAuthByPredecessorId(_) => "set_auth_by_predecessor_id",
            Intent::AuthCall(_) => "auth_call",
        }
    }

    /// Number of arms in [`intent_tag`].
    const INTENT_VARIANTS: usize = 11;

    #[test]
    fn every_intent_variant_has_a_declared_key_set() {
        let mut seen = BTreeSet::new();
        for case in every_kind_with_every_key() {
            let input: serde_json::Value = serde_json::from_str(&case).unwrap();
            let intent: Intent = serde_json::from_str(&case).unwrap();
            let tag = intent_tag(&intent);
            assert_eq!(input["intent"].as_str(), Some(tag));
            assert!(intent_keys(tag).is_some(), "{tag} has no declared key set");
            seen.insert(tag);
        }
        assert_eq!(
            seen.len(),
            INTENT_VARIANTS,
            "a variant has no case: {seen:?}"
        );
    }

    /// Each kind's declared keys re-encode exactly: a listed key the type lacks,
    /// or a field the list misses, fails here. `account_id` is checked separately.
    #[test]
    fn every_declared_intent_key_is_a_field_of_its_kind() {
        let cases = every_kind_with_every_key();
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
