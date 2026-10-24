#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::fs;
use std::path::PathBuf;
use visualsign::SignablePayload;
use visualsign::vsptrait::{
    Transaction, VisualSignConverterFromString, VisualSignError, VisualSignOptions,
};
use visualsign_cosmos::{CosmosTransaction, CosmosVisualSignConverter, MAX_EXEC_DEPTH};

// ---- minimal protobuf writer for building test inputs ----

fn varint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}
fn bytes_field(n: u64, v: &[u8], out: &mut Vec<u8>) {
    varint((n << 3) | 2, out);
    varint(v.len() as u64, out);
    out.extend_from_slice(v);
}
/// Varint field; proto3 omits zero values.
fn uint_field(n: u64, v: u64, out: &mut Vec<u8>) {
    if v == 0 {
        return;
    }
    varint(n << 3, out);
    varint(v, out);
}
fn any(type_url: &str, value: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    bytes_field(1, type_url.as_bytes(), &mut o);
    bytes_field(2, value, &mut o);
    o
}
fn coin(amount: &str, denom: &str) -> Vec<u8> {
    let mut o = Vec::new();
    bytes_field(1, denom.as_bytes(), &mut o);
    bytes_field(2, amount.as_bytes(), &mut o);
    o
}
fn msg_send_body(from: &str, to: &str, amount: &str) -> Vec<u8> {
    let mut m = Vec::new();
    bytes_field(1, from.as_bytes(), &mut m);
    bytes_field(2, to.as_bytes(), &mut m);
    bytes_field(3, &coin(amount, "uatom"), &mut m);
    m
}
fn msg_send(from: &str, to: &str, amount: &str) -> Vec<u8> {
    any(
        "/cosmos.bank.v1beta1.MsgSend",
        &msg_send_body(from, to, amount),
    )
}
fn tx_body(msgs: &[Vec<u8>]) -> Vec<u8> {
    let mut o = Vec::new();
    for m in msgs {
        bytes_field(1, m, &mut o);
    }
    o
}
/// `AuthInfo` with one secp256k1 signer (direct mode), sequence 1, no fee.
fn auth_info() -> Vec<u8> {
    let mut key = Vec::new();
    bytes_field(1, &[2u8; 33], &mut key);
    let pk = any("/cosmos.crypto.secp256k1.PubKey", &key);
    let mut single = Vec::new();
    uint_field(1, 1, &mut single);
    let mut mode = Vec::new();
    bytes_field(1, &single, &mut mode);
    let mut signer = Vec::new();
    bytes_field(1, &pk, &mut signer);
    bytes_field(2, &mode, &mut signer);
    uint_field(3, 1, &mut signer);
    let mut auth = Vec::new();
    bytes_field(1, &signer, &mut auth);
    auth
}
fn sign_doc(body: &[u8], auth: &[u8], chain_id: &str, account: u64) -> Vec<u8> {
    let mut o = Vec::new();
    bytes_field(1, body, &mut o);
    bytes_field(2, auth, &mut o);
    bytes_field(3, chain_id.as_bytes(), &mut o);
    uint_field(4, account, &mut o);
    o
}
fn doc_with(msgs: &[Vec<u8>]) -> String {
    hex::encode(sign_doc(&tx_body(msgs), &auth_info(), "test-1", 1))
}

// ---- helpers ----

fn options() -> VisualSignOptions {
    VisualSignOptions {
        include_intermediate_output: false,
        decode_transfers: true,
        transaction_name: None,
        metadata: None,
        developer_config: None,
    }
}

fn convert(input: &str) -> Result<SignablePayload, VisualSignError> {
    CosmosVisualSignConverter
        .to_visual_sign_payload_from_string(input, options())
        .map(|r| r.payload)
}

/// Fallback text of every field labelled `label`, in order.
fn texts(payload: &SignablePayload, label: &str) -> Vec<String> {
    payload
        .fields
        .iter()
        .filter(|f| f.label() == label)
        .map(|f| f.fallback_text().clone())
        .collect()
}

fn has(payload: &SignablePayload, label: &str, text: &str) -> bool {
    texts(payload, label).iter().any(|t| t == text)
}

fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

// ---- golden fixtures ----

static FIXTURES: [&str; 8] = [
    // CosmJS proto-signing test vector (faucet MsgSend).
    "cosmjs-send",
    "direct-staking",
    "direct-gov-ibc-wasm",
    "direct-authz",
    "direct-unknown-type",
    "direct-mldsa65",
    "amino-send",
    "amino-authz-vote",
];

#[test]
fn test_with_fixtures() {
    let dir = fixture_path("");
    for test_name in &FIXTURES {
        let input_path = dir.join(format!("{test_name}.input"));
        let input = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {input_path:?}"));

        let actual = match convert(input.trim()) {
            Ok(payload) => payload.to_json().unwrap(),
            Err(error) => format!("Error: {error:?}"),
        };

        let expected_path = dir.join(format!("{test_name}.expected"));

        // Regeneration escape hatch: `REGEN_FIXTURES=1 cargo test -p visualsign-cosmos
        // --test lib_test test_with_fixtures` rewrites every `.expected` from the
        // current parser output. Always review the resulting diff: an intended
        // decoding change and a regression look identical here.
        let regen = std::env::var("REGEN_FIXTURES")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        if regen {
            assert!(
                !actual.starts_with("Error:"),
                "REGEN_FIXTURES: refusing to write error output for '{test_name}' as expected; \
                 fix the parser or input first:\n{actual}"
            );
            fs::write(&expected_path, format!("{}\n", actual.trim_end()))
                .unwrap_or_else(|e| panic!("Failed to regenerate {expected_path:?}: {e}"));
            continue;
        }

        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {expected_path:?}"));
        assert_eq!(
            actual.trim(),
            expected.trim(),
            "Test case '{test_name}' failed"
        );
    }
}

#[test]
fn fixture_output_is_ascii_and_deterministic() {
    let dir = fixture_path("");
    for test_name in &FIXTURES {
        let input = fs::read_to_string(dir.join(format!("{test_name}.input"))).unwrap();
        let a = convert(input.trim()).unwrap();
        let b = convert(input.trim()).unwrap();
        assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
        assert!(a.to_json().unwrap().is_ascii(), "{test_name} not ASCII");
        a.validate_charset().unwrap();
    }
}

// ---- CosmJS vectors (cosmjs ec55c66, packages/proto-signing/src/testutils.ts) ----

const COSMJS_BODY: &str = "0a90010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e6412700a2d636f736d6f7331706b707472653766646b6c366766727a6c65736a6a766878686c63337234676d6d6b38727336122d636f736d6f7331717970717870713971637273737a673270767871367273307a716733797963356c7a763778751a100a0575636f736d120731323334353637";
const COSMJS_AUTH_INFO: [&str; 3] = [
    "0a4e0a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a02080112130a0d0a0575636f736d12043230303010c09a0c",
    "0a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a020801180112130a0d0a0575636f736d12043230303010c09a0c",
    "0a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a020801180212130a0d0a0575636f736d12043230303010c09a0c",
];
const COSMJS_SIGN_BYTES: [&str; 3] = [
    "0a93010a90010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e6412700a2d636f736d6f7331706b707472653766646b6c366766727a6c65736a6a766878686c63337234676d6d6b38727336122d636f736d6f7331717970717870713971637273737a673270767871367273307a716733797963356c7a763778751a100a0575636f736d12073132333435363712650a4e0a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a02080112130a0d0a0575636f736d12043230303010c09a0c1a0c73696d642d74657374696e672001",
    "0a93010a90010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e6412700a2d636f736d6f7331706b707472653766646b6c366766727a6c65736a6a766878686c63337234676d6d6b38727336122d636f736d6f7331717970717870713971637273737a673270767871367273307a716733797963356c7a763778751a100a0575636f736d12073132333435363712670a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a020801180112130a0d0a0575636f736d12043230303010c09a0c1a0c73696d642d74657374696e672001",
    "0a93010a90010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e6412700a2d636f736d6f7331706b707472653766646b6c366766727a6c65736a6a766878686c63337234676d6d6b38727336122d636f736d6f7331717970717870713971637273737a673270767871367273307a716733797963356c7a763778751a100a0575636f736d12073132333435363712670a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21034f04181eeba35391b858633a765c4a0c189697b40d216354d50890d350c7029012040a020801180212130a0d0a0575636f736d12043230303010c09a0c1a0c73696d642d74657374696e672001",
];
const FAUCET: &str = "cosmos1pkptre7fdkl6gfrzlesjjvhxhlc3r4gmmk8rs6";
const FAUCET_TO: &str = "cosmos1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5lzv7xu";

