pub fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => charset_safe(s),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(items) => {
            if let Some(hex) = bytes_as_hex(items) {
                hex
            } else {
                let inner: Vec<String> = items.iter().map(format_arg_value).collect();
                format!("[{}]", inner.join(","))
            }
        }
        serde_json::Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", charset_safe(k), format_arg_value(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
}

fn bytes_as_hex(items: &[serde_json::Value]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(items.len());
    for item in items {
        let byte = item.as_u64().filter(|n| *n <= u8::MAX as u64)? as u8;
        bytes.push(byte);
    }
    Some(format!("0x{}", hex::encode(bytes)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scalars() {
        assert_eq!(format_arg_value(&json!("hello")), "hello");
        assert_eq!(format_arg_value(&json!(42)), "42");
        assert_eq!(format_arg_value(&json!(true)), "true");
        assert_eq!(format_arg_value(&serde_json::Value::Null), "null");
    }

    #[test]
    fn test_byte_array_renders_as_hex() {
        assert_eq!(format_arg_value(&json!([1u8, 2, 3])), "0x010203");
        assert_eq!(format_arg_value(&json!([0u8, 255u8])), "0x00ff");
    }

    #[test]
    fn test_non_byte_array_renders_as_bracketed_list() {
        assert_eq!(format_arg_value(&json!([1, 2, 256])), "[1,2,256]");
        assert_eq!(format_arg_value(&json!(["a", "b"])), "[a,b]");
    }

    #[test]
    fn test_empty_array_renders_as_brackets() {
        assert_eq!(format_arg_value(&json!([])), "[]");
    }

    #[test]
    fn test_object_renders_quote_free() {
        let result = format_arg_value(&json!({"side": "buy", "amount": 100}));
        assert!(
            result == "{amount:100,side:buy}" || result == "{side:buy,amount:100}",
            "unexpected: {result}"
        );
    }

    #[test]
    fn test_empty_object_renders_as_braces() {
        assert_eq!(format_arg_value(&json!({})), "{}");
    }

    #[test]
    fn test_charset_safe_no_quotes_or_backslashes() {
        let result = format_arg_value(&json!({"k\"ey": "val\\ue"}));
        assert!(!result.contains('"'), "must not contain quote: {result}");
        assert!(
            !result.contains('\\'),
            "must not contain backslash: {result}"
        );
    }

    #[test]
    fn test_string_with_forbidden_chars_is_stripped() {
        assert_eq!(format_arg_value(&json!("a\"b\\c d")), "abc d");
        assert_eq!(format_arg_value(&json!("a\tb\rc")), "abc");
    }

    #[test]
    fn test_nested_object_does_not_emit_quotes() {
        let nested = json!({"outer": {"inner": "value"}});
        let result = format_arg_value(&nested);
        assert!(
            !result.contains('"'),
            "nested object must be quote-free: {result}"
        );
    }
}
