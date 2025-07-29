use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::JsonAbi;

use alloy_primitives::{Bytes, FixedBytes};
use visualsign::{
    SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    vsptrait::VisualSignError,
};

/// Parses a JSON ABI and a raw transaction, decodes the transaction input using the ABI,
/// and returns a SignablePayload with decoded function and arguments.
///
/// # Arguments
/// * `abi_json` - The contract ABI in JSON format (string)
/// * `tx` - The typed transaction to decode
/// * `options` - VisualSignOptions for customizing the payload
pub fn parse_json_abi_input(
    input: Bytes,
    abi_json: &str,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    // Parse the ABI
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

    // Decode the arguments
    let decoded = function
        .abi_decode_input(&input[4..])
        .map_err(|e| VisualSignError::DecodeError(format!("Failed to decode input: {e}")))?;

    // Prepare fields for the payload
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
        let value_str = format_abi_value(&value);
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: value_str.clone(),
                label: param.name.clone(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: value_str },
        });
    }

    Ok(fields)
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
        DynSolValue::Array(arr) => {
            let vals: Vec<String> = arr.iter().map(format_abi_value).collect();
            format!("[{}]", vals.join(", "))
        }
        DynSolValue::Tuple(tup) => {
            let vals: Vec<String> = tup.iter().map(format_abi_value).collect();
            format!("({})", vals.join(", "))
        }
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
