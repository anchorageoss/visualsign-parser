//! The CLI token-metadata path when the binary carries no signing key.
//!
//! Lives in `tests/` rather than the crate's own test module on purpose.
//! `sign_token_metadata_for_cli` is gated on
//! `cfg(any(test, feature = "dev-signing"))`, so under unit tests the signing
//! variant is always linked and its `Err` arm is unreachable. An integration
//! test compiles the library without `cfg(test)`, so with `dev-signing` off the
//! error-returning variant is what links -- the configuration a shipped binary
//! has.
//!
//! `make test` already runs this in the right shape: the workspace pass
//! excludes `parser_cli`, the only crate that enables `dev-signing`, so feature
//! unification cannot switch it back on here.
//!
//! Scope is the half that nothing else covers -- that an entry is dropped,
//! not registered unsigned, when signing is unavailable, matching Ethereum's
//! `--abi-json-mappings` flow: an unsigned entry can never pass the strict
//! `RequireAllowlistedSigner` posture `register` installs, so counting it as
//! loaded would be misleading.

#![cfg(all(feature = "cli-plugin", not(feature = "dev-signing")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use parser_cli_core::ChainPlugin;
use visualsign_near::{NearArgs, NearPlugin};

const ASSET_ID: &str = "nep141:not-a-seeded-asset.near";
const VALUE: &str = r#"{"symbol":"GAP","decimals":6}"#;

#[test]
fn without_dev_signing_the_cli_drops_the_entry_instead_of_registering_it_unsigned() {
    let dir = std::env::temp_dir().join("vsp_near_no_dev_signing");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("token.json");
    std::fs::write(&file, VALUE).expect("write");

    let plugin = NearPlugin::new(NearArgs {
        near_token_metadata_mappings: vec![format!("GAP@{}@{ASSET_ID}", file.display())],
    });
    let metadata = plugin
        .create_metadata(Some("NEAR_MAINNET".to_string()))
        .expect("metadata builds")
        .expect("the network flag alone yields metadata even with no token mappings");

    let Some(generated::parser::chain_metadata::Metadata::Near(near)) = metadata.metadata.as_ref()
    else {
        panic!("expected NEAR chain metadata, got {:?}", metadata.metadata);
    };

    assert!(
        !near.token_mappings.contains_key(ASSET_ID),
        "a binary built without dev-signing must drop the entry rather than \
         registering it unsigned, got {:?}",
        near.token_mappings.get(ASSET_ID)
    );

    std::fs::remove_dir_all(&dir).ok();
}
