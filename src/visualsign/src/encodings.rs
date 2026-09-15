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
        let detected = if strip_hex_prefix(data)
            .chars()
            .all(|c| c.is_ascii_hexdigit())
        {
            Self::Hex
        } else {
            Self::Base64
        };
        if quint_oracle::enabled() {
            use quint_oracle::ToLogged;
            quint_oracle::Event::builder(quint_oracle::current_test(), "detect_encoding")
                .argument(
                    "data",
                    data.chars().map(String::from).collect::<Vec<String>>(),
                    Some("HEX_INPUTS"),
                )
                .assert(
                    vec![
                        quint_oracle::PathSeg::ident("facts"),
                        quint_oracle::PathSeg::ident("detect"),
                    ],
                    quint_oracle::record([("isHex", (detected == Self::Hex).to_logged())]),
                )
                .scope("shared-encoding-and-time-primitives")
                .send();
        }
        detected
    }

    /// Convert encoding to string representation
    pub fn as_str(&self) -> &'static str {
        let name = match self {
            Self::Base64 => "base64",
            Self::Hex => "hex",
        };
        if quint_oracle::enabled() {
            use quint_oracle::ToLogged;
            quint_oracle::Event::builder(quint_oracle::current_test(), "encoding_as_str")
                .argument(
                    // A payload-free Quint variant: `{ tag, value: <empty tuple> }`.
                    "encoding",
                    quint_oracle::record([
                        (
                            "tag",
                            match self {
                                Self::Base64 => "Base64",
                                Self::Hex => "Hex",
                            }
                            .to_logged(),
                        ),
                        ("value", quint_oracle::tuple(std::iter::empty())),
                    ]),
                    Some("ENCODINGS"),
                )
                .assert(
                    vec![
                        quint_oracle::PathSeg::ident("facts"),
                        quint_oracle::PathSeg::ident("nameRender"),
                    ],
                    quint_oracle::record([("name", name.to_logged())]),
                )
                .scope("shared-encoding-and-time-primitives")
                .send();
        }
        name
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
        let parsed = match s.to_lowercase().as_str() {
            "base64" => Ok(Self::Base64),
            "hex" => Ok(Self::Hex),
            _ => Err(format!(
                "Unsupported encoding format: {s}. Supported formats are: base64, hex"
            )),
        };
        if quint_oracle::enabled() {
            use quint_oracle::ToLogged;
            quint_oracle::Event::builder(quint_oracle::current_test(), "encoding_from_str")
                .argument("s", s, Some("ENCODING_NAMES"))
                .assert(
                    vec![
                        quint_oracle::PathSeg::ident("facts"),
                        quint_oracle::PathSeg::ident("nameParse"),
                    ],
                    quint_oracle::record([
                        ("ok", parsed.is_ok().to_logged()),
                        (
                            "name",
                            match &parsed {
                                Ok(Self::Base64) => "base64",
                                Ok(Self::Hex) => "hex",
                                Err(_) => "",
                            }
                            .to_logged(),
                        ),
                    ]),
                )
                .scope("shared-encoding-and-time-primitives")
                .send();
        }
        parsed
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
    let body = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if quint_oracle::enabled() {
        use quint_oracle::ToLogged;
        quint_oracle::Event::builder(quint_oracle::current_test(), "strip_hex_prefix")
            .argument(
                "value",
                value.chars().map(String::from).collect::<Vec<String>>(),
                Some("HEX_INPUTS"),
            )
            .assert(
                vec![
                    quint_oracle::PathSeg::ident("facts"),
                    quint_oracle::PathSeg::ident("strip"),
                ],
                quint_oracle::record([
                    (
                        "output",
                        body.chars()
                            .map(String::from)
                            .collect::<Vec<String>>()
                            .to_logged(),
                    ),
                    ("removedPrefix", (body.len() != value.len()).to_logged()),
                ]),
            )
            .scope("shared-encoding-and-time-primitives")
            .send();
    }
    body
}

/// Return the hex body when `value` carries a `0x`/`0X` prefix, or `None` when it
/// does not. Use this where the prefix is mandatory (e.g. JSON-RPC quantities and
/// data); the caller turns `None` into its own error.
#[must_use]
pub fn split_hex_prefix(value: &str) -> Option<&str> {
    let body = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    if quint_oracle::enabled() {
        use quint_oracle::ToLogged;
        quint_oracle::Event::builder(quint_oracle::current_test(), "split_hex_prefix")
            .argument(
                "value",
                value.chars().map(String::from).collect::<Vec<String>>(),
                Some("HEX_INPUTS"),
            )
            .assert(
                vec![
                    quint_oracle::PathSeg::ident("facts"),
                    quint_oracle::PathSeg::ident("split"),
                ],
                quint_oracle::record([
                    ("found", body.is_some().to_logged()),
                    (
                        "body",
                        body.unwrap_or("")
                            .chars()
                            .map(String::from)
                            .collect::<Vec<String>>()
                            .to_logged(),
                    ),
                ]),
            )
            .scope("shared-encoding-and-time-primitives")
            .send();
    }
    body
}

