//! Conversion functions between the generated parser Chain enum and the visualsign registry Chain enum.
use visualsign::registry::Chain as RegistryChain;

use generated::parser::Chain as ProtoChain;

/// Converts a wire `Chain` to the registry's `Chain`.
///
/// `custom_chain_name` is required (non-empty) when `proto == ProtoChain::Custom`
/// — it names the `Chain::Custom(name)` a caller-supplied registry (see
/// `parser_app`'s `external-chains` feature) registered a converter under.
/// Every other `proto` value ignores it.
pub(crate) fn proto_to_registry(
    proto: ProtoChain,
    custom_chain_name: Option<&str>,
) -> Result<RegistryChain, String> {
    match proto {
        ProtoChain::Unspecified => Ok(RegistryChain::Unspecified),
        ProtoChain::Bitcoin => Ok(RegistryChain::Bitcoin),
        ProtoChain::Ethereum => Ok(RegistryChain::Ethereum),
        ProtoChain::Solana => Ok(RegistryChain::Solana),
        ProtoChain::Sui => Ok(RegistryChain::Sui),
        ProtoChain::Tron => Ok(RegistryChain::Tron),
        ProtoChain::Custom => match custom_chain_name {
            Some(name) if !name.is_empty() => Ok(RegistryChain::Custom(name.to_string())),
            _ => Err("chain is CHAIN_CUSTOM but custom_chain_name is missing or empty".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn registry_to_proto(registry: &RegistryChain) -> ProtoChain {
        match registry {
            RegistryChain::Unspecified => ProtoChain::Unspecified,
            RegistryChain::Bitcoin => ProtoChain::Bitcoin,
            RegistryChain::Ethereum => ProtoChain::Ethereum,
            RegistryChain::Solana => ProtoChain::Solana,
            RegistryChain::Sui => ProtoChain::Sui,
            RegistryChain::Tron => ProtoChain::Tron,
            _ => ProtoChain::Custom,
        }
    }

    #[test]
    fn test_conversions() {
        // Test supported chains round-trip
        for (proto, registry) in [
            (ProtoChain::Bitcoin, RegistryChain::Bitcoin),
            (ProtoChain::Ethereum, RegistryChain::Ethereum),
            (ProtoChain::Solana, RegistryChain::Solana),
            (ProtoChain::Sui, RegistryChain::Sui),
            (ProtoChain::Tron, RegistryChain::Tron),
        ] {
            assert_eq!(proto_to_registry(proto, None), Ok(registry.clone()));
            assert_eq!(registry_to_proto(&registry), proto);
        }

        assert_eq!(
            proto_to_registry(ProtoChain::Unspecified, None),
            Ok(RegistryChain::Unspecified),
        );

        // A registry chain with no proto slot (e.g. Aptos) round-trips to Custom.
        assert_eq!(registry_to_proto(&RegistryChain::Aptos), ProtoChain::Custom);

        // Custom chains: the name carries which chain, not the enum discriminant.
        assert_eq!(
            proto_to_registry(ProtoChain::Custom, Some("near")),
            Ok(RegistryChain::Custom("near".to_string())),
        );
        assert!(proto_to_registry(ProtoChain::Custom, None).is_err());
        assert!(proto_to_registry(ProtoChain::Custom, Some("")).is_err());
    }
}
