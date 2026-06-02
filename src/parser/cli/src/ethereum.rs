use std::collections::{HashMap, HashSet};

use clap::Args as ClapArgs;
use generated::parser::{Abi, AbiType, ChainMetadata, EthereumMetadata, chain_metadata::Metadata};
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign_ethereum::abi_metadata::{CLI_DEV_SIGNING_KEY_SEED, sign_abi};
use visualsign_ethereum::networks::parse_network;
use visualsign_ethereum::token_metadata::parse_network_id;

use crate::mapping_parser;

/// CLI arguments specific to Ethereum.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct EthereumArgs {
    /// Map custom ABI JSON file to contract address.
    /// Format: `AbiName:/path/to/abi.json:0xAddress`. Can be used multiple times.
    #[arg(
        long = "abi-json-mappings",
        value_name = "ABI_NAME:FILE_PATH:0xADDRESS"
    )]
    pub abi_json_mappings: Vec<String>,

    /// Declare that a proxy address delegates to an implementation address. The
    /// proxy's calldata is decoded against the implementation's ABI. Both addresses
    /// should also be supplied via `--abi-json-mappings` (a proxy with no ABI file
    /// gets an empty ABI synthesized). Format: `0xProxy:0xImpl`. Repeatable.
    #[arg(long = "abi-proxy-mappings", value_name = "0xPROXY:0xIMPL")]
    pub abi_proxy_mappings: Vec<String>,
}

/// [`crate::ChainPlugin`] implementation for Ethereum.
pub struct EthereumPlugin {
    args: EthereumArgs,
}

impl EthereumPlugin {
    /// Creates a new `EthereumPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: EthereumArgs) -> Self {
        Self { args }
    }
}

impl crate::ChainPlugin for EthereumPlugin {
    fn chain(&self) -> Chain {
        Chain::Ethereum
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
            Chain::Ethereum,
            visualsign_ethereum::EthereumVisualSignConverter::new(),
        );
    }

    fn create_metadata(&self, network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        create_chain_metadata(
            network,
            &self.args.abi_json_mappings,
            &self.args.abi_proxy_mappings,
        )
    }
}

/// Validate an Ethereum address string: `0x` prefix + 40 hex chars.
fn validate_eth_address(addr: &str) -> Result<(), String> {
    let hex = addr
        .strip_prefix("0x")
        .or_else(|| addr.strip_prefix("0X"))
        .ok_or("must start with 0x")?;
    if hex.len() != 40 {
        return Err(format!(
            "expected 0x + 40 hex chars, got {} hex chars",
            hex.len()
        ));
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("contains non-hex characters".to_string());
    }
    Ok(())
}

/// Normalize an Ethereum address to lowercase `0x` + 40 hex chars.
///
/// Callers must validate the address with [`validate_eth_address`] before calling
/// this function. The returned string is always `0x<40 lowercase hex chars>`.
fn normalize_eth_address(addr: &str) -> String {
    // Strip the prefix (0x or 0X) and re-attach lowercase 0x.
    let hex = addr
        .strip_prefix("0x")
        .or_else(|| addr.strip_prefix("0X"))
        .unwrap_or(addr);
    format!("0x{}", hex.to_ascii_lowercase())
}

/// Load ABI JSON files and build mappings for `EthereumMetadata.abi_mappings`.
///
/// Address keys are normalized to lowercase so they match regardless of the
/// checksum casing the user supplied (e.g. `0xAbCd...` and `0xabcd...` both
/// produce the same key). The [`validate_eth_address`] validator runs first, so
/// [`normalize_eth_address`] can safely strip the prefix without re-checking.
///
/// `chain_id` is the numeric chain id the metadata targets; it is bound into each
/// ABI signature (alongside the contract address) so the signature matches what the
/// parser verifies for this (chain, address).
fn build_abi_mappings_from_files(
    abi_json_mappings: &[String],
    chain_id: u64,
) -> (HashMap<String, Abi>, usize) {
    let (raw, count) = mapping_parser::load_mappings(
        abi_json_mappings,
        "ABI",
        "UniswapV2:path/to/uniswap.json:0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f",
        "ContractAddress",
        validate_eth_address,
        |components, json| {
            // The metadata-ABI extraction path rejects unsigned entries,
            // so the CLI attaches an integrity signature using a deterministic local
            // dev key. This is integrity, not identity, the CLI is a local dev tool
            // that already trusts its input files; production trust comes from the
            // gRPC caller verifying the public key against an allowlist.
            //
            // The signature binds the contract address and chain id, so it must be
            // produced for the same (chain, address) the parser verifies with. The
            // parser uses the entry's map-key address (normalized to lowercase, but
            // hex parsing is case-insensitive so the signed bytes match) and the
            // resolved chain id, which for the CLI flow is this `chain_id`.
            //
            // If parsing or signing fails (e.g. an invalid seed in future refactors),
            // surface the failure as a `load_mappings` rejection so the entry is
            // skipped and the success count stays accurate, rather than emitting an
            // `Abi` that the extractor would silently drop later.
            let addr = components
                .identifier
                .parse::<alloy_primitives::Address>()
                .map_err(|e| format!("invalid contract address: {e}"))?;
            let signature = sign_abi(&json, &addr, chain_id, &CLI_DEV_SIGNING_KEY_SEED)
                .map_err(|e| format!("failed to sign ABI: {e}"))?;
            Ok(Abi {
                value: json,
                signature: Some(signature),
                ..Default::default()
            })
        },
    );
    let normalized = raw
        .into_iter()
        .map(|(addr, abi)| (normalize_eth_address(&addr), abi))
        .collect();
    (normalized, count)
}

