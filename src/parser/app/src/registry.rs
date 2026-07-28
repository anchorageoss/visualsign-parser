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
    #[cfg(feature = "unspecified")]
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        visualsign::registry::Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}
