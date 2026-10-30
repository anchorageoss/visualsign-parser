//! Schema-derived cross-version Cosmos SDK corpus (see
//! `docs/cosmos-sdk-compatibility.md`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use sha2::{Digest, Sha256};
use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignOptions};
use visualsign_cosmos::CosmosVisualSignConverter;

#[test]
fn tagged_sdk_fixture_matrix_preserves_display_semantics() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/sdk-compatibility.json")).unwrap();
    assert_eq!(corpus["format_version"], 1);
    let fixtures = corpus["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 17);
    assert_eq!(corpus["sources"].as_array().unwrap().len(), 14);
    for fixture in fixtures {
        let name = fixture["id"].as_str().unwrap();
        let input = fixture["sign_doc"].as_str().unwrap();
        let bytes = if fixture["format"] == "direct" {
            hex::decode(input).unwrap()
        } else {
            input.as_bytes().to_vec()
        };
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            fixture["sha256"].as_str().unwrap(),
            "{name}: fixture integrity"
        );
        let options = VisualSignOptions {
            include_intermediate_output: false,
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
        };
        let payload = CosmosVisualSignConverter
            .to_visual_sign_payload_from_string(input, options)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"))
            .payload;
        for field in fixture["fields"].as_array().unwrap() {
            let label = field[0].as_str().unwrap();
            let value = field[1].as_str().unwrap();
            assert!(
                payload
                    .fields
                    .iter()
                    .any(|f| f.label().eq_ignore_ascii_case(label)
                        && (f.fallback_text() == value
                            || (label.ends_with(" / Option")
                                && f.fallback_text().eq_ignore_ascii_case(value)))),
                "{name}: missing {label} = {value}; got {:?}",
                payload.fields
            );
        }
        if name.contains("shares") {
            assert!(
                !payload
                    .fields
                    .iter()
                    .any(|f| f.label() == "Message 1 / Amount"),
                "{name}: shares must never be labeled token Amount"
            );
        }
    }
}
