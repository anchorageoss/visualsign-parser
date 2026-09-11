use std::collections::BTreeMap;

use clap::Args as ClapArgs;
use generated::parser::{ChainMetadata, NearMetadata, TokenMetadataEntry, chain_metadata};
use visualsign::registry::{Chain, TransactionConverterRegistry};

use parser_cli_core::mapping_parser::{MappingComponents, MappingFormat, load_mappings};

use crate::networks::NearNetwork;
use crate::presets::intents::sign_token_metadata_for_cli;

/// CLI arguments specific to NEAR.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct NearArgs {
    /// Map custom token metadata JSON file to a NEAR Intents asset id.
    ///
    /// Format: `Name@/path/to/token.json@AssetId`. `@` is the field
    /// separator here, not `:` (the convention `--abi-json-mappings`/
    /// `--idl-json-mappings` use), because NEAR Intents asset ids embed
    /// their own colons (e.g. `nep141:wrap.near`), which would make the
    /// identifier ambiguous under a colon-delimited format. The file should
    /// contain `{"symbol":"...","decimals":N}`. Can be used multiple times.
    #[arg(
        long = "near-token-metadata-mappings",
        value_name = "NAME@FILE_PATH@ASSET_ID"
    )]
    pub near_token_metadata_mappings: Vec<String>,
}

/// [`parser_cli_core::ChainPlugin`] implementation for NEAR.
pub struct NearPlugin {
    args: NearArgs,
}

impl NearPlugin {
    /// Creates a new `NearPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: NearArgs) -> Self {
        Self { args }
    }
}

/// The CLI always runs the strict posture -- every token-metadata entry must
/// carry a signature -- matching `visualsign-ethereum`'s CLI plugin (see its
/// `cli_trust_policy`). Signer identity for a present signature is checked per
/// origin chain against `authorized_token_metadata_signers`, which is the
/// allowlist this posture carries: the list the variant advertises and the list
/// the decode path checks are the same value, so an empty one (no
/// `VISUALSIGN_*_TOKEN_SIGNERS` configured, no `dev-signing`) rejects every
/// entry rather than quietly checking against something else.
fn cli_trust_policy() -> visualsign::signing::MetadataTrustPolicy {
    visualsign::signing::MetadataTrustPolicy::RequireAllowlistedSigner(
        crate::presets::intents::authorized_token_metadata_signers().clone(),
    )
}

impl parser_cli_core::ChainPlugin for NearPlugin {
    fn chain(&self) -> Chain {
        Chain::Near
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<crate::NearTransaction, _>(
            Chain::Near,
            crate::NearVisualSignConverter::with_trust_policy(cli_trust_policy()),
        );
    }

    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        create_chain_metadata(network, &self.args.near_token_metadata_mappings)
    }
}

/// Parse the NEAR-specific `Name@Path@AssetId` mapping format.
///
/// Splits into exactly 3 parts on `@`; the third part is the asset id
/// verbatim, including any colons it contains (unlike the shared
/// `mapping_parser::parse_mapping`, which splits on `:` and would truncate a
/// NEAR asset id at its first embedded colon).
fn parse_near_mapping(mapping_str: &str) -> Result<MappingComponents, String> {
    // Split on every '@' rather than the first two: a path containing '@' is
    // legal on Linux/macOS, and `splitn(3, '@')` would quietly absorb the
    // remainder into the asset id, turning "/tmp/a@b/t.json" into path
    // "/tmp/a" and asset id "b/t.json@nep141:...". An asset id never contains
    // '@', so a fourth component means the path did.
    let parts: Vec<&str> = mapping_str.split('@').collect();
    let [name, path, asset_id] = parts[..] else {
        if parts.len() > 3 {
            return Err(format!(
                "Invalid mapping format: found {} '@'-separated components, expected 3 \
                 (Name@FilePath@AssetId). A file path containing '@' cannot be used here: \
                 {mapping_str}",
                parts.len()
            ));
        }
        return Err(format!(
            "Invalid mapping format (expected Name@FilePath@AssetId): {mapping_str}"
        ));
    };
    if name.is_empty() || path.is_empty() || asset_id.is_empty() {
        return Err(format!("Mapping components cannot be empty: {mapping_str}"));
    }
    Ok(MappingComponents {
        name: name.to_string(),
        path: path.to_string(),
        identifier: asset_id.to_string(),
    })
}

