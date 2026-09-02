//! Registry module for managing type definitions and lookups

// TODO(pg): this may not be the right place for this
/// Creates and configures a new transaction converter registry.
///
/// Returns a registry containing a converter for each chain enabled via Cargo
/// features (see `[features]` in `parser_app/Cargo.toml`). Chains whose
/// feature is disabled are omitted; requests for those chains hit the
/// registry-miss path in `convert_transaction` and surface as
/// `InvalidArgument` at the gRPC layer.
///
/// `config` carries the deploy-time settings the converters need, chiefly the ABI
/// trust posture. It is threaded in rather than read from a global so the posture
/// is set once at startup and cannot vary between requests.
#[must_use]
#[cfg_attr(not(feature = "ethereum"), allow(unused_variables))]
pub fn create_registry(
    config: &crate::config::ParserConfig,
) -> visualsign::registry::TransactionConverterRegistry {
    #[allow(unused_mut)] // mut is unused when no chain features are enabled
    let mut registry = visualsign::registry::TransactionConverterRegistry::new();
    // TODO: Create a ChainRegistry trait that all chains can implement for token metadata,
    // contract types, etc. Currently only Ethereum has a ContractRegistry.
    #[cfg(feature = "ethereum")]
    registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
        visualsign::registry::Chain::Ethereum,
        visualsign_ethereum::EthereumVisualSignConverter::with_policy(config.abi_trust.clone()),
    );
    #[cfg(feature = "solana")]
    registry.register::<visualsign_solana::SolanaTransactionWrapper, _>(
        visualsign::registry::Chain::Solana,
        visualsign_solana::SolanaVisualSignConverter,
    );
    #[cfg(feature = "sui")]
    registry.register::<visualsign_sui::SuiTransactionWrapper, _>(
        visualsign::registry::Chain::Sui,
        visualsign_sui::SuiVisualSignConverter,
    );
    #[cfg(feature = "tron")]
    registry.register::<visualsign_tron::TronTransactionWrapper, _>(
        visualsign::registry::Chain::Tron,
        visualsign_tron::TronVisualSignConverter,
    );
    #[cfg(feature = "near")]
    registry.register::<visualsign_near::NearTransaction, _>(
        visualsign::registry::Chain::Near,
        visualsign_near::NearVisualSignConverter::new(),
    );
    #[cfg(feature = "unspecified")]
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        visualsign::registry::Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}

#[cfg(all(test, feature = "ethereum"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use generated::parser::{Abi, ChainMetadata, EthereumMetadata, chain_metadata};
    use std::collections::BTreeMap;
    use visualsign::registry::Chain;
    use visualsign::vsptrait::VisualSignOptions;

    /// EIP-1559 transaction to `0x1111..1111` calling `customFoo(uint256)` with
    /// `x = 7`, chain 1, unsigned. `customFoo` is deliberately a function no
    /// built-in visualizer knows, so a caller-supplied metadata ABI is the only
    /// thing that can decode it: whether the selector or the function name renders
    /// is a direct read of the trust posture. RLP is deterministic, so this is a
    /// fixed string rather than a reason to pull alloy into this crate's dev-deps.
    const CUSTOM_FOO_TX_HEX: &str = "0x02f84b0180830f4240843b9aca00830186a094111111111111111111111111111111111111111180a46ab6fefe0000000000000000000000000000000000000000000000000000000000000007c0";
    const CUSTOM_FOO_SELECTOR: &str = "6ab6fefe";

    fn options_with_unsigned_abi() -> VisualSignOptions {
        let mut abi_mappings = BTreeMap::new();
        abi_mappings.insert(
            "0x1111111111111111111111111111111111111111".to_string(),
            Abi {
                value: r#"[{
                    "type": "function",
                    "name": "customFoo",
                    "inputs": [{"name": "x", "type": "uint256"}],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                }]"#
                .to_string(),
                signature: None,
                ..Default::default()
            },
        );
        VisualSignOptions {
            include_intermediate_output: false,
            decode_transfers: true,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("ETHEREUM_MAINNET".to_string()),
                    abi_mappings: abi_mappings.into_iter().collect(),
                })),
            }),
            developer_config: None,
        }
    }

    /// Builds a registry under the given policy and renders the fixture tx, for the
    /// two tests below that assert opposite outcomes for the same conversion.
    fn render_custom_foo(policy: visualsign::signing::MetadataTrustPolicy) -> String {
        let config = crate::config::ParserConfig::new(policy);
        super::create_registry(&config)
            .convert_transaction(
                &Chain::Ethereum,
                CUSTOM_FOO_TX_HEX,
                options_with_unsigned_abi(),
            )
            .expect("fixture transaction must convert")
            .payload
            .to_json()
            .expect("payload must serialize")
    }

    /// Pins that `create_registry` forwards a supplied permissive policy to the
    /// converter, rather than silently dropping it.
    ///
    /// `create_registry` is the only production construction site for the Ethereum
    /// converter, and which posture it installs is decided purely by the
    /// [`ParserConfig`](crate::config::ParserConfig) this file forwards into
    /// `with_policy`. Moving between `AcceptUnsigned` and
    /// `RequireAllowlistedSigner(..)` compiles clean and, without this test,
    /// leaves the whole suite green, so nothing would tell a reviewer that the
    /// wiring broke.
    ///
    /// The posture is a deploy-time choice the operator makes via the `parser_app`
    /// CLI flags (the CLI may construct either policy; this test does not pin
    /// which one the deployed binary actually runs). This test builds the config
    /// explicitly with `AcceptUnsigned` and asserts the unsigned caller ABI
    /// decodes, exercising the permissive branch of that wiring. The mirror test
    /// below exercises the restrictive branch.
    #[test]
    fn create_registry_runs_the_accept_unsigned_abi_posture() {
        let rendered = render_custom_foo(visualsign::signing::MetadataTrustPolicy::AcceptUnsigned);

        assert!(
            rendered.contains("customFoo"),
            "create_registry is expected to run accept-unsigned, so the unsigned \
             caller ABI must decode. If this now fails because the posture was \
             tightened on purpose, update this test; if it fails unexpectedly, the \
             production posture moved without anyone deciding to. Got: {rendered}"
        );
        assert!(
            !rendered.contains(CUSTOM_FOO_SELECTOR),
            "a decoded call must not fall back to the raw selector, got: {rendered}"
        );
    }

    /// The mirror of the test above, for the posture that actually enforces something.
    ///
    /// Asserting only the accept-unsigned direction would leave the security-relevant
    /// wiring unpinned: a `create_registry` that dropped `config.abi_trust` and always
    /// built a permissive converter would still pass the accept-unsigned test. Here the
    /// same unsigned fixture must be refused and fall back to the raw selector, so the
    /// posture has to reach the converter for the test to hold.
    #[test]
    fn create_registry_runs_the_require_signed_abi_posture() {
        let rendered = render_custom_foo(
            visualsign::signing::MetadataTrustPolicy::RequireAllowlistedSigner(
                visualsign::signing::SignerAllowlist::new(),
            ),
        );

        assert!(
            !rendered.contains("customFoo"),
            "require-signed must drop the unsigned caller ABI; decoding it means the \
             posture never reached the converter. Got: {rendered}"
        );
        assert!(
            rendered.contains(CUSTOM_FOO_SELECTOR),
            "dropping the ABI must leave the raw selector {CUSTOM_FOO_SELECTOR}, got: {rendered}"
        );
    }
}
