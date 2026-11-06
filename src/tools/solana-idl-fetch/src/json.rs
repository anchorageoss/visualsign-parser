//! Minimal JSON representation with insertion-ordered objects, a strict
//! parser, and printers that match the Python script this crate replaces.
//!
//! serde_json's `Value` alphabetizes object keys (the workspace does not
//! enable its `preserve_order` feature), but the Python original prints
//! JSON in insertion order (`json.dumps` without `sort_keys`) and the exact
//! IDL bytes are what provenance records hash, so key order must be kept.
//! Enabling `preserve_order` here would also change serde_json's behaviour
//! for every crate whose features unify with this one in a workspace-wide
//! build, so this module keeps a small ordered JSON representation of its
//! own instead.
//!
//! [`pretty`] matches `json.dumps(indent=2, ensure_ascii=False)` byte for
//! byte for the shapes IDLs and provenance records contain (objects,
//! arrays, strings, integers, booleans, null). Floats are printed with
//! Rust's shortest round-trip form, which differs from Python's only for
//! values Python itself renders in exponent notation; IDLs and records
//! carry no floats. [`canonical`] matches `json.dumps(sort_keys=True)`
//! up to those same float edge cases and is only ever compared against
//! other `canonical` output.

use std::fmt::Write as _;

/// A JSON value whose objects keep their insertion order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Build an object from `(key, value)` pairs, in the given order.
    pub fn object(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// Build a string value.
    pub fn str(text: &str) -> Json {
        Json::Str(text.to_string())
    }

    /// Value stored under `key`, or `None` outside objects. The parser
    /// resolves duplicate keys the way CPython's `dict` does (last value
    /// at the first position), so an object has at most one entry per key.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Element of an array value, or `None` for anything else.
    pub fn at(&self, index: usize) -> Option<&Json> {
        match self {
            Json::Array(items) => items.get(index),
            _ => None,
        }
    }

    /// Elements of an array value, or `None` for anything else.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// String value, or `None` for anything else.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    /// Non-negative integer value, or `None` for anything else.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Int(value) if *value >= 0 => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// Is this the JSON `null` literal?
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

/// Parse strict JSON (RFC 8259, plus Python-style tolerance of duplicate
/// keys, resolved to the last value).
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(format!("unexpected trailing data at offset {}", parser.pos));
    }
    Ok(value)
}

/// Render with 2-space indentation, keys in insertion order, non-ASCII kept
/// verbatim: `json.dumps(value, indent=2, ensure_ascii=False)`.
pub fn pretty(json: &Json) -> String {
    let mut out = String::new();
    write_pretty(&mut out, json, 0);
    out
}

