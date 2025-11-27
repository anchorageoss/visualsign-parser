//! Registry module for managing type definitions and lookups

// TODO(pg): this may not be the right place for this
/// Creates and configures a new transaction converter registry with all supported chains.
///
/// Returns a registry with converters for Solana and Unspecified transaction types.
#[must_use]
pub fn create_registry() -> visualsign::registry::TransactionConverterRegistry {
    let mut registry = visualsign::registry::TransactionConverterRegistry::new();
    // TODO: Create a ChainRegistry trait that all chains can implement for token metadata,
    // contract types, etc. Currently only Ethereum has a ContractRegistry.
    registry.register::<visualsign_ethereum::EthereumTransactionWrapper, _>(
        visualsign::registry::Chain::Ethereum,
        visualsign_ethereum::EthereumVisualSignConverter::new(),
    );
    registry.register::<visualsign_solana::SolanaTransactionWrapper, _>(
        visualsign::registry::Chain::Solana,
        visualsign_solana::SolanaVisualSignConverter,
    );
    registry.register::<visualsign_sui::SuiTransactionWrapper, _>(
        visualsign::registry::Chain::Sui,
        visualsign_sui::SuiVisualSignConverter,
    );
    registry.register::<visualsign_tron::TronTransactionWrapper, _>(
        visualsign::registry::Chain::Tron,
        visualsign_tron::TronVisualSignConverter,
    );
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        visualsign::registry::Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}