/// Load token-metadata JSON files and build mappings for
/// `NearMetadata.token_mappings`. Each entry is signed with the CLI's local
/// dev key (NEAR-origin ed25519) so it extracts as verified rather than
/// dropped by the strict posture [`cli_trust_policy`] installs; a signing
/// failure rejects the entry, matching Ethereum's `--abi-json-mappings` flow,
/// since an unsigned entry cannot pass that posture anyway and would
/// otherwise be logged and counted as loaded despite being dead weight.
///
/// The load/dedupe/count/report loop is `load_mappings`, shared with the ABI
/// and IDL flags, so the three chains cannot drift on what a duplicate or an
/// unreadable file does. Only the parse differs, because a NEAR asset id
/// carries its own colons.
fn build_token_mappings_from_files(
    mappings: &[String],
    signing_network: NearNetwork,
) -> (BTreeMap<String, TokenMetadataEntry>, usize) {
    load_mappings(
        mappings,
        &MappingFormat {
            kind: "token metadata",
            example: "USDC@usdc.json@nep141:a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near",
            identifier_label: "AssetId",
            format_hint: "Name@/path/to/file.json@AssetId",
        },
        parse_near_mapping,
        // Any non-empty string is a candidate asset id: the id space is the
        // caller's contract naming, not a fixed encoding this side can check,
        // and `parse_near_mapping` has already rejected an empty component.
        |_| Ok(()),
        |components, json| {
            let signature = sign_token_metadata_for_cli(
                signing_network.network_id(),
                &components.identifier,
                &json,
            )
            .map_err(|e| format!("failed to sign token metadata: {e}"))?;
            Ok(TokenMetadataEntry {
                value: json,
                signature: Some(signature),
                origin_chain: None,
            })
        },
    )
}

