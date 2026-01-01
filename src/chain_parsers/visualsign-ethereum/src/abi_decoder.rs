//! ABI-based function call decoder
//!
//! Decodes function calls using compile-time embedded ABIs.
//! Converts function calldata into structured visualizations.
//!
//! Uses alloy-dyn-abi for runtime type parsing and decoding, supporting
//! all Solidity types including arrays, tuples, structs, and nested types.

use std::sync::Arc;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::{Function, JsonAbi};

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

/// Formats a DynSolValue into a human-readable string
fn format_dyn_sol_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Address(addr) => format!("{addr:?}"),
        DynSolValue::Uint(val, _bits) => val.to_string(),
        DynSolValue::Int(val, _bits) => val.to_string(),
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Bytes(bytes) => format!("0x{}", hex::encode(bytes)),
        DynSolValue::FixedBytes(bytes, _size) => format!("0x{}", hex::encode(bytes)),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
            if values.is_empty() {
                "[]".to_string()
            } else {
                let formatted: Vec<String> = values.iter().map(format_dyn_sol_value).collect();
                format!("[{}]", formatted.join(", "))
            }
        }
        DynSolValue::Tuple(values) => {
            let formatted: Vec<String> = values.iter().map(format_dyn_sol_value).collect();
            format!("({})", formatted.join(", "))
        }
        DynSolValue::Function(func) => format!("0x{}", hex::encode(func.0)),
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

        // Only decode if there are parameters
        if !function.inputs.is_empty() {
            // Parse all parameter types
            let param_types: Vec<DynSolType> = function
                .inputs
                .iter()
                .map(|input| {
                    DynSolType::parse(&input.ty)
                        .map_err(|e| format!("Failed to parse type '{}': {}", input.ty, e))
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Function parameters are ABI-encoded as a tuple
            let tuple_type = DynSolType::Tuple(param_types);

            // Decode the tuple
            let decoded = tuple_type.abi_decode(input_data).map_err(|e| {
                format!(
                    "Failed to decode parameters: {}. Data length: {}, Data: 0x{}",
                    e,
                    input_data.len(),
                    hex::encode(input_data)
                )
            })?;

            // Extract values from the tuple
            let values = match decoded {
                DynSolValue::Tuple(vals) => vals,
                _ => return Err("Expected tuple from decode".into()),
            };

            // Build fields from decoded values
            for (i, (input, value)) in function.inputs.iter().zip(values.iter()).enumerate() {
                let param_name = if !input.name.is_empty() {
                    input.name.clone()
                } else {
                    format!("param{i}")
                };

                let formatted = format_dyn_sol_value(value);

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
    use alloy_primitives::U256;

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

    #[test]
    fn test_decode_transfer_with_alloy() {
        use alloy_sol_types::{SolCall, sol};

        let abi: JsonAbi = serde_json::from_str(SIMPLE_ABI).unwrap();
        let decoder = AbiDecoder::new(Arc::new(abi));

        sol! {
            interface IERC20 {
                function transfer(address to, uint256 amount) external returns (bool);
            }
        }

        let to_addr: alloy_primitives::Address = "0x1234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let call = IERC20::transferCall {
            to: to_addr,
            amount: U256::from(1_000_000u128),
        };
        let calldata = IERC20::transferCall::abi_encode(&call);

        // Test decode_function
        let (func_name, _hex) = decoder.decode_function(&calldata).expect("Decode failed");
        assert_eq!(func_name, "transfer");

        // Test visualize
        let field = decoder
            .visualize(&calldata, 1, None)
            .expect("Visualize failed");

        // Should have PreviewLayout with expanded fields
        match field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                let expanded = preview_layout
                    .expanded
                    .expect("Should have expanded fields");
                assert_eq!(expanded.fields.len(), 2); // to + amount
            }
            _ => panic!("Expected PreviewLayout"),
        }
    }
}
