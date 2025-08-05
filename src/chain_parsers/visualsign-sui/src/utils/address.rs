/// Truncate address to show first 6 and last 4 characters
pub fn truncate_address(address: &str) -> String {
    if address.len() <= 10 {
        return address.to_string();
    }

    format!("{}...{}", &address[..6], &address[address.len() - 4..])
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
}
