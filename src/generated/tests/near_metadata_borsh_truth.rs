//! Ground truth for the Go mirror in visualsign-turnkeyclient: the exact bytes
//! borsh::to_vec produces for a ChainMetadata carrying NearMetadata.
//!
//! The constants below are duplicated in that repository's
//! api/borsh_test.go. Both sides assert, so a schema change here fails this
//! test rather than waiting for the Go repository's CI to notice -- a
//! cross-language ground truth only works if the side that owns the schema is
//! the side that breaks. Run with --nocapture to print the bytes when
//! regenerating after a deliberate change.
use std::collections::BTreeMap;

use generated::parser::{
    chain_metadata, ChainMetadata, Metadata, NearMetadata, SignatureMetadata, TokenMetadataEntry,
    TokenOriginChain,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn print_near_chain_metadata_borsh() {
    let a = ChainMetadata {
        metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
            network_id: Some("NEAR_MAINNET".to_string()),
            token_mappings: BTreeMap::new(),
        })),
    };
    let case_a = hex(&borsh::to_vec(&a).expect("borsh"));
    println!("CASE_A {case_a}");
    assert_eq!(case_a, "0102010c0000004e4541525f4d41494e4e455400000000");

    // Inserted in reverse key order on purpose: BTreeMap ordering is what the
    // Go side has to reproduce with an explicit sort.
    let mut tm = BTreeMap::new();
    tm.insert(
        "nep141:zzz.near".to_string(),
        TokenMetadataEntry {
            value: r#"{"symbol":"ZZZ","decimals":8}"#.to_string(),
            signature: None,
            origin_chain: None,
        },
    );
    tm.insert(
        "nep141:wrap.near".to_string(),
        TokenMetadataEntry {
            value: r#"{"symbol":"wNEAR","decimals":24}"#.to_string(),
            signature: Some(SignatureMetadata {
                value: "deadbeef".to_string(),
                metadata: vec![
                    Metadata { key: "algorithm".to_string(), value: "ed25519".to_string() },
                    Metadata { key: "public_key".to_string(), value: "abc123".to_string() },
                ],
            }),
            origin_chain: Some(TokenOriginChain::Ethereum as i32),
        },
    );
    let b = ChainMetadata {
        metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
            network_id: Some("NEAR_TESTNET".to_string()),
            token_mappings: tm,
        })),
    };
    let case_b = hex(&borsh::to_vec(&b).expect("borsh"));
    println!("CASE_B {case_b}");
    assert_eq!(
        case_b,
        concat!(
            "0102010c0000004e4541525f544553544e455402000000100000006e65703134313a777261702e6e656172",
            "200000007b2273796d626f6c223a22774e454152222c22646563696d616c73223a32347d0108000000",
            "64656164626565660200000009000000616c676f726974686d07000000656432353531390a00000070",
            "75626c69635f6b65790600000061626331323301020000000f0000006e65703134313a7a7a7a2e6e65",
            "61721d0000007b2273796d626f6c223a225a5a5a222c22646563696d616c73223a387d0000"
        )
    );
}