/// Apply `--abi-proxy-mappings` links onto the loaded ABI mappings.
///
/// Each `0xProxy:0xImpl` entry stamps `abi_type = ABI_TYPE_PROXY` and
/// `implementation_address` onto the proxy's `Abi`. If the proxy address has no ABI
/// file entry, an empty (`[]`) ABI is synthesized so resolution still works off the
/// link. Malformed links are skipped with a warning.
///
/// `abi_json_mappings` is the original `--abi-json-mappings` list; it is used to
/// emit a louder warning when a proxy's ABI file was specified but failed to load
/// (vs. simply never having an ABI file specified at all).
///
/// `chain_id` is the numeric chain id the metadata targets; it is bound into the
/// synthesized proxy ABI's signature (alongside the proxy address) so the parser
/// accepts it for this (chain, address).
fn apply_proxy_mappings(
    abi_mappings: &mut HashMap<String, Abi>,
    proxy_mappings: &[String],
    abi_json_mappings: &[String],
    chain_id: u64,
) {
    // Pre-compute the set of addresses that were attempted in --abi-json-mappings.
    // Used below to distinguish "ABI file was specified but failed" from "no ABI file at all".
    let attempted_abi_addresses: HashSet<String> = abi_json_mappings
        .iter()
        .filter_map(|m| mapping_parser::parse_mapping(m).ok())
        .filter(|c| validate_eth_address(&c.identifier).is_ok())
        .map(|c| normalize_eth_address(&c.identifier))
        .collect();

    for mapping in proxy_mappings {
        let Some((proxy, implementation)) = mapping.split_once(':') else {
            eprintln!(
                "  Warning: Skipping proxy mapping '{mapping}': expected format 0xProxy:0xImpl"
            );
            continue;
        };
        if let Err(e) = validate_eth_address(proxy) {
            eprintln!("  Warning: Skipping proxy mapping '{mapping}': invalid proxy address: {e}");
            continue;
        }
        if let Err(e) = validate_eth_address(implementation) {
            eprintln!(
                "  Warning: Skipping proxy mapping '{mapping}': invalid implementation address: {e}"
            );
            continue;
        }

        // Normalize to lowercase so the lookup matches the key written by
        // `build_abi_mappings_from_files` regardless of checksum casing.
        let proxy_key = normalize_eth_address(proxy);
        let impl_key = normalize_eth_address(implementation);

        // Ensure the proxy has an entry to stamp. If it has no own ABI file, synthesize
        // an empty "[]" ABI. The metadata-ABI extraction path rejects unsigned entries,
        // so the synthesized ABI is signed with the same dev key used for file-loaded
        // ABIs; otherwise the extractor would silently drop the proxy and the
        // proxy->implementation link would be lost.
        if !abi_mappings.contains_key(&proxy_key) {
            if attempted_abi_addresses.contains(&proxy_key) {
                eprintln!(
                    "  Warning: proxy '{proxy_key}' ABI file was specified but failed to load; \
                     synthesizing an empty proxy ABI"
                );
            } else {
                eprintln!(
                    "  Note: proxy '{proxy_key}' has no ABI file; synthesizing an empty proxy ABI"
                );
            }
            let empty_abi = "[]".to_string();
            // `proxy_key` is lowercase-normalized valid hex; parse it to bind the
            // signature to the proxy address (hex parsing is case-insensitive, so
            // the signed bytes match what the parser verifies for this entry).
            let proxy_addr = match proxy_key.parse::<alloy_primitives::Address>() {
                Ok(addr) => addr,
                Err(e) => {
                    eprintln!(
                        "  Warning: Skipping proxy mapping '{mapping}': \
                         invalid proxy address for signing: {e}"
                    );
                    continue;
                }
            };
            let signature =
                match sign_abi(&empty_abi, &proxy_addr, chain_id, &CLI_DEV_SIGNING_KEY_SEED) {
                    Ok(sig) => sig,
                    Err(e) => {
                        eprintln!(
                            "  Warning: Skipping proxy mapping '{mapping}': \
                             failed to sign synthesized proxy ABI: {e}"
                        );
                        continue;
                    }
                };
            abi_mappings.insert(
                proxy_key.clone(),
                Abi {
                    value: empty_abi,
                    signature: Some(signature),
                    ..Default::default()
                },
            );
        }

        let Some(entry) = abi_mappings.get_mut(&proxy_key) else {
            // Unreachable: the key was just ensured to exist. Skip rather than
            // panic so a future refactor that breaks this invariant degrades
            // gracefully instead of crashing the CLI.
            continue;
        };
        entry.abi_type = Some(AbiType::Proxy as i32);
        entry.implementation_address = Some(impl_key.clone());
        eprintln!("  Linked proxy '{proxy_key}' to implementation '{impl_key}'");
    }
}

