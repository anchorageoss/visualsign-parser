use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Bytes, FixedBytes};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldTextV2, vsptrait::VisualSignError,
};

pub fn parse_json_abi_input(
    input: Bytes,
    abi_json: &str,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    let abi: JsonAbi = serde_json::from_str(abi_json)
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid ABI JSON: {e}")))?;

    if input.is_empty() {
        return Err(VisualSignError::DecodeError(
            "Transaction has no input data".to_string(),
        ));
    }

    let function = abi
        .function_by_selector(FixedBytes::<4>::from_slice(&input[..4]))
        .ok_or_else(|| {
            VisualSignError::DecodeError("Function selector not found in ABI".to_string())
        })?;

    let decoded = function
        .abi_decode_input(&input[4..])
        .map_err(|e| VisualSignError::DecodeError(format!("Failed to decode input: {e}")))?;

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: function.name.clone(),
            label: "Function".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: function.name.clone(),
        },
    }];
    for (param, value) in function.inputs.iter().zip(decoded) {
        fields.push(dynsol_to_signable_field(&param.name, &value));
    }
    Ok(fields)
}

fn dynsol_to_signable_field(label: &str, value: &DynSolValue) -> SignablePayloadField {
    match value {
        DynSolValue::Array(arr) => {
            let annotated_fields: Vec<AnnotatedPayloadField> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| AnnotatedPayloadField {
                    signable_payload_field: dynsol_to_signable_field(&format!("Element {i}"), v),
                    static_annotation: None,
                    dynamic_annotation: None,
                })
                .collect();

            SignablePayloadField::ListLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "{label}: [{}]",
                        arr.iter()
                            .map(format_abi_value)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    label: label.to_string(),
                },
                list_layout: SignablePayloadFieldListLayout {
                    fields: annotated_fields,
                },
            }
        }
        DynSolValue::Tuple(arr) => {
            let annotated_fields: Vec<AnnotatedPayloadField> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| AnnotatedPayloadField {
                    signable_payload_field: dynsol_to_signable_field(&format!("Field {i}"), v),
                    static_annotation: None,
                    dynamic_annotation: None,
                })
                .collect();

            SignablePayloadField::ListLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!(
                        "{label}: ({})",
                        arr.iter()
                            .map(format_abi_value)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    label: label.to_string(),
                },
                list_layout: SignablePayloadFieldListLayout {
                    fields: annotated_fields,
                },
            }
        }
        _ => SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format_abi_value(value),
                label: label.to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format_abi_value(value),
            },
        },
    }
}

fn format_abi_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Address(addr) => format!("{addr:?}"),
        DynSolValue::Uint(u, _) => u.to_string(),
        DynSolValue::Int(i, _) => i.to_string(),
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => "<unsupported>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, U256};

    use super::*;

    // Minimal ERC20 ABI for transfer(address,uint256)
    const ERC20_TRANSFER_ABI: &str = r#"
    [
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}]
        }
    ]
    "#;

    #[test]
    fn parses_valid_erc20_transfer_input() {
        let abi: JsonAbi = serde_json::from_str(ERC20_TRANSFER_ABI).unwrap();
        let function = abi.function("transfer").unwrap().first().unwrap();
        let amount = DynSolValue::Uint(U256::from(100u64), 256);
        let to = DynSolValue::Address(
            "0x000000000000000000000000000000000000dead"
                .parse()
                .unwrap(),
        );
        let input = function
            .abi_encode_input(&[to.clone(), amount.clone()])
            .unwrap();

        assert_eq!(
            parse_json_abi_input(Bytes::from(input.to_vec()), ERC20_TRANSFER_ABI).unwrap(),
            vec![
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "transfer".to_string(),
                        label: "Function".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "transfer".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format_abi_value(&to),
                        label: "to".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format_abi_value(&to),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format_abi_value(&amount),
                        label: "amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format_abi_value(&amount),
                    },
                },
            ]
        );
    }

    #[test]
    fn returns_error_on_invalid_abi_json() {
        let err = parse_json_abi_input(Bytes::from([0u8; 4].to_vec()), "not json").unwrap_err();
        assert!(format!("{err}").contains("Invalid ABI JSON"));
    }

    #[test]
    fn returns_error_on_empty_input() {
        let err = parse_json_abi_input(Bytes::from([].to_vec()), ERC20_TRANSFER_ABI).unwrap_err();
        assert!(format!("{err}").contains("Transaction has no input data"));
    }

    #[test]
    fn returns_error_on_unknown_selector() {
        // Use a selector not in the ABI
        let input = [0xde, 0xad, 0xbe, 0xef, 1, 2, 3, 4];
        let err =
            parse_json_abi_input(Bytes::from(input.to_vec()), ERC20_TRANSFER_ABI).unwrap_err();
        assert!(format!("{err}").contains("Function selector not found in ABI"));
    }

    #[test]
    fn returns_error_on_decode_failure() {
        // Use correct selector but not enough data for arguments
        let input = hex::decode("a9059cbb").unwrap();
        let err =
            parse_json_abi_input(Bytes::from(input.to_vec()), ERC20_TRANSFER_ABI).unwrap_err();
        assert!(format!("{err}").contains("Failed to decode input"));
    }
}