#[test]
fn cosmjs_vectors_decode_and_match_the_signing_layout() {
    for i in 0..3 {
        // The sign bytes are exactly SignDoc{body, auth_info, "simd-testing", 1}.
        let rebuilt = sign_doc(
            &hex::decode(COSMJS_BODY).unwrap(),
            &hex::decode(COSMJS_AUTH_INFO[i]).unwrap(),
            "simd-testing",
            1,
        );
        assert_eq!(hex::encode(&rebuilt), COSMJS_SIGN_BYTES[i], "vector {i}");

        let p = convert(COSMJS_SIGN_BYTES[i]).unwrap();
        assert!(has(&p, "Chain ID", "simd-testing"));
        assert!(has(&p, "Account Number", "1"));
        assert!(has(&p, "Message 1 / Type", "Send"));
        assert!(has(&p, "Message 1 / From", FAUCET));
        assert!(has(&p, "Message 1 / To", FAUCET_TO));
        assert!(has(&p, "Message 1 / Amount", "1234567 ucosm"));
        assert!(has(&p, "Fee", "2000 ucosm"));
        assert!(has(&p, "Gas Limit", "200000"));
        assert!(has(&p, "Signer 1 / Sign Mode", "direct"));
        assert!(has(&p, "Signer 1 / Sequence", &i.to_string()));
        assert!(
            texts(&p, "Signer 1 / Public Key")[0]
                .starts_with("secp256k1 034f04181eeba35391b858633a765c4a")
        );
    }
}

#[test]
fn cosmjs_vector_accepts_prefix_and_base64() {
    use base64::Engine as _;
    let raw = hex::decode(COSMJS_SIGN_BYTES[0]).unwrap();
    let from_hex = convert(COSMJS_SIGN_BYTES[0]).unwrap().to_json().unwrap();
    let from_prefixed = convert(&format!("0x{}", COSMJS_SIGN_BYTES[0]))
        .unwrap()
        .to_json()
        .unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let from_b64 = convert(&b64).unwrap().to_json().unwrap();
    assert_eq!(from_hex, from_prefixed);
    assert_eq!(from_hex, from_b64);
}

// ---- message coverage ----

