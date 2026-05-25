use std::collections::BTreeMap;

// Constants
const ADDRESS_TRUNCATION_LENGTH: usize = 8;
/// Helper function to create a complete Solana transaction from a message with empty signatures.
/// Used by test code in this crate and by integration tests.
#[allow(clippy::unwrap_used)]
pub fn create_transaction_with_empty_signatures(message_base64: &str) -> String {
    use base64::Engine;
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
pub fn get_token_lookup_table() -> BTreeMap<&'static str, TokenInfo> {
    let mut tokens = BTreeMap::new();

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

/// Maximum supported token decimals. Beyond this we cannot compute `10^decimals`
/// in a `u64` divisor, so we fall back to the raw amount rather than panic.
/// `10^19` fits in `u64` (`u64::MAX` is ~1.84e19); `10^20` does not.
///
/// Test-only constant: the runtime fallback in `format_token_amount` uses
/// `checked_pow` directly, so production code does not need to reference this
/// bound. Kept here (rather than inlined in the test) so the documented limit
/// stays close to the function it constrains.
#[cfg(test)]
const MAX_SUPPORTED_DECIMALS: u8 = 19;

/// Helper function to format token amounts.
///
/// Defensive against attacker-controlled `decimals`: uses `checked_pow` so an
/// out-of-range value (>= 20) returns the raw amount rather than triggering a
/// divide-by-zero panic (10^64 wraps to 0 in `u64`, etc.). Callers that ingest
/// `decimals` from untrusted transaction bytes should additionally validate the
/// value up front and surface a parse error to the user.
pub fn format_token_amount(amount: u64, decimals: u8) -> String {
    let Some(divisor) = 10_u64.checked_pow(decimals as u32) else {
        // decimals is out of the representable range for u64; render as raw.
        return amount.to_string();
    };
    if divisor == 0 {
        // Belt and braces: should be unreachable given checked_pow above.
        return amount.to_string();
    }
    let whole = amount / divisor;
    let fractional = amount % divisor;

    if fractional == 0 {
        format!("{whole}")
    } else {
        let fractional_str = format!("{:0width$}", fractional, width = decimals as usize);
        let trimmed = fractional_str.trim_end_matches('0');
        if trimmed.is_empty() {
            format!("{whole}")
        } else {
            format!("{whole}.{trimmed}")
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
            name: format!("Unknown Token ({truncated})"),
            decimals: 0,
            amount,
            human_readable_amount: amount.to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_amount_typical_decimals() {
        // 9 decimals (SOL)
        assert_eq!(format_token_amount(1_000_000_000, 9), "1");
        assert_eq!(format_token_amount(1_500_000_000, 9), "1.5");
        // 6 decimals (USDC)
        assert_eq!(format_token_amount(1_000_000, 6), "1");
        assert_eq!(format_token_amount(1_234_567, 6), "1.234567");
        // 0 decimals
        assert_eq!(format_token_amount(42, 0), "42");
    }

    #[test]
    fn test_format_token_amount_zero_amount() {
        assert_eq!(format_token_amount(0, 9), "0");
        assert_eq!(format_token_amount(0, 0), "0");
        // Out-of-range decimals with zero amount should still return "0", not panic.
        assert_eq!(format_token_amount(0, 64), "0");
    }

    /// Regression for PRS-221: decimals >= 20 must not trigger a divide-by-zero
    /// panic. `10_u64.pow(20)` overflows in debug and wraps in release; for
    /// `decimals == 64` the wrapped value is exactly `0` because `10^64 mod
    /// 2^64 == 0`, which used to panic on division.
    #[test]
    fn test_format_token_amount_decimals_out_of_range_does_not_panic() {
        // Each call must return without panicking.
        for decimals in [20u8, 21, 38, 63, 64, 100, 200, u8::MAX] {
            let formatted = format_token_amount(12_345_678_u64, decimals);
            // Fallback path: render the raw amount.
            assert_eq!(
                formatted, "12345678",
                "decimals={decimals} should fall back to raw amount"
            );
        }
    }

    #[test]
    fn test_format_token_amount_max_supported_decimals() {
        // 10^19 fits in u64; this is the last decimals value where we still
        // compute a fractional representation.
        let amount = 1_u64;
        let formatted = format_token_amount(amount, MAX_SUPPORTED_DECIMALS);
        // 1 / 10^19 = 0.0000000000000000001
        assert!(
            formatted.starts_with("0.") && formatted.ends_with('1'),
            "expected leading zero fractional, got {formatted}"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod test_utils {
    use crate::transaction_string_to_visual_sign;
    use visualsign::SignablePayload;
    use visualsign::vsptrait::VisualSignOptions;

    pub fn payload_from_b64(data: &str) -> SignablePayload {
        transaction_string_to_visual_sign(
            data,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
                developer_config: None,
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
