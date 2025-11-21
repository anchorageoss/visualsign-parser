//! ABI-based function call decoder
//!
//! Decodes function calls using compile-time embedded ABIs.
//! Converts function calldata into structured visualizations.

use std::sync::Arc;

use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::U256;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

/// Decodes a single Solidity value from calldata
/// Simple implementation that handles common types
/// TODO: make this more comprehensive, use proper alloy library for this
fn decode_solidity_value(ty: &str, data: &[u8], offset: &mut usize) -> String {
    if ty == "address" {
        // Addresses are 32 bytes (20 bytes address padded to 32)
        if *offset + 32 <= data.len() {
            let bytes = &data[*offset..*offset + 32];
            let addr_bytes = &bytes[12..32]; // Take last 20 bytes
            *offset += 32;
            return format!("0x{}", hex::encode(addr_bytes));
        }
    } else if ty == "uint256" || ty == "uint" {
        // uint256 is 32 bytes
        if *offset + 32 <= data.len() {
            let bytes = &data[*offset..*offset + 32];
            let val = U256::from_be_bytes(bytes.try_into().unwrap_or([0; 32]));
            *offset += 32;
            return val.to_string();
        }
    } else if ty.starts_with("uint") {
        // Other uint types - still 32 bytes in encoding
        if *offset + 32 <= data.len() {
            let bytes = &data[*offset..*offset + 32];
            let val = U256::from_be_bytes(bytes.try_into().unwrap_or([0; 32]));
            *offset += 32;
            return val.to_string();
        }
    } else if ty == "address[]" {
        // Dynamic address arrays - offset points to location of array
        if *offset + 32 <= data.len() {
            let array_offset =
                U256::from_be_bytes(data[*offset..*offset + 32].try_into().unwrap_or([0; 32]));
            *offset += 32;

            // Read array length at the offset
            let array_offset_usize = array_offset.try_into().unwrap_or(0usize);
            if array_offset_usize + 32 <= data.len() {
                let array_len_val = U256::from_be_bytes(
                    data[array_offset_usize..array_offset_usize + 32]
                        .try_into()
                        .unwrap_or([0; 32]),
                );
                let array_len: usize = array_len_val.try_into().unwrap_or(0);
                let mut addresses = Vec::new();

                for i in 0..array_len {
                    let addr_offset_val: usize =
                        (U256::from(array_offset_usize) + U256::from(32) + U256::from(i * 32))
                            .try_into()
                            .unwrap_or(0);
                    if addr_offset_val + 32 <= data.len() {
                        let addr_bytes = &data[addr_offset_val + 12..addr_offset_val + 32]; // Take last 20 bytes
                        addresses.push(format!("0x{}", hex::encode(addr_bytes)));
                    }
                }

                if addresses.is_empty() {
                    return "[]".to_string();
                } else {
                    return format!("[{}]", addresses.join(", "));
                }
            }
        }
    } else if ty.ends_with("[]") {
        // Other dynamic arrays - just show offset for now
        if *offset + 32 <= data.len() {
            let array_offset_val =
                U256::from_be_bytes(data[*offset..*offset + 32].try_into().unwrap_or([0; 32]));
            *offset += 32;
            return format!("(dynamic array at offset {})", array_offset_val);
        }
    }

    // Fallback for unknown types
    if *offset + 32 <= data.len() {
        let hex_val = hex::encode(&data[*offset..(*offset + 32).min(data.len())]);
        *offset = (*offset + 32).min(data.len());
        format!("{}: 0x{}", ty, hex_val)
    } else {
        format!("{}: (insufficient data)", ty)
    }
}

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

        let mut expanded_fields = Vec::new();
        let mut offset = 0;

        // Build field for each input parameter
        for (i, input) in function.inputs.iter().enumerate() {
            let param_name = if !input.name.is_empty() {
                input.name.clone()
            } else {
                format!("param{i}")
            };

            // Simple decoding based on type
            let formatted = decode_solidity_value(&input.ty, input_data, &mut offset);

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
