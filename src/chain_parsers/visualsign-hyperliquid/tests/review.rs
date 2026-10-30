//! Refusal regressions: each `review_*` test takes a valid SDK vector, makes one
//! change an attacker or a buggy client could make, and asserts the parser refuses it.
//!
//! Mutations are textual so everything else stays byte-identical; re-serialising
//! through `serde_json::Value` would re-order keys and change the hash for an
//! unrelated reason.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use visualsign::vsptrait::Transaction;
use visualsign_hyperliquid::HyperliquidTransaction;
use visualsign_hyperliquid::input::{MAX_DEPTH, MAX_INPUT_BYTES};

fn vector(name: &str) -> String {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/vectors.json"
    ))
    .unwrap();
    let file: serde_json::Value = serde_json::from_str(&raw).unwrap();
    file["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no vector {name}"))["unsignedPayload"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Replace exactly one occurrence, so a mutation that silently misses fails the test.
fn mutate(payload: &str, from: &str, to: &str) -> String {
    assert_eq!(
        payload.matches(from).count(),
        1,
        "pattern {from:?} must occur once"
    );
    payload.replacen(from, to, 1)
}

fn refused(payload: &str) -> String {
    match HyperliquidTransaction::from_string(payload) {
        Ok(tx) => panic!("expected refusal, got {tx:?}"),
        Err(e) => e.to_string(),
    }
}

fn assert_refused(payload: &str, needle: &str) {
    let err = refused(payload);
    assert!(err.contains(needle), "expected {needle:?} in: {err}");
}

const ORDER: &str = "l1_order_limit_mainnet";
const MISMATCH: &str = "connectionId mismatch";

#[test]
fn review_unmodified_vector_is_accepted() {
    // Control: every mutation below starts from a payload that parses.
    HyperliquidTransaction::from_string(&vector(ORDER)).unwrap();
}

#[test]
fn review_mutated_price_is_refused() {
    let p = mutate(&vector(ORDER), r#""p":"100000""#, r#""p":"100001""#);
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_mutated_side_is_refused() {
    let p = mutate(&vector(ORDER), r#""b":true"#, r#""b":false"#);
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_mutated_asset_is_refused() {
    let p = mutate(&vector(ORDER), r#""a":0,"#, r#""a":1,"#);
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_swapped_key_order_is_refused() {
    // Same keys and values, different order: msgpack bytes differ, so the hash does.
    let p = mutate(&vector(ORDER), r#""a":0,"b":true"#, r#""b":true,"a":0"#);
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_wrong_nonce_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""nonce":1761480000000"#,
        r#""nonce":1761480000001"#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_vault_added_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""vaultAddress":null"#,
        r#""vaultAddress":"0x1719884eb866cb12b2287399b15f7db5e7d775ea""#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_vault_removed_is_refused() {
    let p = mutate(
        &vector("l1_order_vault"),
        r#""vaultAddress":"0x1719884eb866cb12b2287399b15f7db5e7d775ea""#,
        r#""vaultAddress":null"#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_vault_changed_is_refused() {
    let p = mutate(
        &vector("l1_order_vault"),
        "0x1719884eb866cb12b2287399b15f7db5e7d775ea",
        "0x1719884eb866cb12b2287399b15f7db5e7d775eb",
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_expires_after_added_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""expiresAfter":null"#,
        r#""expiresAfter":1761483600000"#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_expires_after_removed_is_refused() {
    let p = mutate(
        &vector("l1_order_expires_after"),
        r#""expiresAfter":1761483600000"#,
        r#""expiresAfter":null"#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_expires_after_changed_is_refused() {
    let p = mutate(
        &vector("l1_cancel_vault_expires_after"),
        r#""expiresAfter":1761483600000"#,
        r#""expiresAfter":1861483600000"#,
    );
    assert_refused(&p, MISMATCH);
}

#[test]
fn review_float_price_is_refused() {
    let p = mutate(&vector(ORDER), r#""p":"100000""#, r#""p":100000.5"#);
    assert_refused(&p, "non-integer number");
}

#[test]
fn review_float_integral_value_is_refused() {
    // 0.0 is a float in JSON even though it is integral; it would hash as float64.
    let p = mutate(&vector(ORDER), r#""a":0,"#, r#""a":0.0,"#);
    assert_refused(&p, "non-integer number");
}

#[test]
fn review_unknown_type_is_refused() {
    let p = mutate(
        &vector("l1_noop"),
        r#"{"type":"noop"}"#,
        r#"{"type":"setReferrer","code":"X"}"#,
    );
    assert_refused(&p, "unsupported L1 action type 'setReferrer'");
}

#[test]
fn review_unknown_field_is_refused_even_with_a_valid_hash() {
    // vectors.json carries this action validly hashed by the SDK; see vectors.rs.
    assert_refused(
        &vector("refuse_l1_order_unknown_field"),
        "unknown field `extra`",
    );
}

#[test]
fn review_unknown_spot_deploy_variant_is_refused() {
    let p = mutate(
        &vector("l1_spot_deploy_enable_quote_token"),
        "enableQuoteToken",
        "setDeployerLabel",
    );
    // Refused at the hash (mutation) before the shape check; either refusal is fine,
    // but it must not render.
    refused(&p);
}

#[test]
fn review_source_c_is_refused() {
    let p = mutate(&vector(ORDER), r#""source":"a""#, r#""source":"c""#);
    assert_refused(&p, "unknown L1 source 'c'");
}

#[test]
fn review_wrong_chain_id_is_refused() {
    let p = mutate(&vector(ORDER), r#""chainId":1337"#, r#""chainId":1338"#);
    assert_refused(&p, "L1 domain must be");
}

#[test]
fn review_string_chain_id_is_refused() {
    let p = mutate(&vector(ORDER), r#""chainId":1337"#, r#""chainId":"1337""#);
    assert_refused(&p, "typedData is not a Hyperliquid L1");
}

#[test]
fn review_wrong_domain_name_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""name":"Exchange""#,
        r#""name":"Exchange2""#,
    );
    assert_refused(&p, "L1 domain must be");
}

#[test]
fn review_reordered_agent_type_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#"[{"name":"source","type":"string"},{"name":"connectionId","type":"bytes32"}]"#,
        r#"[{"name":"connectionId","type":"bytes32"},{"name":"source","type":"string"}]"#,
    );
    assert_refused(&p, "does not match the pinned definition");
}

#[test]
fn review_wrong_primary_type_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""primaryType":"Agent""#,
        r#""primaryType":"Mail""#,
    );
    refused(&p);
}

#[test]
fn review_missing_action_is_refused() {
    let p = vector(ORDER);
    let start = p.find(r#","action":"#).unwrap();
    let end = p.find(r#","nonce":"#).unwrap();
    let without = format!("{}{}", &p[..start], &p[end..]);
    assert_refused(&without, "requires the action");
}

#[test]
fn review_unknown_envelope_field_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""kind":"l1","#,
        r#""kind":"l1","extra":1,"#,
    );
    assert_refused(&p, "unknown field `extra`");
}

#[test]
fn review_unknown_kind_is_refused() {
    let p = mutate(&vector(ORDER), r#""kind":"l1""#, r#""kind":"l2""#);
    assert_refused(&p, "unknown kind 'l2'");
}

#[test]
fn review_oversized_input_is_refused_before_parsing() {
    // Not valid JSON at all: the size cap must fire before the parser runs.
    let p = "x".repeat(MAX_INPUT_BYTES + 1);
    assert_refused(&p, "the limit is");
}

#[test]
fn review_deep_nesting_is_refused() {
    let depth = MAX_DEPTH + 1;
    let nested = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let p = mutate(
        &vector(ORDER),
        r#""grouping":"na""#,
        &format!(r#""grouping":{nested}"#),
    );
    assert_refused(&p, "nests deeper than");
}

#[test]
fn review_brackets_inside_strings_do_not_count_as_nesting() {
    // The nesting scan must skip string contents (including escaped quotes), or a
    // client order id full of brackets would be refused.
    let p = vector(ORDER);
    assert!(!p.contains("[[[[[[[[[[[[[[[[[["));
    let with_brackets = mutate(&p, r#""kind":"l1""#, r#""kind":"l1\"[[[[[[[[[[[[[[[[[[""#);
    // Refused for the unknown kind, not for nesting.
    assert_refused(&with_brackets, "unknown kind");
}

// ---------------------------------------------------------------- user-signed

#[test]
fn review_user_signed_control_is_accepted() {
    HyperliquidTransaction::from_string(&vector("user_usd_send")).unwrap();
}

#[test]
fn review_unknown_user_signed_primary_type_is_refused() {
    let p = vector("user_usd_send").replace(
        "HyperliquidTransaction:UsdSend",
        "HyperliquidTransaction:UsdSend2",
    );
    assert_refused(&p, "unsupported user-signed primaryType");
}

#[test]
fn review_accept_terms_needs_its_exact_primary_type() {
    // AcceptTerms is allowlisted under `Hyperliquid:AcceptTerms` only; the
    // `HyperliquidTransaction:` prefix the other actions use must not match it.
    let input = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/recorded/t1-accept-terms.input.json"
    ))
    .unwrap();
    HyperliquidTransaction::from_string(&input).unwrap();
    let p = input.replace(
        "Hyperliquid:AcceptTerms",
        "HyperliquidTransaction:AcceptTerms",
    );
    assert_refused(&p, "unsupported user-signed primaryType");
}

#[test]
fn review_user_signed_wrong_domain_name_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""name":"HyperliquidSignTransaction""#,
        r#""name":"Exchange""#,
    );
    assert_refused(&p, "user-signed domain must be");
}

#[test]
fn review_user_signed_extra_message_field_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""amount":"1.5","#,
        r#""amount":"1.5","memo":"x","#,
    );
    assert_refused(&p, "unexpected field 'memo'");
}

#[test]
fn review_user_signed_extra_type_field_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#"{"name":"time","type":"uint64"}"#,
        r#"{"name":"time","type":"uint64"},{"name":"memo","type":"string"}"#,
    );
    assert_refused(&p, "does not match the pinned definition");
}

#[test]
fn review_user_signed_type_mismatch_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""type":"usdSend""#,
        r#""type":"spotSend""#,
    );
    assert_refused(&p, "does not match primaryType");
}

#[test]
fn review_user_signed_signature_chain_id_mismatch_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""signatureChainId":"0x66eee""#,
        r#""signatureChainId":"0xa4b1""#,
    );
    assert_refused(&p, "does not match domain chainId");
}

#[test]
fn review_user_signed_unknown_network_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""hyperliquidChain":"Mainnet""#,
        r#""hyperliquidChain":"Devnet""#,
    );
    assert_refused(&p, "hyperliquidChain must be Mainnet or Testnet");
}