/// Render compactly with keys sorted at every level, for equality
/// comparison only: `json.dumps(value, sort_keys=True)`.
pub fn canonical(json: &Json) -> String {
    let mut out = String::new();
    write_canonical(&mut out, json);
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while matches!(
            self.bytes.get(self.pos),
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.bytes.get(self.pos) {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(byte) if *byte == b'-' || byte.is_ascii_digit() => self.parse_number(),
            Some(byte) => Err(format!("unexpected byte {byte} at offset {}", self.pos)),
            None => Err("unexpected end of JSON input".to_string()),
        }
    }

    fn parse_literal(&mut self, literal: &str, value: Json) -> Result<Json, String> {
        let expected = literal.as_bytes();
        if self.bytes.len() - self.pos < expected.len() {
            return Err(format!("expected {literal} at offset {}", self.pos));
        }
        if &self.bytes[self.pos..self.pos + expected.len()] != expected {
            return Err(format!("expected {literal} at offset {}", self.pos));
        }
        self.pos += expected.len();
        Ok(value)
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.pos += 1; // '{'
        let mut fields: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(format!("expected object key at offset {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(format!("expected ':' at offset {}", self.pos));
            }
            self.pos += 1;
            let value = self.parse_value()?;
            // CPython: a duplicate key keeps its first position and takes
            // the last value.
            match fields.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = value,
                None => fields.push((key, value)),
            }
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(format!("expected ',' or '}}' at offset {}", self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.pos += 1; // '['
        let mut items: Vec<Json> = Vec::new();
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at offset {}", self.pos)),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        // The caller checked for the opening quote.
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(byte) = self.bytes.get(self.pos).copied() else {
                return Err("unterminated string".to_string());
            };
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.bytes.get(self.pos).copied() else {
                        return Err("unterminated escape sequence".to_string());
                    };
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let high = self.parse_hex4()?;
                            let code = if (0xD800..0xDC00).contains(&high) {
                                if self.bytes.get(self.pos) != Some(&b'\\')
                                    || self.bytes.get(self.pos + 1) != Some(&b'u')
                                {
                                    return Err("unpaired surrogate in \\u escape".to_string());
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err("unpaired surrogate in \\u escape".to_string());
                                }
                                0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&high) {
                                return Err("unpaired surrogate in \\u escape".to_string());
                            } else {
                                high
                            };
                            let Some(c) = char::from_u32(code) else {
                                return Err("invalid \\u escape".to_string());
                            };
                            out.push(c);
                        }
                        _ => {
                            return Err(format!("invalid escape at offset {}", self.pos - 1));
                        }
                    }
                }
                byte if byte < 0x20 => {
                    return Err(format!("control character at offset {}", self.pos - 1));
                }
                byte if byte < 0x80 => out.push(byte as char),
                byte => {
                    // Multi-byte UTF-8: the input is a &str, so the
                    // sequence is well-formed; quote and backslash are
                    // ASCII, so it cannot be split.
                    let len = utf8_len(byte)?;
                    let start = self.pos - 1;
                    let Some(raw) = self.bytes.get(start..start + len) else {
                        return Err("truncated UTF-8 sequence".to_string());
                    };
                    let decoded =
                        std::str::from_utf8(raw).map_err(|_| "invalid UTF-8 in string")?;
                    out.push_str(decoded);
                    self.pos = start + len;
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        let Some(raw) = self.bytes.get(self.pos..self.pos + 4) else {
            return Err("truncated \\u escape".to_string());
        };
        let text = std::str::from_utf8(raw).map_err(|_| "invalid \\u escape")?;
        let code = u32::from_str_radix(text, 16).map_err(|_| "invalid \\u escape")?;
        self.pos += 4;
        Ok(code)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.bytes.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(byte) if byte.is_ascii_digit() => {
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    self.pos += 1;
                }
            }
            _ => return Err(format!("expected a digit at offset {}", self.pos)),
        }
        let mut is_float = false;
        if self.bytes.get(self.pos) == Some(&b'.') {
            is_float = true;
            self.pos += 1;
            self.consume_digits()?;
        }
        if matches!(self.bytes.get(self.pos), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            self.consume_digits()?;
        }
        let raw = self.bytes.get(start..self.pos).ok_or("truncated number")?;
        let text = std::str::from_utf8(raw).map_err(|_| "invalid number")?;
        if is_float {
            text.parse::<f64>()
                .map(Json::Float)
                .map_err(|e| format!("invalid number {text}: {e}"))
        } else {
            // Python keeps arbitrary-precision integers; IDLs do not have
            // any, and falling back to f64 keeps the whole value readable
            // rather than failing the fetch.
            text.parse::<i64>()
                .map(Json::Int)
                .or_else(|_| text.parse::<f64>().map(Json::Float))
                .map_err(|e| format!("invalid number {text}: {e}"))
        }
    }

    fn consume_digits(&mut self) -> Result<(), String> {
        let had_digit = self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_digit());
        if !had_digit {
            return Err(format!("expected a digit at offset {}", self.pos));
        }
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.pos += 1;
        }
        Ok(())
    }
}

fn utf8_len(first_byte: u8) -> Result<usize, String> {
    match first_byte {
        0xC0..=0xDF => Ok(2),
        0xE0..=0xEF => Ok(3),
        0xF0..=0xF7 => Ok(4),
        _ => Err("invalid UTF-8 lead byte".to_string()),
    }
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level * 2 {
        out.push(' ');
    }
}

fn write_pretty(out: &mut String, json: &Json, level: usize) {
    match json {
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, level + 1);
                write_pretty(out, item, level + 1);
            }
            out.push('\n');
            push_indent(out, level);
            out.push(']');
        }
        Json::Object(fields) => {
            if fields.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, level + 1);
                write_json_string(out, key);
                out.push_str(": ");
                write_pretty(out, value, level + 1);
            }
            out.push('\n');
            push_indent(out, level);
            out.push('}');
        }
        leaf => write_leaf(out, leaf),
    }
}

fn write_canonical(out: &mut String, json: &Json) {
    match json {
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_canonical(out, item);
            }
            out.push(']');
        }
        Json::Object(fields) => {
            let mut sorted: Vec<&(String, Json)> = fields.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (index, (key, value)) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_json_string(out, key);
                out.push_str(": ");
                write_canonical(out, value);
            }
            out.push('}');
        }
        leaf => write_leaf(out, leaf),
    }
}

