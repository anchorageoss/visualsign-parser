//! Registry module for managing type definitions and lookups

// TODO(pg): this may not be the right place for this
/// Creates and configures a new transaction converter registry with all supported chains.
///
/// Returns a registry with converters for Solana and Unspecified transaction types.
#[must_use]
pub fn create_registry() -> visualsign::registry::TransactionConverterRegistry {
    let mut registry = visualsign::registry::TransactionConverterRegistry::new();
    registry.register::<visualsign_solana::SolanaTransactionWrapper, _>(
        visualsign::registry::Chain::Solana,
        visualsign_solana::SolanaVisualSignConverter,
    );
    registry.register::<visualsign_unspecified::UnspecifiedTransactionWrapper, _>(
        visualsign::registry::Chain::Unspecified,
        visualsign_unspecified::UnspecifiedVisualSignConverter,
    );
    registry
}
