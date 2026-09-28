/// Truncate address to show first 6 and last 4 characters
pub fn truncate_address(address: &str) -> String {
    if address.len() <= 10 {
        return address.to_string();
    }

    // Snap to char boundaries so non-ASCII input can't panic; head and tail
    // never exceed 6 and 4 bytes respectively.
    let head = &address[..address.floor_char_boundary(6)];
    let tail = &address[address.ceil_char_boundary(address.len() - 4)..];
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_address() {
        let address = "0x1234567890abcdef1234567890abcdef12345678";
        let truncated = truncate_address(address);
        assert_eq!(truncated, "0x1234...5678");

        let short_address = "0x12345";
        let truncated_short = truncate_address(short_address);
        assert_eq!(truncated_short, "0x12345");
    }

    #[test]
    fn test_truncate_address_multibyte_does_not_panic() {
        // Byte offsets 6 and len-4 both fall inside a 2-byte 'é'.
        let address = "0x123éabcdefé123";
        assert_eq!(truncate_address(address), "0x123...123");
    }
}
