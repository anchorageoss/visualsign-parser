//! Canonical JSON: every object's keys in sorted order, at every nesting level.
//!
//! Intermediate outputs embed JSON strings inside Borsh, so the same value must
//! always serialize to the same bytes. `serde_json` is built with its
//! `preserve_order` feature elsewhere in the workspace (pulled in transitively
//! via `indexmap`), which makes `serde_json::Map` keep *insertion* order rather
//! than sorting. These helpers re-key the tree in sorted order before it is
//! serialized, so the output is independent of that build feature.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// Return a copy of `value` with every nested object's entries inserted in
/// sorted key order. Arrays are traversed element-wise; scalars are returned
/// unchanged.
///
/// Inserting in sorted order makes the serialized output alphabetized whether
/// `serde_json::Map` is backed by `BTreeMap` (default) or `IndexMap`
/// (`preserve_order`).
#[must_use]
pub fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(canonicalize_map(map)),
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

/// [`canonicalize`] for a bare map.
#[must_use]
pub fn canonicalize_map(map: &Map<String, Value>) -> Map<String, Value> {
    let sorted: BTreeMap<&String, &Value> = map.iter().collect();
    sorted
        .into_iter()
        .map(|(key, value)| (key.clone(), canonicalize(value)))
        .collect()
}

/// Serialize `value` canonically.
///
/// # Errors
/// Returns the `serde_json` error if serialization fails.
pub fn to_canonical_string(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&canonicalize(value))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn sorts_keys_at_every_level() {
        let mut inner = Map::new();
        inner.insert("z".to_string(), json!(1));
        inner.insert("a".to_string(), json!([{"y": 2, "b": 3}]));
        let mut outer = Map::new();
        outer.insert("m".to_string(), Value::Object(inner));
        outer.insert("c".to_string(), json!("x"));
        assert_eq!(
            to_canonical_string(&Value::Object(outer)).unwrap(),
            r#"{"c":"x","m":{"a":[{"b":3,"y":2}],"z":1}}"#
        );
    }

    #[test]
    fn scalars_and_arrays_keep_their_order() {
        assert_eq!(
            to_canonical_string(&json!([3, "b", null, [2, 1]])).unwrap(),
            r#"[3,"b",null,[2,1]]"#
        );
    }
}
