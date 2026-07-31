//! Registry module for managing type definitions and lookups

// TODO(pg): this may not be the right place for this
/// Creates and configures a new transaction converter registry.
///
/// Returns a registry containing a converter for each chain enabled via Cargo
/// features (see `[features]` in `parser_app/Cargo.toml`). Chains whose
/// feature is disabled are omitted; requests for those chains hit the
/// registry-miss path in `convert_transaction` and surface as
/// `InvalidArgument` at the gRPC layer.
#[must_use]
pub fn create_registry() -> visualsign::registry::TransactionConverterRegistry {
    #[allow(unused_mut)] // mut is unused when no chain features are enabled
    let mut registry = visualsign::registry::TransactionConverterRegistry::new();
    // TODO: Create a ChainRegistry trait that all chains can implement for token metadata,
    // contract types, etc. Currently only Ethereum has a ContractRegistry.
    #[cfg(feature = "ethereum")]
    registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
        visualsign::registry::Chain::Ethereum,
        visualsign_ethereum::EthereumVisualSignConverter::new(),
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

    /// Pins the ABI trust posture the deployed binary actually runs.
    ///
    /// `create_registry` is the only production construction site for the Ethereum
    /// converter, and which posture it installs is decided purely by which
    /// constructor this file calls. Moving between `new()` (accept-unsigned) and
    /// `with_policy(RequireAllowlistedSigner(..))` compiles clean and, without this
    /// test, leaves the whole suite green, so nothing would tell a reviewer that the
    /// production posture had changed.
    ///
    /// Today it is accept-unsigned, deliberately: this is the posture the enclave
    /// binary has always effectively run, since the compiled-in allowlist is empty
    /// there and an unsigned entry was already accepted. The deploy-time flag that
    /// lets an operator choose require-signed is a follow-up; when it lands, this
    /// assertion is the one that has to be updated on purpose, which is the point.
    #[test]
    fn create_registry_runs_the_accept_unsigned_abi_posture() {
        let rendered = super::create_registry()
            .convert_transaction(
                &Chain::Ethereum,
                CUSTOM_FOO_TX_HEX,
                options_with_unsigned_abi(),
            )
            .expect("fixture transaction must convert")
            .payload
            .to_json()
            .expect("payload must serialize");

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
}
