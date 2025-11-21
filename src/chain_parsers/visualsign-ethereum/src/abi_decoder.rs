//! ABI-based function call decoder
//!
//! Decodes function calls using compile-time embedded ABIs.
//! Converts function calldata into structured visualizations.

use std::sync::Arc;

use alloy_json_abi::{Function, JsonAbi};

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

/// Decodes function calls using a JSON ABI
pub struct AbiDecoder {
    abi: Arc<JsonAbi>,
}

impl AbiDecoder {
    /// Creates a new decoder for the given ABI
    pub fn new(abi: Arc<JsonAbi>) -> Self {
        Self { abi }
    }

    /// Finds a function by its 4-byte selector
    fn find_function_by_selector(&self, selector: &[u8; 4]) -> Option<&Function> {
        self.abi.functions().find(|f| f.selector() == *selector)
    }

    /// Decodes a function call from calldata
    ///
    /// # Arguments
    /// * `calldata` - Complete calldata including 4-byte function selector
    ///
    /// # Returns
    /// * `Ok((function_name, param_hex))` on success
    /// * `Err` if selector doesn't match any function
    pub fn decode_function(
        &self,
        calldata: &[u8],
    ) -> Result<(String, String), Box<dyn std::error::Error>> {
        if calldata.len() < 4 {
            return Err("Calldata too short for function selector".into());
        }

        let selector: [u8; 4] = calldata[0..4].try_into()?;
        let function = self
            .find_function_by_selector(&selector)
            .ok_or("Function selector not found in ABI")?;

        let input_data = &calldata[4..];
        let param_hex = hex::encode(input_data);

        Ok((function.name.clone(), param_hex))
    }

    /// Creates a PreviewLayout visualization for a function call
    pub fn visualize(
        &self,
        calldata: &[u8],
        _chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> Result<SignablePayloadField, Box<dyn std::error::Error>> {
        if calldata.len() < 4 {
            return Err("Calldata too short".into());
        }

        let selector: [u8; 4] = calldata[0..4].try_into()?;
        let function = self
            .find_function_by_selector(&selector)
            .ok_or("Function not found")?;

        let input_data = &calldata[4..];

        // Build field for each input parameter (showing parameter names and types for now)
        let mut expanded_fields = Vec::new();
        for (i, input) in function.inputs.iter().enumerate() {
            let param_name = if !input.name.is_empty() {
                input.name.clone()
            } else {
                format!("param{i}")
            };

            let formatted = format!(
                "{} ({})",
                input.ty,
                hex::encode(&input_data[..(8.min(input_data.len()))])
            );

            let field = AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: formatted.clone(),
                        label: param_name,
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: formatted },
                },
                static_annotation: None,
                dynamic_annotation: None,
            };
            expanded_fields.push(field);
        }

        // Build function signature
        let param_types: Vec<&str> = function.inputs.iter().map(|i| i.ty.as_str()).collect();
        let signature = format!("{}({})", function.name, param_types.join(","));

        let title = SignablePayloadFieldTextV2 {
            text: function.name.clone(),
        };

        let subtitle = SignablePayloadFieldTextV2 {
            text: signature.clone(),
        };

        Ok(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: signature,
                label: function.name.clone(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(title),
                subtitle: Some(subtitle),
                condensed: None,
                expanded: if expanded_fields.is_empty() {
                    None
                } else {
                    Some(SignablePayloadFieldListLayout {
                        fields: expanded_fields,
                    })
                },
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        },
        {
            "type": "function",
            "name": "approve",
            "inputs": [
                {"name": "spender", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}],
            "stateMutability": "nonpayable"
        }
    ]"#;

    #[test]
    fn test_decoder_creation() {
        let abi: JsonAbi = serde_json::from_str(SIMPLE_ABI).expect("Failed to parse ABI");
        let decoder = AbiDecoder::new(Arc::new(abi));

        // Should be able to look up functions
        let selector = [0xa9, 0x05, 0x9c, 0xbb]; // transfer selector
        assert!(decoder.find_function_by_selector(&selector).is_some());
    }

    #[test]
    fn test_visualize_error_on_empty_calldata() {
        let abi: JsonAbi = serde_json::from_str(SIMPLE_ABI).expect("Failed to parse ABI");
        let decoder = AbiDecoder::new(Arc::new(abi));

        let result = decoder.visualize(&[], 1, None);
        assert!(result.is_err());
    }
}
