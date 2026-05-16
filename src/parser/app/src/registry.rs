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
    #[cfg(feature = "unspecified")]
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        visualsign::registry::Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}
