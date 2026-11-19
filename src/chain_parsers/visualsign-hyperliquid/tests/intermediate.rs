//! Intermediate output: the structured copy of the verified action that a policy
//! engine reads. Envelopes are built here with the crate's own hash, so each one
//! carries the `connectionId` its action really commits to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use visualsign::vsptrait::{ConversionResult, VisualSignConverterFromString, VisualSignOptions};
use visualsign_hyperliquid::HyperliquidVisualSignConverter;
use visualsign_hyperliquid::hash::{action_msgpack, connection_id};
use visualsign_hyperliquid::intermediate::{
    HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION, HyperliquidIntermediateField,
    HyperliquidIntermediateOutput,
};

const NONCE: u64 = 1_761_091_200_000;
const OPERATOR: &str = "0xCD0160530441212612EE9E773F4cdb7974894410";

fn l1_envelope(action: &str) -> String {
    let id = connection_id(&action_msgpack(action).unwrap(), NONCE, None, None);
    let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        r#"{{"kind":"l1","typedData":{{"domain":{{"chainId":1337,"name":"Exchange","verifyingContract":"0x0000000000000000000000000000000000000000","version":"1"}},"types":{{"Agent":[{{"name":"source","type":"string"}},{{"name":"connectionId","type":"bytes32"}}],"EIP712Domain":[{{"name":"name","type":"string"}},{{"name":"version","type":"string"}},{{"name":"chainId","type":"uint256"}},{{"name":"verifyingContract","type":"address"}}]}},"primaryType":"Agent","message":{{"source":"b","connectionId":"0x{id}"}}}},"action":{action},"nonce":{NONCE},"vaultAddress":null,"expiresAfter":null}}"#
    )
}

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

fn convert(payload: &str, include: bool) -> ConversionResult {
    let options = VisualSignOptions {
        include_intermediate_output: include,
        ..VisualSignOptions::default()
    };
    HyperliquidVisualSignConverter
        .to_visual_sign_payload_from_string(payload, options)
        .unwrap()
}

fn output(payload: &str) -> HyperliquidIntermediateOutput {
    let bytes = convert(payload, true).intermediate_output.unwrap();
    borsh::from_slice(&bytes).unwrap()
}

fn fields(items: &[(&str, &str, &str)]) -> Vec<HyperliquidIntermediateField> {
    items
        .iter()
        .map(|(name, data_type, value)| HyperliquidIntermediateField {
            name: (*name).to_string(),
            data_type: (*data_type).to_string(),
            value: (*value).to_string(),
        })
        .collect()
}

#[test]
fn halt_trading_carries_the_signed_fields() {
    let out = output(&l1_envelope(
        r#"{"type":"perpDeploy","haltTrading":{"coin":"prb:TEST","isHalted":true}}"#,
    ));
    assert_eq!(out.schema_version, HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION);
    assert_eq!(out.kind, "l1");
    assert_eq!(out.network, "Testnet");
    assert_eq!(out.action_type, "perpDeploy");
    assert_eq!(out.variant, "haltTrading");
    assert_eq!(out.nonce, NONCE);
    assert_eq!(out.vault_address, "");
    assert_eq!(out.expires_after, "");
    assert_eq!(
        out.fields,
        fields(&[("coin", "string", "prb:TEST"), ("isHalted", "bool", "true")])
    );
    assert!(out.entries.is_empty());
}

#[test]
fn register_asset2_carries_every_field_in_wire_order() {
    let action = format!(
        r#"{{"type":"perpDeploy","registerAsset2":{{"maxGas":null,"assetRequest":{{"coin":"prb:TEST","szDecimals":2,"oraclePx":"10.0","marginTableId":10,"marginMode":"strictIsolated"}},"dex":"prb","schema":{{"fullName":"Probe dex","collateralToken":0,"oracleUpdater":"{OPERATOR}"}}}}}}"#
    );
    let out = output(&l1_envelope(&action));
    assert_eq!(out.variant, "registerAsset2");
    assert_eq!(
        out.fields,
        fields(&[
            ("maxGas", "null", ""),
            ("assetRequest.coin", "string", "prb:TEST"),
            ("assetRequest.szDecimals", "uint64", "2"),
            ("assetRequest.oraclePx", "string", "10.0"),
            ("assetRequest.marginTableId", "uint64", "10"),
            ("assetRequest.marginMode", "string", "strictIsolated"),
            ("dex", "string", "prb"),
            ("schema.fullName", "string", "Probe dex"),
            ("schema.collateralToken", "uint64", "0"),
            ("schema.oracleUpdater", "string", OPERATOR),
        ])
    );
}

