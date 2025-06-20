//! Conversion functions between the generated parser Chain enum and the visualsign registry Chain enum.
use visualsign::registry::Chain as RegistryChain;

use generated::parser::Chain as ProtoChain;

pub(crate) fn proto_to_registry(proto: ProtoChain) -> RegistryChain {
    match proto {
        ProtoChain::Solana => RegistryChain::Solana,
        ProtoChain::Ethereum => RegistryChain::Ethereum,
        ProtoChain::Sui => RegistryChain::Sui,
        ProtoChain::Unspecified => RegistryChain::Unspecified,
        _ => RegistryChain::Custom("custom_unknown".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn registry_to_proto(registry: &RegistryChain) -> ProtoChain {
        match registry {
            RegistryChain::Unspecified => ProtoChain::Unspecified,
            RegistryChain::Solana => ProtoChain::Solana,
            RegistryChain::Ethereum => ProtoChain::Ethereum,
            RegistryChain::Sui => ProtoChain::Sui,
            _ => ProtoChain::Custom,
        }
    }

    #[test]
    fn test_conversions() {
        // Test supported chains round-trip
        for (proto, registry) in [
            (ProtoChain::Solana, RegistryChain::Solana),
            (ProtoChain::Ethereum, RegistryChain::Ethereum),
            (ProtoChain::Sui, RegistryChain::Sui),
        ] {
            assert_eq!(proto_to_registry(proto), registry);
            assert_eq!(registry_to_proto(&registry), proto);
        }

        // Test unsupported map to unspecified
        assert_eq!(
            registry_to_proto(&RegistryChain::Bitcoin),
            ProtoChain::Custom
        );
        assert_eq!(
            proto_to_registry(ProtoChain::Unspecified),
            RegistryChain::Unspecified,
        );

        // Test custom chains
        assert_eq!(
            proto_to_registry(ProtoChain::Custom),
            RegistryChain::Custom("custom_unknown".into()),
        );
    }
}
