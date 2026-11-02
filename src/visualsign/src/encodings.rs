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

/// Escape signed free-form text (a message, memo, token id) for display, so that
/// different texts never display the same.
///
/// Printable ASCII and space pass through. Every other character is replaced by
/// percent-encoding of its UTF-8 bytes (`%0A` for a line feed, `%C3%A9` for `é`),
/// and a literal `%` or `\` is encoded as well (`%25`, `%5C`). A space is encoded
/// as `%20` when it is first, last, or next to another space, so a renderer that
/// trims or collapses whitespace (HTML does both by default) still shows
/// `"pay alice "`, `"pay  alice"` and `"pay alice"` differently.
///
/// Dropping characters, or replacing them with one marker, lets two different
/// signed texts render identically: `a\nb`, `ab`, `a?b` and `aéb` would all be
/// indistinguishable to the signer, and a literal marker collides with anything
/// replaced by it. Escaping is reversible ([`unescape_display_text`] recovers the
/// text exactly), so the display determines the signed bytes. Escaping `%` stops a
/// text from forging the escape of a character it doesn't contain, and escaping
/// `\` keeps the output clear of every JSON escape that
/// [`SignablePayload::validate_charset`](crate::SignablePayload::validate_charset)
/// refuses.
///
/// Use this for text the signer approves. Structured values with their own
/// rendering (addresses, amounts, hex) don't need it. For text that arrives as
/// bytes, use [`escape_display_bytes`] on the bytes themselves: converting them
/// with `String::from_utf8_lossy` first maps every invalid sequence to U+FFFD,
/// so distinct inputs would display the same.
///
/// The output is plain ASCII. ASCII look-alikes (`l`, `I`, `1`; `O`, `0`) and
/// percent-encoded non-Latin text stay hard to tell apart or read; presenting
/// those is the wallet's concern, not this function's.
pub fn escape_display_text(text: &str) -> String {
    escape_display_bytes(text.as_bytes())
}

/// Escape signed bytes for display, whether or not they are UTF-8.
///
/// Same output as [`escape_display_text`] for UTF-8 input (its escape works
/// byte by byte), and every byte that isn't printable ASCII is percent-encoded
/// the same way, so `[0x61, 0xFF]` shows as `a%FF` and stays distinct from
/// `[0x61, 0xFE]` and from the UTF-8 encoding of `a\u{FFFD}`.
/// [`unescape_display_bytes`] inverts it.
pub fn escape_display_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        let passes = match b {
            b' ' => {
                let edge = i == 0 || i + 1 == bytes.len();
                let next_to_space =
                    (i > 0 && bytes[i - 1] == b' ') || bytes.get(i + 1).is_some_and(|&n| n == b' ');
                !edge && !next_to_space
            }
            b'%' | b'\\' => false,
            _ => b.is_ascii_graphic(),
        };
        if passes {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0x0F)]));
        }
    }
    out
}

/// The text [`escape_display_text`] was given, or `None` when `escaped` is not
/// something it produces: a malformed or lowercase escape, an escape of a
/// character that passes through, a raw character it would have escaped, or
/// bytes that aren't UTF-8.
pub fn unescape_display_text(escaped: &str) -> Option<String> {
    String::from_utf8(unescape_display_bytes(escaped)?).ok()
}