#[test]
fn register_asset2_without_a_schema_says_so() {
    let out = output(&l1_envelope(
        r#"{"type":"perpDeploy","registerAsset2":{"maxGas":5,"assetRequest":{"coin":"prb:TWO","szDecimals":0,"oraclePx":"1","marginTableId":3,"marginMode":"noCross"},"dex":"prb","schema":null}}"#,
    ));
    assert_eq!(out.fields[0], fields(&[("maxGas", "uint64", "5")])[0]);
    assert_eq!(
        out.fields.last().unwrap(),
        &fields(&[("schema", "null", "")])[0]
    );
}

#[test]
fn set_sub_deployers_emits_one_entry_per_grant() {
    let action = format!(
        r#"{{"type":"perpDeploy","setSubDeployers":{{"dex":"prb","subDeployers":[{{"variant":"setOracle","user":"{OPERATOR}","allowed":true}},{{"variant":"haltTrading","user":"{OPERATOR}","allowed":false}}]}}}}"#
    );
    let out = output(&l1_envelope(&action));
    assert_eq!(out.variant, "setSubDeployers");
    assert_eq!(out.fields, fields(&[("dex", "string", "prb")]));
    assert_eq!(
        out.entries,
        vec![
            fields(&[
                ("variant", "string", "setOracle"),
                ("user", "string", OPERATOR),
                ("allowed", "bool", "true"),
            ]),
            fields(&[
                ("variant", "string", "haltTrading"),
                ("user", "string", OPERATOR),
                ("allowed", "bool", "false"),
            ]),
        ]
    );
}

#[test]
fn send_asset_carries_the_message_fields() {
    let payload = vector("user_send_asset");
    let message: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let message = &message["typedData"]["message"];
    let out = output(&payload);
    assert_eq!(out.kind, "userSigned");
    assert_eq!(out.action_type, "SendAsset");
    assert_eq!(out.network, message["hyperliquidChain"].as_str().unwrap());
    assert_eq!(out.variant, "");
    let names: Vec<&str> = out.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "hyperliquidChain",
            "destination",
            "sourceDex",
            "destinationDex",
            "token",
            "amount",
            "fromSubAccount",
            "nonce"
        ]
    );
    for f in &out.fields {
        let signed = &message[f.name.as_str()];
        let expected = signed
            .as_str()
            .map_or_else(|| signed.to_string(), str::to_string);
        assert_eq!(f.value, expected, "field {}", f.name);
    }
    assert_eq!(out.fields[1].data_type, "string");
    assert_eq!(out.fields[7].data_type, "uint64");
}

#[test]
fn nothing_is_emitted_unless_asked() {
    let payload =
        l1_envelope(r#"{"type":"perpDeploy","haltTrading":{"coin":"prb:TEST","isHalted":true}}"#);
    let without = convert(&payload, false);
    let with = convert(&payload, true);
    assert!(without.intermediate_output.is_none());
    assert_eq!(
        without.payload.to_json().unwrap(),
        with.payload.to_json().unwrap()
    );
}

#[test]
fn an_action_without_a_mapping_is_refused_when_output_is_asked_for() {
    let payload = vector("l1_perp_deploy_set_oracle");
    assert!(convert(&payload, false).intermediate_output.is_none());
    let options = VisualSignOptions {
        include_intermediate_output: true,
        ..VisualSignOptions::default()
    };
    let err = HyperliquidVisualSignConverter
        .to_visual_sign_payload_from_string(&payload, options)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("intermediate output is not available"),
        "{err}"
    );
}

#[test]
fn every_user_signed_vector_emits_its_message_fields() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/vectors.json"
    ))
    .unwrap();
    let file: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut checked = 0;
    for v in file["vectors"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        if !name.starts_with("user_") {
            continue;
        }
        let payload = v["unsignedPayload"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(payload).unwrap();
        let message = &envelope["typedData"]["message"];
        let out = output(payload);
        assert_eq!(out.kind, "userSigned", "{name}");
        assert!(!out.fields.is_empty(), "{name}");
        for f in &out.fields {
            let signed = &message[f.name.as_str()];
            let expected = signed
                .as_str()
                .map_or_else(|| signed.to_string(), str::to_string);
            assert_eq!(f.value, expected, "{name}: field {}", f.name);
        }
        checked += 1;
    }
    assert!(checked >= 9, "user-signed vector set shrank");
}