#[test]
fn review_user_signed_string_time_is_refused() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""time":1761480000000"#,
        r#""time":"1761480000000""#,
    );
    assert_refused(&p, "is not a valid uint64");
}

#[test]
fn review_user_signed_rejects_l1_fields() {
    let p = mutate(
        &vector("user_usd_send"),
        r#""kind":"userSigned","#,
        r#""kind":"userSigned","nonce":1,"#,
    );
    assert_refused(&p, "takes only typedData");
}

#[test]
fn review_user_signed_bad_address_is_refused() {
    let p = mutate(
        &vector("user_approve_builder_fee"),
        r#""builder":"0x8c967e73e7b15087c42a10d344cff4c96d877f1d""#,
        r#""builder":"0x8c967e73e7b15087c42a10d344cff4c96d877f1""#,
    );
    assert_refused(&p, "is not a valid address");
}

#[test]
fn review_agent_malformed_valid_until_is_refused() {
    let p = mutate(
        &vector("user_approve_agent_valid_until"),
        "bot valid_until 1767225600000",
        "bot valid_until 1767225600000 extra",
    );
    assert_refused(&p, "malformed valid_until");
}

#[test]
fn review_user_signed_duplicate_message_key_is_refused() {
    // serde_json would keep the last "amount"; another encoder might sign the first.
    let p = mutate(
        &vector("user_usd_send"),
        r#""amount":"1.5","#,
        r#""amount":"1.5","amount":"1500","#,
    );
    assert_refused(&p, "duplicate key 'amount'");
}

#[test]
fn review_l1_duplicate_connection_id_is_refused() {
    let p = mutate(
        &vector(ORDER),
        r#""message":{"source":"a","#,
        r#""message":{"source":"a","connectionId":"0x00","#,
    );
    assert_refused(&p, "duplicate field `connectionId`");
}
