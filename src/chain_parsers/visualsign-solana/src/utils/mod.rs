use std::collections::HashMap;

use base64::{self, Engine};

// Constants
const ADDRESS_TRUNCATION_LENGTH: usize = 8;
/// Helper function to create a complete Solana transaction from a message with empty signatures
pub fn create_transaction_with_empty_signatures(message_base64: &str) -> String {
    // Decode the message
    let message_bytes = base64::engine::general_purpose::STANDARD
        .decode(message_base64)
        .unwrap();

    // Create a complete Solana transaction with empty signatures
    let mut transaction_bytes = Vec::new();

    // Add compact array length for signatures (0 signatures)
    transaction_bytes.push(0u8);

    // Add the message
    transaction_bytes.extend_from_slice(&message_bytes);

    // Encode the complete transaction back to base64
    base64::engine::general_purpose::STANDARD.encode(transaction_bytes)
}

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
}

/// Static lookup table for common Solana token addresses
pub fn get_token_lookup_table() -> HashMap<&'static str, TokenInfo> {
    let mut tokens = HashMap::new();

    // SOL (native)
    tokens.insert(
        "11111111111111111111111111111112",
        TokenInfo {
            symbol: "SOL",
            name: "Solana",
            decimals: 9,
        },
    );

    // USDC
    tokens.insert(
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        TokenInfo {
            symbol: "USDC",
            name: "USD Coin",
            decimals: 6,
        },
    );

    // USDT
    tokens.insert(
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        TokenInfo {
            symbol: "USDT",
            name: "Tether USD",
            decimals: 6,
        },
    );

    tokens
}

/// Helper function to format token amounts
pub fn format_token_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10_u64.pow(decimals as u32);
    let whole = amount / divisor;
    let fractional = amount % divisor;

    if fractional == 0 {
        format!("{}", whole)
    } else {
        let fractional_str = format!("{:0width$}", fractional, width = decimals as usize);
        let trimmed = fractional_str.trim_end_matches('0');
        if trimmed.is_empty() {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, trimmed)
        }
    }
}

/// Enhanced swap instruction with token information
#[derive(Debug, Clone)]
pub struct SwapTokenInfo {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub amount: u64,
    pub human_readable_amount: String,
}

/// Helper function to get token info from address
pub fn get_token_info(address: &str, amount: u64) -> SwapTokenInfo {
    let token_lookup = get_token_lookup_table();

    if let Some(token_info) = token_lookup.get(address) {
        SwapTokenInfo {
            address: address.to_string(),
            symbol: token_info.symbol.to_string(),
            name: token_info.name.to_string(),
            decimals: token_info.decimals,
            amount,
            human_readable_amount: format_token_amount(amount, token_info.decimals),
        }
    } else {
        // Unknown token - show truncated address
        let truncated = if address.len() > ADDRESS_TRUNCATION_LENGTH {
            format!("{}...{}", &address[0..4], &address[address.len() - 4..])
        } else {
            address.to_string()
        };

        SwapTokenInfo {
            address: address.to_string(),
            symbol: truncated.clone(),
            name: format!("Unknown Token ({})", truncated),
            decimals: 0,
            amount,
            human_readable_amount: amount.to_string(),
        }
    }
}

#[cfg(test)]
pub mod test_utils {
    use crate::transaction_string_to_visual_sign;
    use visualsign::SignablePayload;
    use visualsign::vsptrait::VisualSignOptions;

    pub fn payload_from_b64(data: &str) -> SignablePayload {
        transaction_string_to_visual_sign(
            data,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: None,
                partial_parsing: false,
            },
        )
        .expect("Failed to visualize tx commands")
    }

    pub fn assert_has_field(payload: &SignablePayload, label: &str) {
        payload
            .fields
            .iter()
            .find(|f| f.label() == label)
            .unwrap_or_else(|| panic!("Should have a {label} field"));
    }
}
