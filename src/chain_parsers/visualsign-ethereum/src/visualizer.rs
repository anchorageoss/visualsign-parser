use crate::context::VisualizerContext;
use std::collections::HashMap;
use visualsign::AnnotatedPayloadField;
use visualsign::vsptrait::VisualSignError;

/// Trait for visualizing specific contract types
/// We're using Arc so that visualizers can be shared across threads 
/// (we don't have guarantee it's only going to be one thread in tokio)
pub trait ContractVisualizer: Send + Sync {
    /// Returns the contract type this visualizer handles
    fn contract_type(&self) -> &str;

    /// Visualizes a call to this contract type
    ///
    /// # Arguments
    /// * `context` - The visualizer context containing transaction information
    ///
    /// # Returns
    /// * `Ok(Some(fields))` - Successfully visualized into annotated fields
    /// * `Ok(None)` - This visualizer cannot handle this call
    /// * `Err(error)` - Error during visualization
    ///
    /// # TODO
    /// Return hashed data of chain metadata as part of the response
    fn visualize(
        &self,
        context: &VisualizerContext,
    ) -> Result<Option<Vec<AnnotatedPayloadField>>, VisualSignError>;
}

/// Registry for managing Ethereum contract visualizers (Immutable)
///
/// This registry is designed to be built once and shared immutably (e.g., in an Arc).
/// Use `EthereumVisualizerRegistryBuilder` to construct a registry.
pub struct EthereumVisualizerRegistry {
    visualizers: HashMap<String, Box<dyn ContractVisualizer>>,
}

impl EthereumVisualizerRegistry {
    /// Retrieves a visualizer by contract type
    ///
    /// # Arguments
    /// * `contract_type` - The contract type to look up
    ///
    /// # Returns
    /// * `Some(&dyn ContractVisualizer)` - The visualizer if found
    /// * `None` - No visualizer registered for this type
    pub fn get(&self, contract_type: &str) -> Option<&dyn ContractVisualizer> {
        self.visualizers.get(contract_type).map(Box::as_ref)
    }
}

/// Builder for creating a new EthereumVisualizerRegistry (Mutable)
///
/// This builder is used during the setup phase to register visualizers.
/// Once all visualizers are registered, call `build()` to create an immutable registry.
#[derive(Default)]
pub struct EthereumVisualizerRegistryBuilder {
    visualizers: HashMap<String, Box<dyn ContractVisualizer>>,
}

impl EthereumVisualizerRegistryBuilder {
    /// Creates a new empty builder
    pub fn new() -> Self {
        Self {
            visualizers: HashMap::new(),
        }
    }

    /// Creates a new builder pre-populated with default protocols
    pub fn with_default_protocols() -> Self {
        let mut builder = Self::new();
        let mut contract_reg = crate::registry::ContractRegistry::new();
        crate::protocols::register_all(&mut contract_reg, &mut builder);
        builder
    }

    /// Registers a visualizer for a specific contract type
    ///
    /// # Arguments
    /// * `visualizer` - The visualizer to register
    ///
    /// # Returns
    /// * `None` - If this is a new registration
    /// * `Some(old_visualizer)` - If an existing visualizer was replaced
    pub fn register(
        &mut self,
        visualizer: Box<dyn ContractVisualizer>,
    ) -> Option<Box<dyn ContractVisualizer>> {
        let contract_type = visualizer.contract_type().to_string();
        self.visualizers.insert(contract_type, visualizer)
    }

    /// Consumes the builder and returns the immutable registry
    pub fn build(self) -> EthereumVisualizerRegistry {
        EthereumVisualizerRegistry {
            visualizers: self.visualizers,
        }
    }
}

impl Default for EthereumVisualizerRegistry {
    fn default() -> Self {
        EthereumVisualizerRegistryBuilder::default().build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock visualizer for testing
    struct MockVisualizer {
        contract_type: String,
    }

    impl ContractVisualizer for MockVisualizer {
        fn contract_type(&self) -> &str {
            &self.contract_type
        }

        fn visualize(
            &self,
            _context: &VisualizerContext,
        ) -> Result<Option<Vec<AnnotatedPayloadField>>, VisualSignError> {
            Ok(Some(vec![]))
        }
    }

    #[test]
    fn test_builder_new() {
        let builder = EthereumVisualizerRegistryBuilder::new();
        assert_eq!(builder.visualizers.len(), 0);
    }

    #[test]
    fn test_builder_register() {
        let mut builder = EthereumVisualizerRegistryBuilder::new();
        let visualizer = Box::new(MockVisualizer {
            contract_type: "TestToken".to_string(),
        });

        let old = builder.register(visualizer);
        assert!(old.is_none());
        assert_eq!(builder.visualizers.len(), 1);
    }

    #[test]
    fn test_builder_register_returns_old() {
        let mut builder = EthereumVisualizerRegistryBuilder::new();

        let visualizer1 = Box::new(MockVisualizer {
            contract_type: "Token".to_string(),
        });
        let old1 = builder.register(visualizer1);
        assert!(old1.is_none());

        let visualizer2 = Box::new(MockVisualizer {
            contract_type: "Token".to_string(),
        });
        let old2 = builder.register(visualizer2);
        assert!(old2.is_some());
        assert_eq!(old2.unwrap().contract_type(), "Token");
    }

    #[test]
    fn test_builder_build() {
        let mut builder = EthereumVisualizerRegistryBuilder::new();
        let visualizer = Box::new(MockVisualizer {
            contract_type: "ERC20".to_string(),
        });
        builder.register(visualizer);

        let registry = builder.build();
        assert!(registry.get("ERC20").is_some());
        assert_eq!(registry.get("ERC20").unwrap().contract_type(), "ERC20");
    }

    #[test]
    fn test_registry_get_not_found() {
        let registry = EthereumVisualizerRegistry::default();
        assert!(registry.get("NonExistent").is_none());
    }

    #[test]
    fn test_registry_multiple_visualizers() {
        let mut builder = EthereumVisualizerRegistryBuilder::new();

        let erc20 = Box::new(MockVisualizer {
            contract_type: "ERC20".to_string(),
        });
        let uniswap = Box::new(MockVisualizer {
            contract_type: "UniswapV3".to_string(),
        });
        let aave = Box::new(MockVisualizer {
            contract_type: "Aave".to_string(),
        });

        builder.register(erc20);
        builder.register(uniswap);
        builder.register(aave);

        let registry = builder.build();
        assert!(registry.get("ERC20").is_some());
        assert!(registry.get("UniswapV3").is_some());
        assert!(registry.get("Aave").is_some());
        assert!(registry.get("Unknown").is_none());
    }

    #[test]
    fn test_builder_default() {
        let builder = EthereumVisualizerRegistryBuilder::default();
        let registry = builder.build();
        // Default creates empty registry (no default protocols registered in tests)
        assert!(registry.get("ERC20").is_none());
    }

    #[test]
    fn test_registry_default() {
        let registry = EthereumVisualizerRegistry::default();
        // Default calls builder default and builds empty registry
        assert!(registry.get("ERC20").is_none());
    }

    #[test]
    fn test_builder_with_default_protocols() {
        let builder = EthereumVisualizerRegistryBuilder::with_default_protocols();
        let registry = builder.build();
        // Even though with_default_protocols is called, no protocols are registered
        // because crate::protocols::register_all is a placeholder
        assert!(registry.get("ERC20").is_none());
    }
}