#[test]
fn staking_distribution_gov_ibc_wasm_messages() {
    let dir = fixture_path("");
    let staking = convert(
        fs::read_to_string(dir.join("direct-staking.input"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert!(has(&staking, "Message 1 / Type", "Delegate"));
    assert!(has(&staking, "Message 1 / Amount", "1500000 uatom"));
    assert!(has(&staking, "Message 2 / Type", "Undelegate"));
    assert!(has(&staking, "Message 3 / Type", "Redelegate"));
    assert!(has(&staking, "Message 4 / Type", "Withdraw Rewards"));

    let misc = convert(
        fs::read_to_string(dir.join("direct-gov-ibc-wasm.input"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert!(has(&misc, "Message 1 / Type", "Vote"));
    assert!(has(&misc, "Message 1 / Proposal", "42"));
    assert!(has(&misc, "Message 1 / Option", "Yes"));
    assert!(has(&misc, "Message 2 / Type", "IBC Transfer"));
    assert!(has(&misc, "Message 2 / Channel", "transfer / channel-141"));
    assert!(has(&misc, "Message 2 / Timeout Height", "1-9000000"));
    assert!(has(&misc, "Message 2 / Memo", "bridge to osmosis"));
    assert!(has(&misc, "Message 3 / Type", "Wasm Execute"));
    assert!(has(
        &misc,
        "Message 3 / Message (contract-defined JSON)",
        r#"{"swap":{"offer_asset":"uatom","min_receive":"990000"}}"#
    ));
    assert!(has(&misc, "Message 3 / Funds", "1000000 uatom"));
    assert!(has(&misc, "Memo", "batch"));
}

#[test]
fn unknown_message_shows_its_type_url() {
    let input = fs::read_to_string(fixture_path("direct-unknown-type.input")).unwrap();
    let p = convert(input.trim()).unwrap();
    assert!(has(
        &p,
        "Message 1 / Type",
        "/cosmos.feegrant.v1beta1.MsgGrantAllowance (not fully decoded)"
    ));
    assert_eq!(texts(&p, "Message 1 / Value").len(), 1);
}

#[test]
fn ml_dsa_65_public_key_is_recognised() {
    let input = fs::read_to_string(fixture_path("direct-mldsa65.input")).unwrap();
    let p = convert(input.trim()).unwrap();
    let key = &texts(&p, "Signer 1 / Public Key")[0];
    assert!(key.starts_with("ml-dsa-65 (1952 bytes, sha256 "), "{key}");
    assert_eq!(
        visualsign_cosmos::ML_DSA_65_PUBKEY_TYPE_URL,
        "/cosmos.crypto.mldsa65.PubKey"
    );
}

#[test]
fn unrecognised_public_key_type_is_shown_not_hidden() {
    let mut key = Vec::new();
    bytes_field(1, &[1u8; 32], &mut key);
    let pk = any("/cosmos.crypto.ed25519.PubKey", &key);
    let mut signer = Vec::new();
    bytes_field(1, &pk, &mut signer);
    let mut auth = Vec::new();
    bytes_field(1, &signer, &mut auth);
    let doc = hex::encode(sign_doc(
        &tx_body(&[msg_send("a", "b", "1")]),
        &auth,
        "test-1",
        1,
    ));
    let p = convert(&doc).unwrap();
    assert!(has(
        &p,
        "Signer 1 / Public Key",
        "/cosmos.crypto.ed25519.PubKey (not fully decoded)"
    ));
}

#[test]
fn known_message_with_extra_field_is_refused() {
    let mut body = msg_send_body("a", "b", "1");
    bytes_field(15, b"hidden", &mut body);
    let err = convert(&doc_with(&[any("/cosmos.bank.v1beta1.MsgSend", &body)])).unwrap_err();
    assert!(
        format!("{err}").contains("not a canonical encoding"),
        "{err}"
    );
}

/// ibc-go v9 `MsgTransfer`: `repeated Coin tokens = 9`, deprecated `token` (3) unset.
fn ibc_v9_transfer(extra: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    bytes_field(1, b"transfer", &mut m);
    bytes_field(2, b"channel-0", &mut m);
    bytes_field(4, b"sender", &mut m);
    bytes_field(5, b"receiver", &mut m);
    bytes_field(9, &coin("100", "uatom"), &mut m);
    bytes_field(9, &coin("7", "uosmo"), &mut m);
    m.extend_from_slice(extra);
    any("/ibc.applications.transfer.v1.MsgTransfer", &m)
}

#[test]
fn ibc_v9_transfer_tokens_are_shown() {
    let p = convert(&doc_with(&[ibc_v9_transfer(&[])])).unwrap();
    assert_eq!(
        texts(&p, "Message 1 / Amount"),
        ["100 uatom".to_string(), "7 uosmo".to_string()]
    );
}

#[test]
fn ibc_v9_transfer_with_unmodelled_forwarding_is_refused() {
    // ibc-go v9 `Forwarding forwarding = 10` changes where funds go.
    let mut extra = Vec::new();
    bytes_field(10, b"\x0a\x01x", &mut extra);
    assert!(convert(&doc_with(&[ibc_v9_transfer(&extra)])).is_err());
}

#[test]
fn extension_option_value_is_shown() {
    let mut body = tx_body(&[msg_send("a", "b", "1")]);
    bytes_field(
        1023,
        &any("/example.ExtOpt", &[0xde, 0xad, 0xbe, 0xef]),
        &mut body,
    );
    bytes_field(2047, &any("/example.NonCritical", &[0x01, 0x02]), &mut body);
    let doc = hex::encode(sign_doc(&body, &auth_info(), "test-1", 1));
    let p = convert(&doc).unwrap();
    assert_eq!(
        texts(&p, "Extension Option"),
        [
            "/example.ExtOpt (not fully decoded) 0xdeadbeef".to_string(),
            "/example.NonCritical (not fully decoded) 0x0102".to_string(),
        ]
    );
}

#[test]
fn custom_transaction_name_becomes_the_title() {
    let mut o = options();
    o.transaction_name = Some("Pay".to_string());
    let r = CosmosVisualSignConverter
        .to_visual_sign_payload_from_string(COSMJS_SIGN_BYTES[0], o)
        .unwrap();
    assert_eq!(r.payload.title, "Pay");
    assert_eq!(r.payload.payload_type, "CosmosTx");
    assert_eq!(
        CosmosTransaction::from_string(COSMJS_SIGN_BYTES[0])
            .unwrap()
            .transaction_type(),
        "Cosmos"
    );
}

// ---- authz ----

fn exec(inner: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    bytes_field(1, b"grantee", &mut m);
    bytes_field(2, inner, &mut m);
    any("/cosmos.authz.v1beta1.MsgExec", &m)
}

#[test]
fn authz_exec_is_recursive_and_depth_capped() {
    let input = fs::read_to_string(fixture_path("direct-authz.input")).unwrap();
    let p = convert(input.trim()).unwrap();
    assert!(has(&p, "Message 1 / Type", "Authz Exec"));
    assert!(has(&p, "Message 1 / Message 1 / Type", "Send"));
    assert!(has(&p, "Message 1 / Message 1 / Amount", "42 uatom"));

    // MAX_EXEC_DEPTH nested execs are shown; one more is refused.
    let mut ok = msg_send("a", "b", "1");
    for _ in 0..MAX_EXEC_DEPTH {
        ok = exec(&ok);
    }
    assert!(convert(&doc_with(&[ok.clone()])).is_ok());
    let err = convert(&doc_with(&[exec(&ok)])).unwrap_err();
    assert!(format!("{err}").contains("nested too deep"), "{err}");
}

#[test]
fn authz_exec_without_messages_is_refused() {
    let mut m = Vec::new();
    bytes_field(1, b"grantee", &mut m);
    let err = convert(&doc_with(&[any("/cosmos.authz.v1beta1.MsgExec", &m)])).unwrap_err();
    assert!(format!("{err}").contains("without messages"), "{err}");
}

// ---- malformed input ----

#[test]
fn malformed_direct_input_is_refused() {
    // Not hex or base64 at all.
    assert!(convert("not a sign doc!").is_err());
    // Empty.
    assert!(convert("").is_err());
    // Truncated sign doc.
    assert!(convert(&COSMJS_SIGN_BYTES[0][..100]).is_err());
    // A SignDoc with no messages.
    let empty = hex::encode(sign_doc(&[], &auth_info(), "test-1", 1));
    assert!(convert(&empty).is_err());
    // A message whose body does not parse (truncated length-delimited field).
    let bad = any("/cosmos.bank.v1beta1.MsgSend", &[0x0a, 0x05, 0x61]);
    assert!(convert(&doc_with(&[bad])).is_err());
}

#[test]
fn tx_raw_is_not_a_sign_doc() {
    // TxRaw { body_bytes = 1, auth_info_bytes = 2, signatures = 3 }.
    let mut raw = Vec::new();
    bytes_field(1, &tx_body(&[msg_send("a", "b", "1")]), &mut raw);
    bytes_field(2, &auth_info(), &mut raw);
    bytes_field(3, &[7u8; 64], &mut raw);
    assert!(convert(&hex::encode(raw)).is_err());
}

#[test]
fn non_canonical_layers_are_refused() {
    // Unknown trailing field on the SignDoc itself.
    let mut doc = sign_doc(&tx_body(&[msg_send("a", "b", "1")]), &auth_info(), "c", 1);
    bytes_field(9, b"x", &mut doc);
    assert!(convert(&hex::encode(doc)).is_err());
    // Unknown field inside TxBody.
    let mut body = tx_body(&[msg_send("a", "b", "1")]);
    bytes_field(15, b"x", &mut body);
    let doc = sign_doc(&body, &auth_info(), "c", 1);
    assert!(convert(&hex::encode(doc)).is_err());
    // Unknown field inside AuthInfo.
    let mut auth = auth_info();
    bytes_field(15, b"x", &mut auth);
    let doc = sign_doc(&tx_body(&[msg_send("a", "b", "1")]), &auth, "c", 1);
    assert!(convert(&hex::encode(doc)).is_err());
}

#[test]
fn oversized_input_is_refused() {
    let big = "a".repeat(4 * 1024 * 1024 + 2);
    assert!(CosmosTransaction::from_string(&big).is_err());
}

// ---- amino JSON ----

#[test]
fn amino_send_and_authz_vote() {
    let dir = fixture_path("");
    let send = convert(
        fs::read_to_string(dir.join("amino-send.input"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert!(has(&send, "Chain ID", "cosmoshub-4"));
    assert!(has(&send, "Account Number", "12345"));
    assert!(has(&send, "Sequence", "7"));
    assert!(has(&send, "Message 1 / Type", "Send"));
    assert!(has(&send, "Message 1 / Amount", "1000000 uatom"));
    assert!(has(&send, "Fee", "5000 uatom"));
    assert!(has(&send, "Gas Limit", "200000"));

    let nested = convert(
        fs::read_to_string(dir.join("amino-authz-vote.input"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert!(has(&nested, "Message 1 / Type", "Authz Exec"));
    assert!(has(&nested, "Message 1 / Message 1 / Type", "Send"));
    assert!(has(&nested, "Message 2 / Type", "Vote"));
    assert!(has(&nested, "Message 2 / Option", "Yes"));
    assert!(has(&nested, "Memo", "authz"));
}

const AMINO_FEE: &str = r#""fee":{"amount":[],"gas":"1"}"#;

fn historical_amino(ty: &str, value: serde_json::Value) -> String {
    serde_json::json!({
        "account_number": "1", "chain_id": "legacy", "fee": {"amount": [], "gas": "1"},
        "memo": "", "msgs": [{"type": ty, "value": value}], "sequence": "0"
    })
    .to_string()
}

#[test]
fn sdk_historical_bank_shapes_render_every_transfer() {
    let old = historical_amino(
        "cosmos-sdk/Send",
        serde_json::json!({
            "inputs": [{"address":"a","coins":[{"amount":"3","denom":"stake"}]},
                       {"address":"b","coins":[{"amount":"4","denom":"stake"}]}],
            "outputs": [{"address":"c","coins":[{"amount":"2","denom":"stake"}]},
                        {"address":"d","coins":[{"amount":"5","denom":"stake"}]}]
        }),
    );
    let p = convert(&old).unwrap();
    assert!(has(&p, "Message 1 / Type", "Send"));
    for (kind, n, address, amount) in [
        ("Input", 1, "a", "3 stake"),
        ("Input", 2, "b", "4 stake"),
        ("Output", 1, "c", "2 stake"),
        ("Output", 2, "d", "5 stake"),
    ] {
        assert!(has(
            &p,
            &format!("Message 1 / {kind} {n} / Address"),
            address
        ));
        assert!(has(&p, &format!("Message 1 / {kind} {n} / Amount"), amount));
    }
    let later = historical_amino(
        "cosmos-sdk/Send",
        serde_json::json!({
            "from_address":"a","to_address":"b","amount":[{"amount":"7","denom":"stake"}]
        }),
    );
    let p = convert(&later).unwrap();
    assert!(has(&p, "Message 1 / From", "a"));
    assert!(has(&p, "Message 1 / Amount", "7 stake"));
}

#[test]
fn sdk_historical_staking_values_and_shares_are_distinct() {
    for (ty, addresses) in [
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_addr":"a","validator_addr":"v"}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v"}),
        ),
    ] {
        let mut v = addresses;
        v["value"] = serde_json::json!({"amount":"9","denom":"stake"});
        let p = convert(&historical_amino(ty, v)).unwrap();
        assert!(has(&p, "Message 1 / Delegator", "a"));
        assert!(has(&p, "Message 1 / Validator", "v"));
        assert!(has(&p, "Message 1 / Amount", "9 stake"));
    }
    for (ty, v) in [
        (
            "cosmos-sdk/Undelegate",
            serde_json::json!({"delegator_addr":"a","validator_addr":"v","shares_amount":"1.234567890123456789"}),
        ),
        (
            "cosmos-sdk/MsgUndelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","shares_amount":"1.234567890123456789"}),
        ),
        (
            "cosmos-sdk/BeginRedelegate",
            serde_json::json!({"delegator_addr":"a","validator_src_addr":"v","validator_dst_addr":"w","shares_amount":"1.234567890123456789"}),
        ),
        (
            "cosmos-sdk/MsgBeginRedelegate",
            serde_json::json!({"delegator_address":"a","validator_src_address":"v","validator_dst_address":"w","shares_amount":"1.234567890123456789"}),
        ),
    ] {
        let p = convert(&historical_amino(ty, v)).unwrap();
        assert!(has(&p, "Message 1 / Delegator", "a"));
        assert!(has(
            &p,
            "Message 1 / Delegation shares",
            "1.234567890123456789"
        ));
        assert!(texts(&p, "Message 1 / Amount").is_empty());
    }
}

#[test]
fn sdk_colliding_historical_shapes_are_refused() {
    for (ty, v) in [
        (
            "cosmos-sdk/Send",
            serde_json::json!({"inputs":[],"outputs":[],"from_address":"a","to_address":"b","amount":[]}),
        ),
        (
            "cosmos-sdk/MsgSend",
            serde_json::json!({"inputs":[],"outputs":[]}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_addr":"a","delegator_address":"b","validator_addr":"v","value":{"amount":"1","denom":"stake"}}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_addr":"a","validator_address":"v","value":{"amount":"1","denom":"stake"}}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","value":{"amount":"1","denom":"stake"},"amount":{"amount":"1","denom":"stake"}}),
        ),
        (
            "cosmos-sdk/MsgUndelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","shares_amount":"1.5","amount":{"amount":"1","denom":"stake"}}),
        ),
        (
            "cosmos-sdk/Undelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","amount":{"amount":"1","denom":"stake"}}),
        ),
    ] {
        assert!(convert(&historical_amino(ty, v)).is_err(), "accepted {ty}");
    }
}

#[test]
fn sdk_historical_shapes_require_the_documented_field_types() {
    for (ty, v) in [
        (
            "cosmos-sdk/Send",
            serde_json::json!({"inputs":[{"coins":[]}],"outputs":[{"address":"b","coins":[]}]}),
        ),
        (
            "cosmos-sdk/Send",
            serde_json::json!({"inputs":[{"address":"a","coins":{"amount":"1","denom":"stake"}}],"outputs":[{"address":"b","coins":[]}]}),
        ),
        (
            "cosmos-sdk/Send",
            serde_json::json!({"from_address":"a","to_address":"b","amount":{"amount":"1","denom":"stake"}}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","amount":[{"amount":"1","denom":"stake"}]}),
        ),
        (
            "cosmos-sdk/MsgDelegate",
            serde_json::json!({"delegator_address":"a","validator_address":"v","amount":null}),
        ),
    ] {
        assert!(convert(&historical_amino(ty, v)).is_err(), "accepted {ty}");
    }
}

#[test]
fn sdk_historical_vote_options_and_versioned_metadata() {
    for (input, label) in [
        ("Yes", "Yes"),
        ("Abstain", "Abstain"),
        ("No", "No"),
        ("NoWithVeto", "No with Veto"),
        ("VOTE_OPTION_YES", "Yes"),
        ("1", "Yes"),
    ] {
        let p = convert(&historical_amino(
            "cosmos-sdk/MsgVote",
            serde_json::json!({"option":input,"proposal_id":"42","voter":"a"}),
        ))
        .unwrap();
        assert!(has(&p, "Message 1 / Option", label), "{input}");
    }
    for ty in ["cosmos-sdk/MsgVote", "cosmos-sdk/v1/MsgVote"] {
        let p = convert(&historical_amino(
            ty,
            serde_json::json!({"option":1,"proposal_id":"42","voter":"a","metadata":"x"}),
        ))
        .unwrap();
        if ty == "cosmos-sdk/MsgVote" {
            assert!(has(&p, "Message 1 / Unrecognized Field metadata", "\"x\""));
            assert!(texts(&p, "Message 1 / Metadata").is_empty());
        } else {
            assert!(has(&p, "Message 1 / Metadata", "x"));
        }
    }
}

#[test]
fn sdk_direct_vote_metadata_is_only_understood_for_v1() {
    let mut body = Vec::new();
    uint_field(1, 42, &mut body);
    bytes_field(2, b"a", &mut body);
    uint_field(3, 1, &mut body);
    bytes_field(4, b"metadata", &mut body);
    assert!(convert(&doc_with(&[any("/cosmos.gov.v1beta1.MsgVote", &body)])).is_err());
    let p = convert(&doc_with(&[any("/cosmos.gov.v1.MsgVote", &body)])).unwrap();
    assert!(has(&p, "Message 1 / Metadata", "metadata"));
}

#[test]
fn amino_non_canonical_is_refused() {
    let good = r#"{"account_number":"1","chain_id":"c","fee":{"amount":[],"gas":"1"},"memo":"","msgs":[{"type":"cosmos-sdk/MsgSend","value":{"amount":[],"from_address":"a","to_address":"b"}}],"sequence":"0"}"#;
    assert!(convert(good).is_ok());
    // Whitespace.
    assert!(convert(&good.replace("\"c\",", "\"c\", ")).is_err());
    // Keys out of order.
    let unsorted = format!(
        r#"{{"chain_id":"c","account_number":"1",{AMINO_FEE},"memo":"","msgs":[],"sequence":"0"}}"#
    );
    assert!(convert(&unsorted).is_err());
    // Float number.
    assert!(convert(&good.replace("\"gas\":\"1\"", "\"gas\":1.5")).is_err());
    // Not JSON.
    assert!(convert("{not json").is_err());
}

#[test]
fn amino_unknown_message_and_fields_are_shown() {
    let doc = r#"{"account_number":"1","chain_id":"c","extra":"x","fee":{"amount":[],"gas":"1"},"memo":"","msgs":[{"type":"cosmos-sdk/MsgFoo","value":{"a":"b"}}],"sequence":"0"}"#;
    let p = convert(doc).unwrap();
    assert!(has(
        &p,
        "Message 1 / Type",
        "cosmos-sdk/MsgFoo (not fully decoded)"
    ));
    assert!(has(&p, "Unrecognized Field extra", "\"x\""));
}

#[test]
fn amino_wasm_message_is_shown_as_json() {
    let doc = r#"{"account_number":"1","chain_id":"c","fee":{"amount":[],"gas":"1"},"memo":"","msgs":[{"type":"wasm/MsgExecuteContract","value":{"contract":"k","funds":[],"msg":{"go":{}},"sender":"s"}}],"sequence":"0"}"#;
    let p = convert(doc).unwrap();
    assert!(has(&p, "Message 1 / Type", "Wasm Execute"));
    assert!(has(
        &p,
        "Message 1 / Message (contract-defined JSON)",
        r#"{"go":{}}"#
    ));
}

#[test]
fn amino_authz_depth_is_capped() {
    let send = r#"{"type":"cosmos-sdk/MsgSend","value":{"amount":[],"from_address":"a","to_address":"b"}}"#;
    let wrap = |inner: &str| {
        format!(r#"{{"type":"cosmos-sdk/MsgExec","value":{{"grantee":"g","msgs":[{inner}]}}}}"#)
    };
    let doc = |m: &str| {
        format!(
            r#"{{"account_number":"1","chain_id":"c","fee":{{"amount":[],"gas":"1"}},"memo":"","msgs":[{m}],"sequence":"0"}}"#
        )
    };
    let mut ok = send.to_string();
    for _ in 0..MAX_EXEC_DEPTH {
        ok = wrap(&ok);
    }
    assert!(convert(&doc(&ok)).is_ok());
    let err = convert(&doc(&wrap(&ok))).unwrap_err();
    assert!(format!("{err}").contains("nested too deep"), "{err}");
}