/// The bytes [`escape_display_bytes`] was given, or `None` when `escaped` is not
/// something it produces: a malformed or lowercase escape, an escape of a byte
/// that passes through, or a raw character it would have escaped.
pub fn unescape_display_bytes(escaped: &str) -> Option<Vec<u8>> {
    let upper_hex = |d: char| d.is_ascii_digit() || ('A'..='F').contains(&d);
    let mut bytes = Vec::with_capacity(escaped.len());
    let mut chars = escaped.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next().filter(|&d| upper_hex(d))?.to_digit(16)?;
            let lo = chars.next().filter(|&d| upper_hex(d))?.to_digit(16)?;
            bytes.push(u8::try_from(hi * 16 + lo).ok()?);
        } else if c == ' ' || (c.is_ascii_graphic() && c != '\\') {
            bytes.push(c as u8);
        } else {
            return None;
        }
    }
    // Only the canonical form decodes, so each display has exactly one source.
    (escape_display_bytes(&bytes) == escaped).then_some(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::field_builders::create_text_field;
    use crate::{SignablePayload, SignablePayloadField};
    use std::collections::BTreeSet;

    /// Texts that a lossy sanitizer has collapsed together, plus every escape
    /// boundary: controls, `%` and `\`, non-ASCII of each UTF-8 width, and
    /// bidi/zero-width characters.
    fn corpus() -> Vec<String> {
        let mut texts: Vec<String> = [
            "",
            "plain text",
            "a\nb",
            "ab",
            "a?b",
            "a\u{e9}b",
            "a%C3%A9b",
            "a%0Ab",
            "a\\nb",
            "a\\u0041b",
            "%",
            "%%",
            "%2",
            "\\",
            "\"quoted\"",
            "\t\r\x08\x0c\0",
            "\u{7f}",
            "caf\u{e9}",
            "\u{202e}dlrow",
            "zero\u{200b}width",
            "\u{1f600}",
            "\u{10ffff}",
            "  leading and trailing  ",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        texts.extend((0u32..0x800).filter_map(char::from_u32).map(String::from));
        texts
    }

    #[test]
    fn escape_round_trips() {
        for text in corpus() {
            let escaped = escape_display_text(&text);
            assert_eq!(
                unescape_display_text(&escaped).as_deref(),
                Some(text.as_str()),
                "{escaped}"
            );
        }
    }

    #[test]
    fn distinct_texts_display_distinctly() {
        let texts = corpus();
        let unique: BTreeSet<String> = texts.iter().cloned().collect();
        let displays: BTreeSet<String> = unique.iter().map(|t| escape_display_text(t)).collect();
        assert_eq!(displays.len(), unique.len());
    }

    #[test]
    fn collisions_a_marker_or_filter_would_have() {
        let shown = |t: &str| escape_display_text(t);
        assert_eq!(shown("a\nb"), "a%0Ab");
        assert_eq!(shown("a\u{e9}b"), "a%C3%A9b");
        assert_ne!(shown("a\nb"), shown("ab"));
        assert_ne!(shown("a\nb"), shown("a?b"));
        assert_ne!(shown("a\u{e9}b"), shown("a?b"));
        // A literal escape sequence can't pose as the character it encodes.
        assert_eq!(shown("a%0Ab"), "a%250Ab");
        assert_eq!(shown("a%C3%A9b"), "a%25C3%25A9b");
        assert_eq!(shown("a\\nb"), "a%5Cnb");
    }

    #[test]
    fn printable_ascii_and_quotes_pass_through() {
        let text = "Send 1.5 wNEAR to alice.near (id #7) {\"msg\":\"x\"} ~!@$^&*_+=|;:',.<>/?`";
        assert_eq!(escape_display_text(text), text);
    }

    #[test]
    fn unescape_accepts_only_canonical_output() {
        for bad in [
            "%", "%0", "%0a", "%GG", "%41", "a%20b", " ", "a ", " a", "a  b", "a\nb", "\u{e9}",
            "\\", "%FF", "%C3",
        ] {
            assert_eq!(unescape_display_text(bad), None, "{bad:?}");
        }
        // Valid escapes of bytes that aren't UTF-8 are bytes, not text.
        assert_eq!(unescape_display_bytes("%FF"), Some(vec![0xFF]));
        assert_eq!(unescape_display_bytes("%41"), None);
    }

    #[test]
    fn spaces_that_a_renderer_would_trim_or_collapse_are_escaped() {
        let shown = |t: &str| escape_display_text(t);
        assert_eq!(shown("pay alice"), "pay alice");
        assert_eq!(shown("pay alice "), "pay alice%20");
        assert_eq!(shown(" pay alice"), "%20pay alice");
        assert_eq!(shown("pay  alice"), "pay%20%20alice");
        assert_eq!(shown(" "), "%20");
        for text in [
            "pay alice",
            "pay alice ",
            " pay alice",
            "pay  alice",
            "pay   alice",
        ] {
            assert_eq!(unescape_display_text(&shown(text)).as_deref(), Some(text));
        }
    }

    #[test]
    fn bytes_escape_without_a_lossy_utf8_conversion() {
        let shown = [
            escape_display_bytes(&[0x61, 0xFF]),
            escape_display_bytes(&[0x61, 0xFE]),
            escape_display_bytes(&[0x61, 0xEF, 0xBF, 0xBD]),
        ];
        assert_eq!(shown, ["a%FF", "a%FE", "a%EF%BF%BD"]);
        for (bytes, display) in [
            (&[0x61u8, 0xFF][..], &shown[0]),
            (&[0x61, 0xFE], &shown[1]),
            (&[0x61, 0xEF, 0xBF, 0xBD], &shown[2]),
        ] {
            assert_eq!(unescape_display_bytes(display).as_deref(), Some(bytes));
        }
        // Not UTF-8, so not a text.
        assert_eq!(unescape_display_text("a%FF"), None);
        // UTF-8 bytes escape exactly as the text does.
        for text in corpus() {
            assert_eq!(
                escape_display_bytes(text.as_bytes()),
                escape_display_text(&text)
            );
        }
    }

    #[test]
    fn every_byte_round_trips() {
        for b in 0u8..=255 {
            for bytes in [vec![b], vec![b'x', b, b'y'], vec![b, b]] {
                let escaped = escape_display_bytes(&bytes);
                assert_eq!(unescape_display_bytes(&escaped), Some(bytes), "{escaped}");
            }
        }
    }

    #[test]
    fn every_unicode_scalar_round_trips() {
        let mut escaped = String::new();
        let mut text = String::new();
        for c in (0u32..=0x10FFFF).filter_map(char::from_u32) {
            text.clear();
            text.push('x');
            text.push(c);
            text.push('y');
            escaped.clear();
            escaped.push_str(&escape_display_text(&text));
            assert!(
                escaped.bytes().all(|b| b == b' ' || b.is_ascii_graphic()),
                "{escaped}"
            );
            assert_eq!(
                unescape_display_text(&escaped).as_deref(),
                Some(text.as_str())
            );
        }
    }

    #[test]
    fn escaped_text_passes_payload_charset_validation() {
        let fields: Vec<SignablePayloadField> = corpus()
            .iter()
            .map(|t| {
                create_text_field("Message", &escape_display_text(t))
                    .unwrap()
                    .signable_payload_field
            })
            .collect();
        let payload =
            SignablePayload::new(0, "Escape".to_string(), None, fields, "Test".to_string());
        payload.validate_charset().unwrap();
    }

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
