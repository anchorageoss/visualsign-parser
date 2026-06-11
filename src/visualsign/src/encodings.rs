use std::fmt;

// Not every chain will support all the encodings, in which case they
// should return an error TransactionParseError::UnsupportedEncoding
// when the encoding is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedEncodings {
    Base64,
    Hex,
}

impl SupportedEncodings {
    /// Detect encoding format from string content. A leading `0x`/`0X` prefix is
    /// treated as hex (the `x` is not an ASCII hex digit, so it is stripped before
    /// the test).
    pub fn detect(data: &str) -> Self {
        if strip_hex_prefix(data)
            .chars()
            .all(|c| c.is_ascii_hexdigit())
        {
            Self::Hex
        } else {
            Self::Base64
        }
    }

    /// Convert encoding to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Base64 => "base64",
            Self::Hex => "hex",
        }
    }
}

impl fmt::Display for SupportedEncodings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SupportedEncodings {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "base64" => Ok(Self::Base64),
            "hex" => Ok(Self::Hex),
            _ => Err(format!(
                "Unsupported encoding format: {s}. Supported formats are: base64, hex"
            )),
        }
    }
}

/// Strip an optional `0x` / `0X` prefix from a hex string, returning the body.
/// Returns the input unchanged when it carries no prefix. The hex digits
/// themselves are not validated here.
///
/// This is the single definition of how the parser accepts a hex prefix; chain
/// crates use it (directly or via [`decode_hex`] / [`split_hex_prefix`]) rather
/// than hand-rolling prefix stripping, so prefix acceptance stays uniform across
/// chains and address/value/signature inputs.
#[must_use]
pub fn strip_hex_prefix(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}

/// Return the hex body when `value` carries a `0x`/`0X` prefix, or `None` when it
/// does not. Use this where the prefix is mandatory (e.g. JSON-RPC quantities and
/// data); the caller turns `None` into its own error.
#[must_use]
pub fn split_hex_prefix(value: &str) -> Option<&str> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
}

/// Decode a hex string into bytes, tolerating an optional `0x`/`0X` prefix. Hex
/// digit case is accepted in either form (the `hex` crate is case-insensitive on
/// digits). Callers map [`hex::FromHexError`] into their own chain error type.
///
/// # Errors
/// Returns [`hex::FromHexError`] when the body (after any prefix) is not valid hex.
pub fn decode_hex(value: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(strip_hex_prefix(value))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn strip_hex_prefix_handles_presence_absence_and_case() {
        assert_eq!(strip_hex_prefix("0xabcd"), "abcd");
        assert_eq!(strip_hex_prefix("0Xabcd"), "abcd");
        assert_eq!(strip_hex_prefix("abcd"), "abcd");
        assert_eq!(strip_hex_prefix(""), "");
        // Only the first prefix is stripped; a residual prefix is left intact.
        assert_eq!(strip_hex_prefix("0x0Xab"), "0Xab");
    }

    #[test]
    fn split_hex_prefix_requires_a_prefix() {
        assert_eq!(split_hex_prefix("0xabcd"), Some("abcd"));
        assert_eq!(split_hex_prefix("0Xabcd"), Some("abcd"));
        assert_eq!(split_hex_prefix("abcd"), None);
    }

    #[test]
    fn decode_hex_tolerates_optional_prefix_and_digit_case() {
        let expected = vec![0xab, 0xcd];
        assert_eq!(decode_hex("0xabcd").unwrap(), expected);
        assert_eq!(decode_hex("0XABCD").unwrap(), expected);
        assert_eq!(decode_hex("abCD").unwrap(), expected);
        assert!(decode_hex("0xzz").is_err());
        assert!(decode_hex("abc").is_err()); // odd length
    }

    #[test]
    fn detect_treats_prefixed_hex_as_hex() {
        assert_eq!(SupportedEncodings::detect("0xab"), SupportedEncodings::Hex);
        assert_eq!(SupportedEncodings::detect("0Xab"), SupportedEncodings::Hex);
        assert_eq!(SupportedEncodings::detect("abcd"), SupportedEncodings::Hex);
        // Non-hex content is still detected as base64.
        assert_eq!(
            SupportedEncodings::detect("not-hex+/="),
            SupportedEncodings::Base64
        );
    }
}
