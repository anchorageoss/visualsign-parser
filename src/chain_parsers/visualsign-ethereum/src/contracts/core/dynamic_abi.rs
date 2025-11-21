//! Dynamic ABI visualizer
//!
//! Provides visualization for contract calls using compile-time embedded ABI JSON.
//! Falls back to dynamic decoding when built-in visualizers don't recognize the function.

use std::sync::Arc;

use alloy_json_abi::JsonAbi;

use visualsign::SignablePayloadField;

use crate::abi_decoder::AbiDecoder;
use crate::registry::ContractRegistry;
use crate::visualizer::CalldataVisualizer;

/// Visualizer for dynamically decoded ABI-based function calls
pub struct DynamicAbiVisualizer {
    decoder: AbiDecoder,
}

impl DynamicAbiVisualizer {
    /// Creates a new dynamic visualizer from an ABI
    pub fn new(abi: Arc<JsonAbi>) -> Self {
        Self {
            decoder: AbiDecoder::new(abi),
        }
    }
}

impl CalldataVisualizer for DynamicAbiVisualizer {
    fn visualize_calldata(
        &self,
        calldata: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        self.decoder
            .visualize(calldata, chain_id, registry)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        }
    ]"#;

    #[test]
    fn test_dynamic_visualizer_creation() {
        let abi: JsonAbi = serde_json::from_str(TEST_ABI).expect("Failed to parse ABI");
        let _visualizer = DynamicAbiVisualizer::new(Arc::new(abi));
    }

    #[test]
    fn test_calldata_visualizer_trait() {
        let abi: JsonAbi = serde_json::from_str(TEST_ABI).expect("Failed to parse ABI");
        let visualizer = DynamicAbiVisualizer::new(Arc::new(abi));

        // Test with empty calldata - should fail gracefully
        let result = visualizer.visualize_calldata(&[], 1, None);
        assert!(result.is_none());
    }
}
