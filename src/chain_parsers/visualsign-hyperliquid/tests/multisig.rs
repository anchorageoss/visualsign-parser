//! Independent official-SDK multisig fixtures. No signing implementation from this
//! crate or Sigil participates in fixture generation; see multisig/README.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::Deserialize;
use visualsign::vsptrait::{Transaction, VisualSignConverterFromString, VisualSignOptions};
use visualsign_hyperliquid::input::{self, Request};
use visualsign_hyperliquid::{HyperliquidTransaction, HyperliquidVisualSignConverter};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VectorFile {
    sdk_revision: String,
    vectors: Vec<Vector>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vector {
    name: String,
    unsigned_payload: String,
    digest: String,
    leader_action_json: Option<String>,
}
fn vectors() -> VectorFile {
    serde_json::from_str(include_str!("multisig/vectors.json")).unwrap()
}
fn vector(name: &str) -> Vector {
    vectors()
        .vectors
        .into_iter()
        .find(|v| v.name == name)
        .unwrap()
}
fn payload(name: &str) -> String {
    vector(name).unsigned_payload
}
fn without_digest(name: &str) -> String {
    let v = vector(name);
    v.unsigned_payload
        .replace(&format!(",\"signingDigest\":\"{}\"", v.digest), "")
}
fn replace(raw: &str, old: &str, new: &str) -> String {
    assert!(raw.contains(old), "mutation target missing: {old}");
    raw.replacen(old, new, 1)
}
fn refused(raw: &str) {
    assert!(input::parse(raw).is_err(), "unexpectedly accepted: {raw}");
}

#[test]
fn sdk_digest_vectors_cover_conversion_members_and_leaders() {
    let file = vectors();
    assert_eq!(
        file.sdk_revision,
        "2fdb18f9517675ea03695a0962bd19eece9c83f0"
    );
    assert_eq!(file.vectors.len(), 43);
    for v in file.vectors {
        let parsed =
            input::parse(&v.unsigned_payload).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        let Request::Multisig(request) = parsed else {
            panic!("wrong variant: {}", v.name)
        };
        assert_eq!(
            format!("0x{}", hex::encode(request.signing_digest)),
            v.digest,
            "{}",
            v.name
        );
        assert_eq!(
            request.leader_action_json, v.leader_action_json,
            "{} canonical native action",
            v.name
        );
        if v.name.ends_with("_leader") {
            assert_eq!(request.verified_signers.len(), 2, "{}", v.name);
            assert_ne!(request.verified_signers[0], request.verified_signers[1]);
        }
        HyperliquidTransaction::from_string(&v.unsigned_payload).unwrap();
        let rendered = HyperliquidVisualSignConverter
            .to_visual_sign_payload_from_string(&v.unsigned_payload, VisualSignOptions::default())
            .unwrap_or_else(|e| panic!("{} render: {e}", v.name))
            .payload
            .to_json()
            .unwrap();
        assert!(rendered.contains(&v.digest), "{}: digest absent", v.name);
    }
}

#[test]
fn signed_action_network_nonce_account_and_leader_mutations_fail() {
    let raw = payload("order_member");
    for (old, new) in [
        ("\"p\":\"1100\"", "\"p\":\"1101\""),
        ("\"s\":\"0.2\"", "\"s\":\"0.3\""),
        ("\"network\":\"testnet\"", "\"network\":\"mainnet\""),
        ("\"nonce\":1700000000000", "\"nonce\":1700000000001"),
        (
            "\"address\":\"0x0000000000000000000000000000000000000005\"",
            "\"address\":\"0x0000000000000000000000000000000000000006\"",
        ),
        (
            "\"leader\":\"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\"",
            "\"leader\":\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\"",
        ),
        (
            "\"vaultAddress\":null",
            "\"vaultAddress\":\"0x0000000000000000000000000000000000000011\"",
        ),
        ("\"expiresAfter\":null", "\"expiresAfter\":1700000060000"),
    ] {
        refused(&replace(&raw, old, new));
    }
}

#[test]
fn leader_member_signatures_bind_the_action_without_asserted_digest() {
    let raw = without_digest("order_leader");
    for (old, new) in [
        ("\"p\":\"1100\"", "\"p\":\"1101\""),
        ("\"network\":\"testnet\"", "\"network\":\"mainnet\""),
        ("\"nonce\":1700000000000", "\"nonce\":1700000000001"),
        (
            "\"address\":\"0x0000000000000000000000000000000000000005\"",
            "\"address\":\"0x0000000000000000000000000000000000000006\"",
        ),
    ] {
        refused(&replace(&raw, old, new));
    }
}

#[test]
fn quorum_requires_unique_recovered_authorized_signers() {
    let raw = without_digest("order_leader");
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let signatures = value["signatures"].as_array().unwrap();
    let start = raw.find("\"signatures\":[").unwrap();
    let end = raw[start..].find(']').unwrap() + start + 1;
    let with_signatures = |s: &str| format!("{}\"signatures\":{}{}", &raw[..start], s, &raw[end..]);
    refused(&with_signatures("[]"));
    refused(&with_signatures(&format!("[{}]", signatures[0])));
    refused(&with_signatures(&format!("[{0},{0}]", signatures[0])));
    refused(&replace(&raw, "\"threshold\":2", "\"threshold\":0"));
    refused(&replace(&raw, "\"threshold\":2", "\"threshold\":3"));
    refused(&replace(
        &raw,
        "\"authorizedUsers\":[\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\"",
        "\"authorizedUsers\":[\"0x0000000000000000000000000000000000000006\"",
    ));
}

#[test]
fn configuration_is_context_and_not_an_extra_signed_field() {
    let raw = payload("order_leader");
    let changed_threshold = replace(&raw, "\"threshold\":2", "\"threshold\":1");
    // Both signatures still satisfy the supplied threshold. The native digest
    // does not authenticate the threshold, so the renderer must label its trust.
    assert!(input::parse(&changed_threshold).is_ok());
    let reordered = replace(
        &raw,
        "\"authorizedUsers\":[\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\",\"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\"]",
        "\"authorizedUsers\":[\"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\",\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\"]",
    );
    assert!(input::parse(&reordered).is_ok());
}

#[test]
fn duplicate_keys_unknown_fields_and_wrong_shapes_fail_closed() {
    let raw = payload("order_member");
    for (old, new) in [
        (
            "\"kind\":\"multisig\"",
            "\"kind\":\"multisig\",\"kind\":\"multisig\"",
        ),
        ("\"a\":4", "\"a\":4,\"a\":4"),
        ("\"threshold\":2", "\"threshold\":2,\"threshold\":2"),
        ("\"p\":\"1100\"", "\"p\":1.1"),
        (
            "\"kind\":\"multisig\"",
            "\"kind\":\"multisig\",\"unknown\":true",
        ),
        ("\"type\":\"order\"", "\"type\":\"unknownAction\""),
        ("\"role\":\"member\"", "\"role\":\"anything\""),
        (
            "\"signer\":\"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf\"",
            "\"signer\":\"0x0000000000000000000000000000000000000011\"",
        ),
    ] {
        refused(&replace(&raw, old, new));
    }
}

#[test]
fn noncanonical_or_malformed_signatures_are_rejected() {
    let raw = without_digest("order_leader");
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let s = value["signatures"][0]["s"].as_str().unwrap();
    let r = value["signatures"][0]["r"].as_str().unwrap();
    // n-1 is in range but high-s. Reject rather than silently normalizing it.
    refused(&replace(
        &raw,
        s,
        "0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364140",
    ));
    refused(&replace(&raw, s, "0x0"));
    refused(&replace(&raw, r, "0x0"));
    refused(&replace(&raw, r, "0xnothex"));
    let v = value["signatures"][0]["v"].as_u64().unwrap();
    refused(&replace(&raw, &format!("\"v\":{v}"), "\"v\":0"));
}

#[test]
fn initial_conversion_cannot_be_confused_with_member_or_removal() {
    let raw = payload("initialConversion_conversion");
    refused(&replace(
        &raw,
        "\"role\":\"conversion\"",
        "\"role\":\"member\"",
    ));
    refused(&replace(
        &raw,
        "\\\"threshold\\\": 2",
        "\\\"threshold\\\": 0",
    ));
    let removal = payload("removeMultisig_member");
    refused(&replace(
        &removal,
        "\"role\":\"member\"",
        "\"role\":\"conversion\"",
    ));
}

#[test]
fn user_signed_context_must_match_embedded_network_and_nonce() {
    let raw = without_digest("usdSend_member");
    refused(&replace(
        &raw,
        "\"hyperliquidChain\":\"Testnet\"",
        "\"hyperliquidChain\":\"Mainnet\"",
    ));
    refused(&replace(
        &raw,
        "\"signatureChainId\":\"0x66eee\"",
        "\"signatureChainId\":\"0x1\"",
    ));
    refused(&replace(
        &raw,
        "\"time\":1700000000000",
        "\"time\":1700000000001",
    ));
}

#[test]
fn multisig_review_snapshots_preserve_action_and_context_disclosures() {
    let regenerate = std::env::var_os("REGEN_FIXTURES").is_some();
    for name in [
        "order_member",
        "order_leader",
        "initialConversion_conversion",
        "convertToMultiSigUser_member",
        "removeMultisig_member",
        "usdSend_member",
        "approveAgent_member",
    ] {
        let rendered = HyperliquidVisualSignConverter
            .to_visual_sign_payload_from_string(&payload(name), VisualSignOptions::default())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let pretty = serde_json::to_string_pretty(&rendered).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/multisig/{name}.expected.json"));
        if regenerate {
            std::fs::write(path, pretty).unwrap();
        } else {
            assert_eq!(pretty, std::fs::read_to_string(path).unwrap(), "{name}");
        }
    }
}

#[test]
fn nested_signature_and_embedded_configuration_duplicates_are_refused() {
    let raw = payload("order_leader");
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let r = value["signatures"][0]["r"].as_str().unwrap();
    refused(&replace(
        &raw,
        &format!("\"r\":\"{r}\""),
        &format!("\"r\":\"{r}\",\"r\":\"{r}\""),
    ));
    let conversion = payload("initialConversion_conversion");
    refused(&replace(
        &conversion,
        "\\\"threshold\\\": 2",
        "\\\"threshold\\\": 2, \\\"threshold\\\": 2",
    ));
}

#[test]
fn size_depth_and_trailing_input_bounds_are_preserved() {
    refused(&" ".repeat(input::MAX_INPUT_BYTES + 1));
    let raw = payload("order_member");
    refused(&format!("{raw} {{}}"));
    let nested = format!(
        "{}0{}",
        "[".repeat(input::MAX_DEPTH + 1),
        "]".repeat(input::MAX_DEPTH + 1)
    );
    refused(&replace(
        &raw,
        "\"signatures\":[]",
        &format!("\"signatures\":{nested}"),
    ));
}

fn direct_conversion(signers: &str) -> String {
    serde_json::json!({
        "kind": "userSigned",
        "typedData": {
            "domain": {"name":"HyperliquidSignTransaction", "version":"1", "chainId":421614,
                "verifyingContract":"0x0000000000000000000000000000000000000000"},
            "primaryType":"HyperliquidTransaction:ConvertToMultiSigUser",
            "types": {
                "EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},
                    {"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],
                "HyperliquidTransaction:ConvertToMultiSigUser":[
                    {"name":"hyperliquidChain","type":"string"}, {"name":"signers","type":"string"},
                    {"name":"nonce","type":"uint64"}]
            },
            "message":{"hyperliquidChain":"Testnet", "signers":signers, "nonce":1700000000000_u64,
                "type":"convertToMultiSigUser", "signatureChainId":"0x66eee"}
        }
    }).to_string()
}

#[test]
fn direct_user_signed_conversion_and_removal_are_reviewable() {
    for name in ["initialConversion_conversion", "removeMultisig_member"] {
        let raw: serde_json::Value = serde_json::from_str(&payload(name)).unwrap();
        let signers = raw["action"]["signers"].as_str().unwrap();
        let unsigned = direct_conversion(signers);
        assert!(matches!(
            input::parse(&unsigned).unwrap(),
            Request::UserSigned(_)
        ));
        let rendered = HyperliquidVisualSignConverter
            .to_visual_sign_payload_from_string(&unsigned, VisualSignOptions::default())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(rendered.contains("Signer Configuration"), "{rendered}");
        assert!(rendered.contains("Testnet"), "{rendered}");
    }
}

#[test]
fn direct_conversion_rejects_malformed_or_ambiguous_configuration() {
    let member = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";
    for signers in [
        "{}".to_string(),
        "not JSON".to_string(),
        format!("{{\"authorizedUsers\":[\"{member}\"],\"threshold\":0}}"),
        format!("{{\"authorizedUsers\":[\"{member}\"],\"threshold\":2}}"),
        format!("{{\"authorizedUsers\":[\"{member}\",\"{member}\"],\"threshold\":1}}"),
        format!("{{\"authorizedUsers\":[\"{member}\"],\"threshold\":1,\"threshold\":1}}"),
        format!("{{\"authorizedUsers\":[\"{member}\"],\"threshold\":1,\"hidden\":true}}"),
    ] {
        refused(&direct_conversion(&signers));
    }
}

#[test]
fn multisig_account_itself_cannot_be_a_current_or_proposed_member() {
    let current = without_digest("order_member");
    refused(&replace(
        &current,
        "\"authorizedUsers\":[\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\"",
        "\"authorizedUsers\":[\"0x0000000000000000000000000000000000000005\"",
    ));
    let update = without_digest("convertToMultiSigUser_member");
    refused(&replace(
        &update,
        "\\\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\\\"",
        "\\\"0x0000000000000000000000000000000000000005\\\"",
    ));
    let conversion = without_digest("initialConversion_conversion");
    let value: serde_json::Value = serde_json::from_str(&conversion).unwrap();
    let original = value["signer"].as_str().unwrap();
    refused(&replace(
        &conversion,
        "\\\"0x2b5ad5c4795c026514f8317c7a215e218dccd6cf\\\"",
        &format!("\\\"{original}\\\""),
    ));
}

fn with_signatures(raw: &str, signatures: &serde_json::Value) -> String {
    let start = raw.find("\"signatures\":[").unwrap();
    let end = raw[start..].find(']').unwrap() + start + 1;
    format!(
        "{}\"signatures\":{}{}",
        &raw[..start],
        signatures,
        &raw[end..]
    )
}

#[test]
fn padded_uppercase_signature_scalars_produce_sdk_canonical_leader_bytes() {
    let v = vector("leadingZeroSignature_leader");
    let value: serde_json::Value = serde_json::from_str(&v.unsigned_payload).unwrap();
    let mut signatures = value["signatures"].clone();
    for signature in signatures.as_array_mut().unwrap() {
        for key in ["r", "s"] {
            let scalar = signature[key].as_str().unwrap().strip_prefix("0x").unwrap();
            signature[key] = format!("0x{:0>64}", scalar.to_uppercase()).into();
        }
    }
    let raw = with_signatures(&v.unsigned_payload, &signatures);
    let Request::Multisig(request) = input::parse(&raw).unwrap() else {
        panic!("wrong variant")
    };
    assert_eq!(
        format!("0x{}", hex::encode(request.signing_digest)),
        v.digest
    );
    assert_eq!(request.leader_action_json, v.leader_action_json);
}

#[test]
fn valid_signature_order_changes_leader_digest_but_not_member_digest() {
    let raw = without_digest("order_leader");
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut signatures = value["signatures"].clone();
    signatures.as_array_mut().unwrap().reverse();
    let swapped = with_signatures(&raw, &signatures);
    let Request::Multisig(original) = input::parse(&raw).unwrap() else {
        panic!("wrong variant")
    };
    let Request::Multisig(changed) = input::parse(&swapped).unwrap() else {
        panic!("wrong variant")
    };
    assert_eq!(original.member_digest, changed.member_digest);
    assert_ne!(original.signing_digest, changed.signing_digest);
    assert_ne!(original.leader_action_json, changed.leader_action_json);
    assert_eq!(
        changed.verified_signers,
        original
            .verified_signers
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
    );
}

#[test]
fn hash_only_multisig_and_enriched_member_typed_data_are_refused() {
    let raw: serde_json::Value =
        serde_json::from_str(&payload("initialConversion_conversion")).unwrap();
    let mut direct: serde_json::Value = serde_json::from_str(&direct_conversion(
        raw["action"]["signers"].as_str().unwrap(),
    ))
    .unwrap();
    let name = "HyperliquidTransaction:ConvertToMultiSigUser";
    let fields = direct["typedData"]["types"][name].as_array_mut().unwrap();
    fields.insert(
        1,
        serde_json::json!({"name":"payloadMultiSigUser","type":"address"}),
    );
    fields.insert(
        2,
        serde_json::json!({"name":"outerSigner","type":"address"}),
    );
    direct["typedData"]["message"]["payloadMultiSigUser"] =
        "0x0000000000000000000000000000000000000005".into();
    direct["typedData"]["message"]["outerSigner"] =
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf".into();
    refused(&direct.to_string());
    direct["typedData"]["types"]
        .as_object_mut()
        .unwrap()
        .remove(name);
    direct["typedData"]["primaryType"] = "HyperliquidTransaction:SendMultiSig".into();
    direct["typedData"]["types"]["HyperliquidTransaction:SendMultiSig"] = serde_json::json!([
        {"name":"hyperliquidChain","type":"string"}, {"name":"multiSigActionHash","type":"bytes32"},
        {"name":"nonce","type":"uint64"}]);
    direct["typedData"]["message"] = serde_json::json!({"hyperliquidChain":"Testnet",
        "multiSigActionHash":format!("0x{}", "11".repeat(32)), "nonce":1700000000000_u64});
    refused(&direct.to_string());
}
