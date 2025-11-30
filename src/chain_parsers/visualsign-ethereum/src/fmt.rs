use alloy_primitives::{
    U256,
    utils::{ParseUnits, format_units},
};

fn trim_trailing_zeros(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

// Helper function to format wei to ether
pub fn format_ether<T: Into<ParseUnits> + ToString + Copy>(wei: T) -> String {
    trim_trailing_zeros(format_units(wei, "eth").unwrap_or_else(|_| wei.to_string()))
}

// Helper function to format wei to gwei
pub fn format_gwei<T: Into<ParseUnits> + ToString + Copy>(wei: T) -> String {
    trim_trailing_zeros(format_units(wei, "gwei").unwrap_or_else(|_| wei.to_string()))
}

/// Checks if a U256 value represents "unlimited" (max uint256)
/// Used for permit operations and approval amounts that support type(uint).max
pub fn is_unlimited_u256(value: U256) -> bool {
    value == U256::MAX
}

/// Checks if a u128 value represents "unlimited" (max uint128)
pub fn is_unlimited_u128(value: u128) -> bool {
    value == u128::MAX
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ether_basic() {
        // 1 ether = 1_000_000_000_000_000_000 wei
        let wei = 1_000_000_000_000_000_000u128;
        assert_eq!("1", format_ether(wei));
    }

    #[test]
    fn test_format_ether_fractional() {
        let wei = 1_500_000_000_000_000_000u128;
        assert_eq!("1.5", format_ether(wei));
    }

    #[test]
    fn test_format_ether_zero() {
        let wei = 0u128;
        assert_eq!("0", format_ether(wei));
    }

    #[test]
    fn test_format_ether_trailing_zeros() {
        let wei = 1_100_000_000_000_000_000u128;
        assert_eq!("1.1", format_ether(wei));
    }

    #[test]
    fn test_format_gwei_basic() {
        // 1 gwei = 1_000_000_000 wei
        let wei = 1_000_000_000u128;
        assert_eq!("1", format_gwei(wei));
    }

    #[test]
    fn test_format_gwei_fractional() {
        let wei = 1_500_000_000u128;
        assert_eq!("1.5", format_gwei(wei));
    }

    #[test]
    fn test_format_gwei_zero() {
        let wei = 0u128;
        assert_eq!("0", format_gwei(wei));
    }

    #[test]
    fn test_format_gwei_trailing_zeros() {
        let wei = 1_100_000_000u128;
        assert_eq!("1.1", format_gwei(wei));
    }

    #[test]
    fn test_format_ether_large_value() {
        let wei = 123_456_789_000_000_000_000u128;
        assert_eq!("123.456789", format_ether(wei));
    }

    #[test]
    fn test_format_gwei_large_value() {
        let wei = 123_456_789_000u128;
        assert_eq!("123.456789", format_gwei(wei));
    }

    #[test]
    fn test_is_unlimited_u256_max() {
        assert!(is_unlimited_u256(U256::MAX));
    }

    #[test]
    fn test_is_unlimited_u256_not_max() {
        assert!(!is_unlimited_u256(U256::from(1_000_000u128)));
        assert!(!is_unlimited_u256(U256::ZERO));
    }

    #[test]
    fn test_is_unlimited_u128_max() {
        assert!(is_unlimited_u128(u128::MAX));
    }

    #[test]
    fn test_is_unlimited_u128_not_max() {
        assert!(!is_unlimited_u128(1_000_000u128));
        assert!(!is_unlimited_u128(0u128));
    }
}
