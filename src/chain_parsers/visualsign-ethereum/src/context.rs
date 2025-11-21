use alloy_primitives::Address;
use std::sync::Arc;

/// Registry for managing contract ABIs and metadata
pub trait ContractRegistry: Send + Sync {
    /// Format a token amount using the registry's token information
    fn format_token_amount(&self, amount: u128, decimals: u8) -> String;
}

/// Registry for managing contract visualizers
pub trait VisualizerRegistry: Send + Sync {
    /// Retrieves a visualizer by contract type
    ///
    /// # Arguments
    /// * `contract_type` - The contract type to look up
    ///
    /// # Returns
    /// * `Some(&dyn ContractVisualizer)` if found
    /// * `None` if not found
    fn get_visualizer(
        &self,
        contract_type: &str,
    ) -> Option<&dyn crate::visualizer::ContractVisualizer>;
}

/// Arguments for creating a new VisualizerContext
/// This is safer than making a new() with many arguments directly
/// which clippy doesn't like and is bug prone to missing fields or mixing them
pub struct VisualizerContextParams {
    pub chain_id: u64,
    pub sender: Address,
    pub current_contract: Address,
    pub calldata: Vec<u8>,
    pub registry: Arc<dyn ContractRegistry>,
    pub visualizers: Arc<dyn VisualizerRegistry>,
}

/// Context for visualizing Ethereum transactions and calls
#[derive(Clone)]
pub struct VisualizerContext {
    /// The blockchain chain ID (e.g., 1 for Ethereum mainnet)
    pub chain_id: u64,
    /// The sender of the transaction
    pub sender: Address,
    /// The current contract being visualized
    pub current_contract: Address,
    /// The depth of nested calls (0 for top-level)
    pub call_depth: usize,
    /// The raw calldata for the current call, shared via Arc
    pub calldata: Arc<[u8]>,
    /// Registry containing contract ABI and metadata
    pub registry: Arc<dyn ContractRegistry>,
    /// Registry containing contract visualizers
    pub visualizers: Arc<dyn VisualizerRegistry>,
}

impl VisualizerContext {
    /// Creates a new, top-level visualizer context
    pub fn new(params: VisualizerContextParams) -> Self {
        Self {
            chain_id: params.chain_id,
            sender: params.sender,
            current_contract: params.current_contract,
            call_depth: 0, // Set defaults inside the constructor
            calldata: Arc::from(params.calldata),
            registry: params.registry,
            visualizers: params.visualizers,
        }
    }

    /// Creates a child context for a nested call with incremented call_depth
    pub fn for_nested_call(
        &self,
        current_contract: Address,
        calldata: Vec<u8>, // Still takes a Vec, as it's new data
    ) -> Self {
        Self {
            chain_id: self.chain_id,
            sender: self.sender,
            current_contract,
            call_depth: self.call_depth + 1,
            calldata: Arc::from(calldata), // Convert to Arc
            registry: self.registry.clone(),
            visualizers: self.visualizers.clone(),
        }
    }

    /// Helper method to format token amounts using the registry
    pub fn format_token_amount(&self, amount: u128, decimals: u8) -> String {
        self.registry.format_token_amount(amount, decimals)
    }