/// Decode a hex string into bytes, tolerating an optional `0x`/`0X` prefix. Hex
/// digit case is accepted in either form (the `hex` crate is case-insensitive on
/// digits). Callers map [`hex::FromHexError`] into their own chain error type.
///
/// # Errors
/// Returns [`hex::FromHexError`] when the body (after any prefix) is not valid hex.
pub fn decode_hex(value: &str) -> Result<Vec<u8>, hex::FromHexError> {
    let decoded = hex::decode(strip_hex_prefix(value));
    if quint_oracle::enabled() {
        use quint_oracle::ToLogged;
        // `badIndex` is -1 when no single character was at fault; -2 flags a
        // `FromHexError` variant this component does not model, so it surfaces as
        // a conformance mismatch instead of being folded into a modelled arm.
        let (ok, bytes, odd_length, bad_index) = match &decoded {
            Ok(bytes) => (
                true,
                bytes.iter().map(|b| i64::from(*b)).collect::<Vec<i64>>(),
                false,
                -1_i64,
            ),
            Err(hex::FromHexError::OddLength) => (false, Vec::new(), true, -1_i64),
            Err(hex::FromHexError::InvalidHexCharacter { index, .. }) => {
                (false, Vec::new(), false, *index as i64)
            }
            Err(_) => (false, Vec::new(), false, -2_i64),
        };
        quint_oracle::Event::builder(quint_oracle::current_test(), "decode_hex")
            .argument(
                "value",
                value.chars().map(String::from).collect::<Vec<String>>(),
                Some("HEX_INPUTS"),
            )
            .assert(
                vec![
                    quint_oracle::PathSeg::ident("facts"),
                    quint_oracle::PathSeg::ident("decode"),
                ],
                quint_oracle::record([
                    ("ok", ok.to_logged()),
                    ("bytes", bytes.to_logged()),
                    ("oddLength", odd_length.to_logged()),
                    ("badIndex", bad_index.to_logged()),
                ]),
            )
            .scope("shared-encoding-and-time-primitives")
            .send();
    }
    decoded
}

/// Why a fixed-size hex decode failed. [`fmt::Display`] renders a fragment
/// meant to be appended to the name of the field being decoded, so a caller
/// wrapping it as `format!("Invalid {what} {e}")` reads as
/// `"Invalid public key hex: ..."` or
/// `"Invalid public key length: expected 32 bytes, got 31"`.
#[derive(Debug, PartialEq)]
pub enum DecodeHexArrayError {
    /// The body (after any prefix) is not valid hex.
    Hex(hex::FromHexError),
    /// The hex decoded cleanly but to the wrong number of bytes.
    Length { expected: usize, got: usize },
}

impl fmt::Display for DecodeHexArrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hex(e) => write!(f, "hex: {e}"),
            Self::Length { expected, got } => {
                write!(f, "length: expected {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for DecodeHexArrayError {}

/// Decode a hex string into a fixed-size byte array, tolerating an optional
/// `0x`/`0X` prefix. This is the single definition of "decode a hex string of
/// exactly N bytes" for the parser: every chain crate that reads a fixed-width
/// public key or signature out of untrusted metadata goes through it, so a fix
/// to how malformed input is handled lands once rather than per chain.
///
/// # Errors
/// Returns [`DecodeHexArrayError`] when the body is not valid hex, or decodes
/// to a length other than `N`.
pub fn decode_hex_array<const N: usize>(value: &str) -> Result<[u8; N], DecodeHexArrayError> {
    let bytes = decode_hex(value).map_err(DecodeHexArrayError::Hex)?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| DecodeHexArrayError::Length {
            expected: N,
            got: v.len(),
        })
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
    fn decode_hex_array_enforces_the_exact_length() {
        assert_eq!(decode_hex_array::<2>("0xabcd").unwrap(), [0xab, 0xcd]);
        assert_eq!(decode_hex_array::<2>("ABCD").unwrap(), [0xab, 0xcd]);
        assert_eq!(
            decode_hex_array::<2>("abcdef"),
            Err(DecodeHexArrayError::Length {
                expected: 2,
                got: 3
            })
        );
        assert_eq!(
            decode_hex_array::<2>("ab"),
            Err(DecodeHexArrayError::Length {
                expected: 2,
                got: 1
            })
        );
        assert!(matches!(
            decode_hex_array::<2>("0xzzzz"),
            Err(DecodeHexArrayError::Hex(_))
        ));
    }

    #[test]
    fn decode_hex_array_error_reads_as_a_field_suffix() {
        let err = decode_hex_array::<32>("ab").expect_err("wrong length");
        assert_eq!(
            format!("Invalid public key {err}"),
            "Invalid public key length: expected 32 bytes, got 1"
        );
        let err = decode_hex_array::<32>("zz").expect_err("bad hex");
        assert!(format!("Invalid public key {err}").starts_with("Invalid public key hex: "));
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
