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
//! Scope is the half that nothing else covers -- that an entry is registered
//! *unsigned* rather than dropped when signing is unavailable. What happens to
//! an unsigned entry afterwards (refused under `RequireAllowlistedSigner`,
//! accepted only as a gap-fill under `AcceptUnsigned`) is covered by
//! `token_signature.rs`'s own tests, which don't depend on this feature.

#![cfg(all(feature = "cli-plugin", not(feature = "dev-signing")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use parser_cli_core::ChainPlugin;
use visualsign_near::{NearArgs, NearPlugin};

const ASSET_ID: &str = "nep141:not-a-seeded-asset.near";
const VALUE: &str = r#"{"symbol":"GAP","decimals":6}"#;

#[test]
fn without_dev_signing_the_cli_registers_the_entry_unsigned() {
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
        .expect("a loadable mapping yields metadata");

    let Some(generated::parser::chain_metadata::Metadata::Near(near)) = metadata.metadata.as_ref()
    else {
        panic!("expected NEAR chain metadata, got {:?}", metadata.metadata);
    };
    let entry = near
        .token_mappings
        .get(ASSET_ID)
        .expect("the mapping registers even though signing was unavailable");

    assert_eq!(entry.value, VALUE, "the file contents are carried verbatim");
    assert!(
        entry.signature.is_none(),
        "a binary built without dev-signing must register the entry unsigned \
         rather than attaching a signature, got {:?}",
        entry.signature
    );

    std::fs::remove_dir_all(&dir).ok();
}
