use alloy_sol_types::{SolCall, sol};
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

sol! {
    interface IERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);

        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);
    }
}

// Helper struct for ERC-20 transfers
#[derive(Debug)]
struct Erc20Transfer {
    recipient: String,
    amount: String,
}

fn decode_erc20_transfer(input: &[u8]) -> Option<Erc20Transfer> {
    if input.len() < 4 {
        return None;
    }

    // Try to decode as ERC-20 transfer (direct transfer)
    if let Ok(call) = IERC20::transferCall::abi_decode(input) {
        return Some(Erc20Transfer {
            recipient: format!("{:?}", call.to),
            amount: call.amount.to_string(),
        });
    }

    // Try to decode as ERC-20 transferFrom (delegated transfer)
    if let Ok(call) = IERC20::transferFromCall::abi_decode(input) {
        return Some(Erc20Transfer {
            recipient: format!("{:?}", call.to),
            amount: call.amount.to_string(),
            // Note: You might want to also capture the 'from' address for transferFrom
        });
    }

    None
}

pub fn parse_erc20_transfer(input: &[u8]) -> Vec<SignablePayloadField> {
    let mut fields = Vec::new();
    if let Some(decoded_transfer) = decode_erc20_transfer(input) {
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!(
                    "ERC-20 Transfer: {} to {}",
                    decoded_transfer.amount, decoded_transfer.recipient
                ),
                label: "Token Transfer".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!(
                    "Amount: {}\nRecipient: {}",
                    decoded_transfer.amount, decoded_transfer.recipient
                ),
            },
        });
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256};

    #[test]
    fn test_decode_erc20_transfer() {
        let call = IERC20::transferCall {
            amount: U256::from(1000),
            to: Address::from_slice(
                &hex::decode("1234567890123456789012345678901234567890").unwrap(),
            ),
        };
        let input_data = call.abi_encode();
        let result = decode_erc20_transfer(&input_data);
        assert!(result.is_some());
        let transfer = result.unwrap();
        assert_eq!(
            transfer.recipient,
            "0x1234567890123456789012345678901234567890"
        );
        assert_eq!(transfer.amount, "1000");

        // Invalid data (too short)
        let short_data = vec![0xa9, 0x05, 0x9c, 0xbb, 0x12];
        assert!(decode_erc20_transfer(&short_data).is_none());

        // Invalid function selector
        let invalid_selector = vec![0x00, 0x00, 0x00, 0x00];
        assert!(decode_erc20_transfer(&invalid_selector).is_none());
    }
}