    /// Retrieves a visualizer by contract type from the registry
    ///
    /// Enables visualizers to delegate to other visualizers during execution.
    ///
    /// # Arguments
    /// * `contract_type` - The contract type identifier
    ///
    /// # Returns
    /// * `Some(&dyn ContractVisualizer)` if found
    /// * `None` if not registered
    pub fn get_visualizer(
        &self,
        contract_type: &str,
    ) -> Option<&dyn crate::visualizer::ContractVisualizer> {
        self.visualizers.get_visualizer(contract_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock implementation of ContractRegistry for testing
    struct MockContractRegistry;

    impl ContractRegistry for MockContractRegistry {
        fn format_token_amount(&self, amount: u128, decimals: u8) -> String {
            // Use Alloy's format_units utility
            alloy_primitives::utils::format_units(amount, decimals)
                .unwrap_or_else(|_| amount.to_string())
        }
    }

    /// Mock implementation of VisualizerRegistry for testing
    struct MockVisualizerRegistry;

    impl VisualizerRegistry for MockVisualizerRegistry {
        fn get_visualizer(
            &self,
            _contract_type: &str,
        ) -> Option<&dyn crate::visualizer::ContractVisualizer> {
            None
        }
    }

    #[test]
    fn test_visualizer_context_creation() {
        let registry = Arc::new(MockContractRegistry);
        let visualizers = Arc::new(MockVisualizerRegistry);
        let sender = "0x1234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let contract = "0xabcdefabcdefabcdefabcdefabcdefabcdefabce"
            .parse()
            .unwrap();
        let calldata = vec![0x12, 0x34, 0x56, 0x78];

        let params = VisualizerContextParams {
            chain_id: 1,
            sender,
            current_contract: contract,
            calldata: calldata.clone(),
            registry: registry.clone(),
            visualizers: visualizers.clone(),
        };
        let context = VisualizerContext::new(params);

        assert_eq!(context.chain_id, 1);
        assert_eq!(context.call_depth, 0);
        assert_eq!(context.sender, sender);
        assert_eq!(context.current_contract, contract);
        assert_eq!(context.calldata.len(), 4);
        assert_eq!(context.calldata.as_ref(), calldata.as_slice());
    }

    #[test]
    fn test_visualizer_context_clone() {
        let registry = Arc::new(MockContractRegistry);
        let visualizers = Arc::new(MockVisualizerRegistry);
        let sender = "0x1234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let contract = "0xabcdefabcdefabcdefabcdefabcdefabcdefabce"
            .parse()
            .unwrap();
        let calldata = vec![0x12, 0x34, 0x56, 0x78];

        let params = VisualizerContextParams {
            chain_id: 1,
            sender,
            current_contract: contract,
            calldata: calldata.clone(),
            registry: registry.clone(),
            visualizers: visualizers.clone(),
        };
        let context = VisualizerContext::new(params);

        let cloned = context.clone();

        assert_eq!(cloned.chain_id, context.chain_id);
        assert_eq!(cloned.call_depth, context.call_depth);
        assert_eq!(cloned.sender, context.sender);
        assert_eq!(cloned.current_contract, context.current_contract);

        // Test that the Arcs point to the same data and the data is correct
        assert_eq!(cloned.calldata, context.calldata);
        assert_eq!(cloned.calldata.as_ref(), calldata.as_slice());
        // Test that cloning the Arc was cheap (pointer comparison)
        assert!(Arc::ptr_eq(&cloned.calldata, &context.calldata));
        assert!(Arc::ptr_eq(&cloned.registry, &context.registry));
    }

    #[test]
    fn test_for_nested_call() {
        let registry = Arc::new(MockContractRegistry);
        let visualizers = Arc::new(MockVisualizerRegistry);
        let sender = "0x1234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let contract1 = "0xabcdefabcdefabcdefabcdefabcdefabcdefabce"
            .parse()
            .unwrap();
        let contract2 = "0xfedcbafedcbafedcbafedcbafedcbafedcbafeda"
            .parse()
            .unwrap();
        let calldata1 = vec![0x12, 0x34, 0x56, 0x78];
        let calldata2 = vec![0xaa, 0xbb, 0xcc, 0xdd];
        let params = VisualizerContextParams {
            chain_id: 1,
            sender,
            current_contract: contract1,
            calldata: calldata1.clone(),
            registry: registry.clone(),
            visualizers: visualizers.clone(),
        };
        let context = VisualizerContext::new(params);

        let nested = context.for_nested_call(contract2, calldata2.clone());

        assert_eq!(nested.chain_id, context.chain_id);
        assert_eq!(nested.sender, context.sender);
        assert_eq!(nested.current_contract, contract2);
        assert_eq!(nested.call_depth, 1);
        assert_eq!(nested.calldata.as_ref(), calldata2.as_slice());
    }

    #[test]
    fn test_format_token_amount() {
        let registry = Arc::new(MockContractRegistry);
        let visualizers = Arc::new(MockVisualizerRegistry);

        let params = VisualizerContextParams {
            chain_id: 1,
            sender: Address::ZERO,
            current_contract: Address::ZERO,
            calldata: vec![],
            registry: registry.clone(),
            visualizers: visualizers.clone(),
        };
        let context = VisualizerContext::new(params);

        // Test with 18 decimals (like ETH/USDC)
        assert_eq!(
            context.format_token_amount(1000000000000000000, 18),
            "1.000000000000000000"
        );
        assert_eq!(
            context.format_token_amount(1500000000000000000, 18),
            "1.500000000000000000"
        );

        // Test with 6 decimals (like USDT)
        assert_eq!(context.format_token_amount(1000000, 6), "1.000000");
        assert_eq!(context.format_token_amount(1500000, 6), "1.500000");
    }

    #[test]
    fn test_nested_call_increments_depth() {
        let registry = Arc::new(MockContractRegistry);
        let visualizers = Arc::new(MockVisualizerRegistry);
        let contract1 = "0xabcdefabcdefabcdefabcdefabcdefabcdefabce"
            .parse()
            .unwrap();
        let contract2 = "0xfedcbafedcbafedcbafedcbafedcbafedcbafeda"
            .parse()
            .unwrap();
        let contract3 = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let params = VisualizerContextParams {
            chain_id: 1,
            sender: Address::ZERO,
            current_contract: contract1,
            calldata: vec![],
            registry: registry.clone(),
            visualizers: visualizers.clone(),
        };
        let context = VisualizerContext::new(params);

        let nested1 = context.for_nested_call(contract2, vec![]);
        assert_eq!(nested1.call_depth, 1);

        let nested2 = nested1.for_nested_call(contract3, vec![]);
        assert_eq!(nested2.call_depth, 2);
    }
}