/// Leaves render identically in both printers.
fn write_leaf(out: &mut String, json: &Json) {
    match json {
        Json::Null => out.push_str("null"),
        Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Json::Int(value) => {
            let _ = write!(out, "{value}");
        }
        Json::Float(value) => write_float(out, *value),
        Json::Str(value) => write_json_string(out, value),
        // Containers are rendered by write_pretty / write_canonical, which
        // only route leaves here.
        Json::Array(_) | Json::Object(_) => {}
    }
}

/// Shortest round-trip form with a `.0` suffix for whole floats, matching
/// Python for the values IDLs and records actually carry.
fn write_float(out: &mut String, value: f64) {
    if value.is_nan() {
        out.push_str("NaN");
        return;
    }
    if value.is_infinite() {
        out.push_str(if value > 0.0 { "Infinity" } else { "-Infinity" });
        return;
    }
    let text = format!("{value}");
    out.push_str(&text);
    if !text.contains('.') && !text.contains('e') && !text.contains('E') {
        out.push_str(".0");
    }
}

/// Quote and escape a string like `json.dumps`: only `"`, `\` and control
/// characters are escaped; non-ASCII is kept verbatim.
fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn pretty_matches_python_json_dumps() {
        let value = parse(
            r#"{"name":"noop","n":[1,2,{"b":null,"c":true}],"empty":[],"blank":{},"s":"a\"b\\c\n é"}"#
        )
        .unwrap();
        assert_eq!(
            pretty(&value),
            "{\n  \"name\": \"noop\",\n  \"n\": [\n    1,\n    2,\n    {\n      \"b\": null,\n      \"c\": true\n    }\n  ],\n  \"empty\": [],\n  \"blank\": {},\n  \"s\": \"a\\\"b\\\\c\\n é\"\n}"
        );
    }

    #[test]
    fn parse_round_trips_scalars_and_layouts() {
        let value = parse("{\"i\": 0, \"neg\": -5, \"f\": 1.5, \"t\": true, \"x\": null}").unwrap();
        assert_eq!(value.get("i").unwrap().as_u64(), Some(0));
        assert_eq!(value.get("neg").unwrap().as_u64(), None);
        assert_eq!(value.get("f"), Some(&Json::Float(1.5)));
        assert_eq!(value.get("t"), Some(&Json::Bool(true)));
        assert!(value.get("x").unwrap().is_null());
        // Whole floats gain the ".0" Python prints.
        assert_eq!(
            pretty(&parse("[1.0, 0.25]").unwrap()),
            "[\n  1.0,\n  0.25\n]"
        );
    }

    #[test]
    fn duplicate_keys_keep_first_position_last_value() {
        // CPython's json.loads: {"a": 1, "b": 2, "a": 3} has a first with
        // value 3, so both order and lookup match a Python dict.
        let value = parse("{\"a\": 1, \"b\": 2, \"a\": 3}").unwrap();
        assert_eq!(pretty(&value), "{\n  \"a\": 3,\n  \"b\": 2\n}");
        assert_eq!(value.get("a"), Some(&Json::Int(3)));
    }

    #[test]
    fn parse_rejects_malformed_input() {
        for text in [
            "",
            "{",
            "{\"a\" 1}",
            "[1,]",
            "{\"a\": 01}",
            "nul",
            "\"unterminated",
            "{\"a\": \"\\q\"}",
            "{} trailing",
        ] {
            assert!(parse(text).is_err(), "should have rejected {text:?}");
        }
    }

    #[test]
    fn canonical_sorts_keys_at_every_level() {
        let a = parse("{\"b\": 1, \"a\": {\"d\": [], \"c\": 2}}").unwrap();
        let b = parse("{\"a\": {\"c\": 2, \"d\": []}, \"b\": 1}").unwrap();
        assert_eq!(canonical(&a), canonical(&b));
        let c = parse("{\"a\": {\"c\": 3, \"d\": []}, \"b\": 1}").unwrap();
        assert_ne!(canonical(&a), canonical(&c));
    }

    #[test]
    fn strings_escape_only_quotes_backslashes_and_controls() {
        let input = "tab:\t nl:\n q:\" bs:\\ del:\u{7f} é";
        let expected = concat!("\"tab:\\t nl:\\n q:\\\" bs:\\\\ del:", "\u{7f}", " é\"");
        assert_eq!(pretty(&Json::Str(input.to_string())), expected);
        // \u escapes, including surrogate pairs.
        let value = parse(r#""é 😀""#).unwrap();
        assert_eq!(value, Json::Str("é 😀".to_string()));
    }
}
