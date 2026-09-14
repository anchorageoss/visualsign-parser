//! Ground truth for the Go mirror in visualsign-turnkeyclient: the exact bytes
//! borsh::to_vec produces for a ChainMetadata carrying NearMetadata. The Go
//! side asserts BorshBytes() equals these, which pins the digest too.
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
    println!("CASE_A {}", hex(&borsh::to_vec(&a).expect("borsh")));

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
    println!("CASE_B {}", hex(&borsh::to_vec(&b).expect("borsh")));
}
