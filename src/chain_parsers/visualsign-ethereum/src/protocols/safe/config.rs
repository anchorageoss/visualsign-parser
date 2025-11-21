use crate::registry::ContractType;
use alloy_primitives::Address;

/// Type marker for Safe wallet contracts
/// All Safe wallets (regardless of version or deployment address) share this type
pub struct SafeWallet;

impl ContractType for SafeWallet {
    fn short_type_id() -> &'static str {
        "SafeWallet"
    }
}

pub struct SafeConfig;

impl SafeConfig {
    /// Safe v1.3.0 singleton address (used on most chains)
    pub fn safe_v1_3_0_address() -> Address {
        "0xd9Db270c1B5E3Bd161E8c8503c55cEABeE709552"
            .parse()
            .expect("Valid address")
    }

    /// Safe v1.4.1 singleton address
    pub fn safe_v1_4_1_address() -> Address {
        "0x41675C099F32341bf84BFc5382aF534df5C7461a"
            .parse()
            .expect("Valid address")
    }

    /// Chains where Safe is deployed
    pub fn safe_chains() -> &'static [u64] {
        &[
            1,     // Ethereum
            10,    // Optimism
            137,   // Polygon
            8453,  // Base
            42161, // Arbitrum
        ]
    }

    /// Register Safe singleton contracts
    pub fn register_contracts(registry: &mut crate::registry::ContractRegistry) {
        let v1_3_0 = Self::safe_v1_3_0_address();
        let v1_4_1 = Self::safe_v1_4_1_address();

        for &chain_id in Self::safe_chains() {
            // Register both versions as SafeWallet type
            registry.register_contract_typed::<SafeWallet>(chain_id, vec![v1_3_0]);
            registry.register_contract_typed::<SafeWallet>(chain_id, vec![v1_4_1]);
        }
    }

    /// Register a specific Safe instance (for dynamically deployed wallets)
    pub fn register_safe_instance(
        registry: &mut crate::registry::ContractRegistry,
        chain_id: u64,
        address: Address,
    ) {
        registry.register_contract_typed::<SafeWallet>(chain_id, vec![address]);
    }
}
