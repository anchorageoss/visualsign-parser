//! Fallback visualizer for unknown/unhandled contract calls
//!
//! This visualizer acts as a catch-all for contract calls that don't have
//! specific visualizers. It displays the raw calldata as hex.

use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

/// Fallback visualizer that displays raw hex data for unknown contracts
pub struct FallbackVisualizer;

impl FallbackVisualizer {
    /// Creates a new fallback visualizer
    pub fn new() -> Self {
        Self
    }

    /// Visualizes unknown contract calldata as hex
    ///
    /// # Arguments
    /// * `input` - The raw calldata bytes
    ///
    /// # Returns
    /// A SignablePayloadField containing the hex-encoded calldata
    pub fn visualize_hex(&self, input: &[u8]) -> SignablePayloadField {
        let hex_data = if input.is_empty() {
            "0x".to_string()
        } else {
            format!("0x{}", hex::encode(input))
        };

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: hex_data.clone(),
                label: "Contract Call Data".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: hex_data },
        }
    }
}

impl Default for FallbackVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visualize_empty_input() {
        let visualizer = FallbackVisualizer::new();
        let field = visualizer.visualize_hex(&[]);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "0x");
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_visualize_hex_data() {
        let visualizer = FallbackVisualizer::new();
        let input = vec![0x12, 0x34, 0x56, 0x78, 0xab, 0xcd, 0xef];
        let field = visualizer.visualize_hex(&input);

        match field {
            SignablePayloadField::TextV2 { text_v2, common } => {
                assert_eq!(text_v2.text, "0x12345678abcdef");
                assert_eq!(common.label, "Contract Call Data");
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_visualize_function_selector() {
        let visualizer = FallbackVisualizer::new();
        // Simulate a function call with 4-byte selector
        let input = vec![0xa9, 0x05, 0x9c, 0xbb];
        let field = visualizer.visualize_hex(&input);

        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "0xa9059cbb");
            }
            _ => panic!("Expected TextV2 field"),
        }
    }
}