/// Build NEAR chain metadata from the global `--network` flag and the
/// NEAR-specific token-metadata mappings.
///
/// Returns `None` when neither yields anything: with no network and no loadable
/// mapping there is nothing for `NearMetadata` to carry.
fn create_chain_metadata(
    network: Option<String>,
    mappings: &[String],
) -> Result<Option<ChainMetadata>, String> {
    let network = match network {
        Some(network) => match NearNetwork::from_network_id(&network) {
            Some(parsed) => Some(parsed),
            None => {
                return Err(format!(
                    "Invalid network '{network}'. Supported: NEAR_MAINNET, NEAR_TESTNET"
                ));
            }
        },
        None => None,
    };
    let network_id = network.map(|n| n.network_id().to_string());
    // Token-metadata signatures are scoped to a network, so the CLI must sign
    // for the one the parser will resolve for this request: the `--network`
    // flag when given, otherwise the network the converter defaults to.
    let signing_network = network.unwrap_or_default();

    let token_mappings = if mappings.is_empty() {
        BTreeMap::new()
    } else {
        eprintln!("Loading custom token metadata:");
        let (token_mappings, valid_count) =
            build_token_mappings_from_files(mappings, signing_network);
        eprintln!(
            "Successfully loaded {}/{} token metadata mappings\n",
            valid_count,
            mappings.len()
        );
        token_mappings
    };

    if network_id.is_none() && token_mappings.is_empty() {
        return Ok(None);
    }

    Ok(Some(ChainMetadata {
        metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
            network_id,
            token_mappings: token_mappings.into_iter().collect(),
        })),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use near_crypto::{KeyType, PublicKey};
    use near_primitives::action::{Action, FunctionCallAction};
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::{Transaction, TransactionV0};
    use near_primitives::types::{Balance, Gas};
    use parser_cli_core::ChainPlugin;
    use visualsign::vsptrait::VisualSignOptions;

    fn plugin() -> NearPlugin {
        NearPlugin::new(NearArgs::default())
    }

    fn plugin_with_mappings(mappings: Vec<String>) -> NearPlugin {
        NearPlugin::new(NearArgs {
            near_token_metadata_mappings: mappings,
        })
    }

    fn write_temp_json(name: &str, content: &str) -> std::path::PathBuf {
        parser_cli_core::test_utils::write_temp_json("vsp_near_tests", name, content)
    }

    /// Unwrap the `Near` variant of the metadata oneof.
    fn near_metadata(metadata: ChainMetadata) -> NearMetadata {
        let chain_metadata::Metadata::Near(near) = metadata.metadata.unwrap() else {
            panic!("expected Near metadata");
        };
        near
    }

    #[test]
    fn create_metadata_defaults_to_none_without_a_network_flag_or_mappings() {
        assert_eq!(plugin().create_metadata(None).unwrap(), None);
    }

    #[test]
    fn create_metadata_builds_near_metadata_for_a_valid_network() {
        let metadata = plugin()
            .create_metadata(Some("near_testnet".to_string()))
            .unwrap()
            .expect("Some(ChainMetadata)");
        assert_eq!(
            metadata,
            ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_TESTNET".to_string()),
                    token_mappings: Default::default(),
                })),
            }
        );
    }

    #[test]
    fn create_metadata_rejects_an_invalid_network() {
        assert!(plugin().create_metadata(Some("bogus".to_string())).is_err());
    }

    /// An invalid network fails even when a mapping loads fine, so a bad
    /// `--network` can't be masked by a successful mapping load.
    #[test]
    fn create_metadata_rejects_an_invalid_network_even_with_mappings() {
        let path = write_temp_json("reject.json", r#"{"symbol":"X","decimals":6}"#);
        let mappings = vec![format!("X@{}@nep141:x.near", path.display())];
        assert!(
            plugin_with_mappings(mappings)
                .create_metadata(Some("bogus".to_string()))
                .is_err()
        );
    }

    #[test]
    fn create_metadata_carries_a_token_mapping() {
        let path = write_temp_json("usdc.json", r#"{"symbol":"USDC.e","decimals":6}"#);
        let asset_id = "nep141:a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near";
        let mappings = vec![format!("USDC@{}@{asset_id}", path.display())];

        let near = near_metadata(
            plugin_with_mappings(mappings)
                .create_metadata(Some("NEAR_MAINNET".to_string()))
                .unwrap()
                .expect("Some(ChainMetadata)"),
        );
        assert_eq!(near.network_id, Some("NEAR_MAINNET".to_string()));
        assert_eq!(near.token_mappings.len(), 1);
        let entry = near.token_mappings.get(asset_id).expect("mapping present");
        assert!(entry.value.contains("USDC.e"));
        // The CLI signs locally-loaded token metadata so the strict posture it
        // installs accepts the entry instead of dropping it.
        assert!(
            entry.signature.is_some(),
            "CLI should attach a dev-key signature to locally-loaded token metadata"
        );
        assert!(entry.origin_chain.is_none());
    }

    /// Mappings alone are enough to produce metadata; the network flag is
    /// independent of them.
    #[test]
    fn create_metadata_carries_mappings_without_a_network() {
        let path = write_temp_json("net_check.json", r#"{"symbol":"X","decimals":6}"#);
        let mappings = vec![format!("X@{}@nep141:x.near", path.display())];

        let near = near_metadata(
            plugin_with_mappings(mappings)
                .create_metadata(None)
                .unwrap()
                .expect("Some(ChainMetadata)"),
        );
        assert!(near.network_id.is_none());
        assert_eq!(near.token_mappings.len(), 1);
    }

    #[test]
    fn create_metadata_skips_an_unreadable_mapping_file() {
        let mappings = vec!["Bad@/nonexistent/token.json@nep141:missing.near".to_string()];
        assert_eq!(
            plugin_with_mappings(mappings)
                .create_metadata(None)
                .unwrap(),
            None,
            "every mapping failing to load leaves nothing to carry"
        );
    }

    #[test]
    fn create_metadata_carries_multiple_mappings() {
        let path_a = write_temp_json("a.json", r#"{"symbol":"A","decimals":6}"#);
        let path_b = write_temp_json("b.json", r#"{"symbol":"B","decimals":18}"#);
        let mappings = vec![
            format!("A@{}@nep141:a.near", path_a.display()),
            format!("B@{}@nep141:b.near", path_b.display()),
        ];

        let near = near_metadata(
            plugin_with_mappings(mappings)
                .create_metadata(None)
                .unwrap()
                .expect("Some(ChainMetadata)"),
        );
        assert_eq!(near.token_mappings.len(), 2);
        assert!(near.token_mappings.contains_key("nep141:a.near"));
        assert!(near.token_mappings.contains_key("nep141:b.near"));
    }

    /// Regression coverage for the reason `@` is the separator: an asset id
    /// with an embedded colon must survive intact, not get truncated at the
    /// first colon the way the shared colon-delimited mapping format would.
    #[test]
    fn asset_id_with_embedded_colon_is_preserved() {
        let path = write_temp_json("colon.json", r#"{"symbol":"X","decimals":6}"#);
        let asset_id = "nep141:x.factory.bridge.near";
        let mappings = vec![format!("X@{}@{asset_id}", path.display())];

        let near = near_metadata(
            plugin_with_mappings(mappings)
                .create_metadata(None)
                .unwrap()
                .expect("Some(ChainMetadata)"),
        );
        assert!(
            near.token_mappings.contains_key(asset_id),
            "the full asset id (including its colon) must be the map key"
        );
    }

    #[test]
    fn create_metadata_keeps_valid_mappings_alongside_invalid_ones() {
        let path = write_temp_json("good.json", r#"{"symbol":"GOOD","decimals":6}"#);
        let mappings = vec![
            "bad-format-no-at-signs".to_string(),
            format!("Good@{}@nep141:good.near", path.display()),
            "Also@/missing/file.json@nep141:bad.near".to_string(),
        ];

        let near = near_metadata(
            plugin_with_mappings(mappings)
                .create_metadata(None)
                .unwrap()
                .expect("Some(ChainMetadata)"),
        );
        assert_eq!(near.token_mappings.len(), 1);
        assert!(near.token_mappings.contains_key("nep141:good.near"));
    }

    #[test]
    fn parse_near_mapping_accepts_the_three_field_format() {
        let result = parse_near_mapping("MyToken@/path/to/file.json@nep141:wrap.near")
            .expect("valid mapping should parse");
        assert_eq!(result.name, "MyToken");
        assert_eq!(result.path, "/path/to/file.json");
        assert_eq!(result.identifier, "nep141:wrap.near");
    }

    #[test]
    fn parse_near_mapping_rejects_a_malformed_mapping() {
        assert!(parse_near_mapping("NoAtSigns").is_err());
        assert!(parse_near_mapping("OnlyOne@AtSign").is_err());
        assert!(parse_near_mapping("@@EmptyComponents").is_err());
    }

    #[test]
    fn parse_near_mapping_rejects_an_at_sign_in_the_path() {
        // Previously absorbed into the asset id, yielding path "/tmp/my" and
        // asset id "dir/token.json@nep141:wrap.near" with no complaint.
        let err = parse_near_mapping("MyToken@/tmp/my@dir/token.json@nep141:wrap.near")
            .expect_err("a path containing '@' must be rejected");
        assert!(err.contains("expected 3"), "{err}");
        assert!(err.contains("cannot be used here"), "{err}");
    }

    #[test]
    fn build_token_mappings_skips_a_file_whose_json_is_invalid() {
        // Distinct from the file-not-found path: the file opens and reads, and
        // the failure comes from serde_json rejecting its contents.
        let dir = std::env::temp_dir().join("vsp_near_cli_plugin_invalid_json");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("broken.json");
        std::fs::write(&path, "{").expect("write");

        let mapping = format!("Broken@{}@nep141:wrap.near", path.display());
        let (map, valid) = build_token_mappings_from_files(&[mapping], NearNetwork::default());
        assert!(map.is_empty(), "an unparseable file must register nothing");
        assert_eq!(valid, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An `execute_intents` transaction withdrawing the token behind `asset_id`,
    /// as raw borsh hex.
    fn intents_tx_hex(asset_id: &str) -> String {
        let token = asset_id
            .split_once(':')
            .map_or(asset_id, |(_, account)| account);
        let inner = format!(
            r#"{{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{{"intent":"ft_withdraw","token":"{token}","receiver_id":"bob.near","amount":"1000000"}}]}}"#
        );
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});
        let tx = Transaction::V0(TransactionV0 {
            signer_id: "alice.near".parse().unwrap(),
            public_key: PublicKey::empty(KeyType::ED25519),
            nonce: 1,
            receiver_id: "intents.near".parse().unwrap(),
            block_hash: CryptoHash::default(),
            actions: vec![Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "execute_intents".to_string(),
                args: serde_json::to_vec(&args).unwrap(),
                gas: Gas::from_gas(30_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }))],
        });
        hex::encode(borsh::to_vec(&tx).unwrap())
    }

    /// The posture must be pinned through `register`, not through the helper that
    /// feeds it.
    ///
    /// Asserting on `cli_trust_policy()` in isolation proves nothing about what the
    /// CLI actually runs: editing `register` back to
    /// `NearVisualSignConverter::new()` leaves such an assertion passing and
    /// silently reverts the CLI to accept-unsigned. So this goes through the
    /// plugin, converts a real transaction, and observes the decode.
    ///
    /// `test-token.near` is deliberately not in `tokens::SEEDS`, so the unsigned
    /// metadata entry is the only thing that could resolve its symbol. The
    /// accept-unsigned control converter proves the fixture is good, so the
    /// registered converter's refusal cannot be a false negative.
    #[test]
    fn test_register_installs_require_signed_posture() {
        let asset_id = "nep141:test-token.near";

        // Deliberately unsigned: this is the entry the CLI's posture must refuse.
        let build_options = || VisualSignOptions {
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                    network_id: Some("NEAR_MAINNET".to_string()),
                    token_mappings: [(
                        asset_id.to_string(),
                        TokenMetadataEntry {
                            value: r#"{"symbol":"UNVERIFIED","decimals":6}"#.to_string(),
                            signature: None,
                            origin_chain: None,
                        },
                    )]
                    .into_iter()
                    .collect(),
                })),
            }),
            ..VisualSignOptions::default()
        };

        // Control: a converter on the permissive posture resolves the same fixture.
        let mut permissive_registry = TransactionConverterRegistry::new();
        permissive_registry.register::<crate::NearTransaction, _>(
            Chain::Near,
            crate::NearVisualSignConverter::new(),
        );
        let permissive = permissive_registry
            .convert_transaction(&Chain::Near, &intents_tx_hex(asset_id), build_options())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(
            permissive.contains("UNVERIFIED"),
            "accept-unsigned must resolve the unsigned metadata symbol, got: {permissive}"
        );

        // The converter the CLI plugin actually installs must refuse it.
        let mut registry = TransactionConverterRegistry::new();
        plugin().register(&mut registry);
        let rendered = registry
            .convert_transaction(&Chain::Near, &intents_tx_hex(asset_id), build_options())
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(
            !rendered.contains("UNVERIFIED"),
            "the converter `register` installs must not resolve an unsigned metadata symbol, got: {rendered}"
        );
        assert!(
            rendered.contains(&format!("unresolved {asset_id}")),
            "dropping the unsigned entry must leave the raw asset id unresolved, got: {rendered}"
        );
    }

    /// End-to-end gate on the whole CLI path: the flag builds an entry, the dev
    /// key signs it, and the strict posture `register` installs accepts it.
    ///
    /// Each half alone is insufficient. `create_metadata` asserting
    /// `signature.is_some()` says nothing about whether that signature verifies,
    /// and the posture test above only proves an unsigned entry is refused. Drop
    /// the dev key from `authorized_token_metadata_signers`, or sign under the
    /// wrong domain tag, and both still pass while the flag resolves nothing.
    #[test]
    fn cli_signed_mapping_resolves_through_the_registered_converter() {
        let asset_id = "nep141:test-token.near";
        let path = write_temp_json("e2e.json", r#"{"symbol":"CLISIGNED","decimals":6}"#);
        let mappings = vec![format!("Test@{}@{asset_id}", path.display())];
        let plugin = plugin_with_mappings(mappings);

        let metadata = plugin
            .create_metadata(Some("NEAR_MAINNET".to_string()))
            .unwrap()
            .expect("Some(ChainMetadata)");

        let mut registry = TransactionConverterRegistry::new();
        plugin.register(&mut registry);
        let rendered = registry
            .convert_transaction(
                &Chain::Near,
                &intents_tx_hex(asset_id),
                VisualSignOptions {
                    metadata: Some(metadata),
                    ..VisualSignOptions::default()
                },
            )
            .unwrap()
            .payload
            .to_json()
            .unwrap();
        assert!(
            rendered.contains("CLISIGNED"),
            "a CLI-signed entry must resolve under the posture `register` installs, got: {rendered}"
        );
    }
}