/// Creates Ethereum chain metadata from the network argument.
/// Defaults to `ETHEREUM_MAINNET` if no network is specified.
/// Returns an error if the network identifier is invalid.
///
/// # Panics
///
/// Panics if `ETHEREUM_MAINNET` cannot be parsed (should never happen).
pub(crate) fn create_chain_metadata(
    network: Option<String>,
    abi_json_mappings: &[String],
    abi_proxy_mappings: &[String],
) -> Result<Option<ChainMetadata>, String> {
    let network_id = if let Some(network) = network {
        let Some(network_id) = parse_network(&network) else {
            return Err(format!(
                "Invalid network '{network}'. Supported formats:\n\
                 - Chain ID (numeric): 1 (Ethereum), 137 (Polygon), 42161 (Arbitrum)\n\
                 - Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET\n\
                 \n\
                 Run with --help for full list of supported networks."
            ));
        };
        network_id
    } else {
        eprintln!("Warning: No network specified, defaulting to ETHEREUM_MAINNET (chain_id: 1)");
        parse_network("ETHEREUM_MAINNET").expect("ETHEREUM_MAINNET should always be valid")
    };

    // The ABI signature binds the chain id, so the numeric chain id is only needed
    // when there is at least one ABI to sign. Derive it lazily inside each branch so
    // the no-ABI path still works for networks `parse_network_id` does not number
    // (it only knows the canonical mainnets). The double lookup when both lists are
    // non-empty is cheap and avoids sharing an `Option` across the lint-restricted
    // borrow.
    let mut abi_mappings = if abi_json_mappings.is_empty() {
        HashMap::new()
    } else {
        let chain_id = parse_network_id(&network_id)
            .map_err(|e| format!("cannot sign ABI mappings for network '{network_id}': {e}"))?;
        eprintln!("Loading custom ABIs:");
        let (mappings, valid_count) = build_abi_mappings_from_files(abi_json_mappings, chain_id);
        eprintln!(
            "Successfully loaded {}/{} ABI mappings\n",
            valid_count,
            abi_json_mappings.len()
        );
        mappings
    };

    if !abi_proxy_mappings.is_empty() {
        let chain_id = parse_network_id(&network_id).map_err(|e| {
            format!("cannot sign proxy ABI mappings for network '{network_id}': {e}")
        })?;
        eprintln!("Applying proxy mappings:");
        apply_proxy_mappings(
            &mut abi_mappings,
            abi_proxy_mappings,
            abi_json_mappings,
            chain_id,
        );
        eprintln!();
    }

    Ok(Some(ChainMetadata {
        metadata: Some(Metadata::Ethereum(EthereumMetadata {
            network_id: Some(network_id),
            abi_mappings,
        })),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn write_temp_json(name: &str, content: &str) -> std::path::PathBuf {
        crate::test_utils::write_temp_json("vsp_eth_tests", name, content)
    }

    #[test]
    fn test_create_chain_metadata_defaults_to_mainnet() {
        let meta = create_chain_metadata(None, &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ETHEREUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_network_name() {
        let meta = create_chain_metadata(Some("POLYGON_MAINNET".to_string()), &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "POLYGON_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_with_chain_id() {
        let meta = create_chain_metadata(Some("42161".to_string()), &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.network_id.unwrap(), "ARBITRUM_MAINNET");
    }

    #[test]
    fn test_create_chain_metadata_empty_abi_mappings() {
        let meta = create_chain_metadata(None, &[], &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert!(eth.abi_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_with_abi_mappings() {
        let path = write_temp_json("eth_abi.json", r#"[{"type":"function","name":"swap"}]"#);
        let mappings = vec![format!(
            "Token:{}:0xdAC17F958D2ee523a2206206994597C13D831ec7",
            path.display()
        )];

        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 1);
        // Address keys are normalized to lowercase regardless of input casing.
        let abi = eth
            .abi_mappings
            .get("0xdac17f958d2ee523a2206206994597c13d831ec7")
            .expect("mapping present");
        assert!(abi.value.contains("swap"));
        // CLI signs locally-loaded ABIs so the metadata-ABI extractor
        // (which rejects unsigned entries) can register them.
        assert!(
            abi.signature.is_some(),
            "CLI should attach a dev-key signature to locally-loaded ABIs"
        );
    }

    #[test]
    fn test_create_chain_metadata_invalid_abi_file_skipped() {
        let mappings = vec![
            "BadABI:/nonexistent/abi.json:0xdAC17F958D2ee523a2206206994597C13D831ec7".to_string(),
        ];
        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert!(eth.abi_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_multiple_abi_mappings() {
        let path1 = write_temp_json("abi_a.json", r#"[{"type":"function","name":"a"}]"#);
        let path2 = write_temp_json("abi_b.json", r#"[{"type":"function","name":"b"}]"#);
        let mappings = vec![
            format!(
                "A:{}:0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                path1.display()
            ),
            format!(
                "B:{}:0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                path2.display()
            ),
        ];

        let meta = create_chain_metadata(None, &mappings, &[])
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        assert_eq!(eth.abi_mappings.len(), 2);
        // Address keys are normalized to lowercase regardless of input casing.
        assert!(
            eth.abi_mappings
                .contains_key("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
        );
        assert!(
            eth.abi_mappings
                .contains_key("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2")
        );
    }

    #[test]
    fn test_create_chain_metadata_invalid_network_returns_error() {
        let result = create_chain_metadata(Some("INVALID_NETWORK".to_string()), &[], &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid network"));
    }

    const PROXY: &str = "0x1111111111111111111111111111111111111111";
    const IMPL: &str = "0x2222222222222222222222222222222222222222";

    #[test]
    fn test_proxy_mapping_stamps_type_on_existing_entry() {
        let proxy_path =
            write_temp_json("proxy.json", r#"[{"type":"function","name":"upgradeTo"}]"#);
        let impl_path = write_temp_json("impl.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![
            format!("Proxy:{}:{PROXY}", proxy_path.display()),
            format!("Impl:{}:{IMPL}", impl_path.display()),
        ];
        let proxy_links = vec![format!("{PROXY}:{IMPL}")];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };

        let proxy_abi = eth.abi_mappings.get(PROXY).expect("proxy entry present");
        assert_eq!(proxy_abi.abi_type, Some(AbiType::Proxy as i32));
        assert_eq!(proxy_abi.implementation_address.as_deref(), Some(IMPL));
        // The proxy keeps its own ABI file content.
        assert!(proxy_abi.value.contains("upgradeTo"));

        // The implementation entry is untouched (defaults to implementation).
        let impl_abi = eth.abi_mappings.get(IMPL).expect("impl entry present");
        assert_eq!(impl_abi.abi_type, None);
    }

    #[test]
    fn test_proxy_mapping_synthesizes_entry_when_no_abi_file() {
        // Only the implementation has an ABI file; the proxy is declared purely
        // via --abi-proxy-mappings.
        let impl_path = write_temp_json("impl2.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![format!("Impl:{}:{IMPL}", impl_path.display())];
        let proxy_links = vec![format!("{PROXY}:{IMPL}")];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };

        let proxy_abi = eth
            .abi_mappings
            .get(PROXY)
            .expect("synthesized proxy entry");
        assert_eq!(proxy_abi.value, "[]");
        assert_eq!(proxy_abi.abi_type, Some(AbiType::Proxy as i32));
        assert_eq!(proxy_abi.implementation_address.as_deref(), Some(IMPL));
        // Regression guard: the synthesized proxy ABI must be signed. The
        // metadata-ABI extraction path rejects unsigned entries, so an unsigned
        // synthesized proxy would be silently dropped and the proxy->impl link lost.
        assert!(
            proxy_abi.signature.is_some(),
            "synthesized proxy ABI must be signed so the extractor accepts it",
        );

        // End-to-end: the signed synthesized proxy survives extraction. `list_abis`
        // returns each registered entry's address string; the proxy being present
        // proves it cleared the unsigned-rejection gate.
        let extracted = ChainMetadata {
            metadata: Some(Metadata::Ethereum(eth)),
        };
        let registry = visualsign_ethereum::abi_metadata::try_extract_from_chain_metadata(
            Some(&extracted),
            1,
            // dev-signing is enabled for the CLI, so this allowlists the dev key the
            // CLI signed the synthesized proxy with.
            &visualsign_ethereum::abi_metadata::authorized_abi_signers(),
        )
        .expect("metadata with a signed synthesized proxy must extract");
        assert!(
            registry.list_abis().contains(&PROXY),
            "signed synthesized proxy must survive extraction (unsigned would be dropped)",
        );
    }

    #[test]
    fn test_proxy_mapping_invalid_link_skipped() {
        let impl_path = write_temp_json("impl3.json", r#"[{"type":"function","name":"transfer"}]"#);
        let abi_mappings = vec![format!("Impl:{}:{IMPL}", impl_path.display())];
        // Malformed: missing impl, and bad proxy address.
        let proxy_links = vec!["0xnothex:0xalsobad".to_string(), "noseparator".to_string()];

        let meta = create_chain_metadata(None, &abi_mappings, &proxy_links)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };
        // No proxy entry was created; only the implementation remains.
        assert_eq!(eth.abi_mappings.len(), 1);
        assert!(eth.abi_mappings.contains_key(IMPL));
    }

    /// Mixed-casing between --abi-json-mappings and --abi-proxy-mappings must not
    /// silently lose the proxy link. The Copilot review comment identified a scenario
    /// where the user supplies the same address in different case across the two flags,
    /// causing a `HashMap` miss and a synthesized empty ABI overwriting the real one.
    #[test]
    fn test_proxy_mapping_mixed_case_links_correctly() {
        // Use a freshly created pair so the uppercase vs lowercase contrast is clear.
        let proxy_uc_path = write_temp_json(
            "proxy_uc.json",
            r#"[{"type":"function","name":"upgradeTo"}]"#,
        );
        let impl_uc_path =
            write_temp_json("impl_uc.json", r#"[{"type":"function","name":"transfer"}]"#);

        // Register the ABI with uppercase address in --abi-json-mappings.
        let abi_mappings_uc = vec![
            format!(
                "Proxy:{}:0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                proxy_uc_path.display()
            ),
            format!(
                "Impl:{}:0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                impl_uc_path.display()
            ),
        ];
        // Supply the same addresses in lowercase via --abi-proxy-mappings.
        let proxy_links_lc = vec![
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        ];

        let meta = create_chain_metadata(None, &abi_mappings_uc, &proxy_links_lc)
            .unwrap()
            .expect("should return Some");
        let Metadata::Ethereum(eth) = meta.metadata.unwrap() else {
            panic!("expected Ethereum metadata");
        };

        // Both entries must be present; no duplicate synthetic empty entry.
        assert_eq!(eth.abi_mappings.len(), 2);
        // Proxy must retain its ABI file content (not the synthesized empty ABI).
        let proxy_abi = eth
            .abi_mappings
            .get("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("proxy entry present");
        assert_eq!(proxy_abi.abi_type, Some(AbiType::Proxy as i32));
        assert!(
            proxy_abi.value.contains("upgradeTo"),
            "proxy ABI should contain the file content, not the synthesized empty ABI"
        );
        assert_eq!(
            proxy_abi.implementation_address.as_deref(),
            Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn test_normalize_eth_address() {
        assert_eq!(
            normalize_eth_address("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
            "0xdac17f958d2ee523a2206206994597c13d831ec7"
        );
        assert_eq!(
            normalize_eth_address("0xABCDEF1234567890abcdef1234567890ABCDEF12"),
            "0xabcdef1234567890abcdef1234567890abcdef12"
        );
        // Already lowercase is a no-op.
        assert_eq!(
            normalize_eth_address("0x1111111111111111111111111111111111111111"),
            "0x1111111111111111111111111111111111111111"
        );
    }
}
