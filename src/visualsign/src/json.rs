//! Canonical JSON for schemas that are serialized into a signed digest.
//!
//! An intermediate output is appended to what the HSM signs, so two runs over
//! the same input have to produce the same bytes. `serde_json::Map` is backed by
//! `BTreeMap` by default and by `IndexMap` under the `preserve_order` feature --
//! which is enabled transitively in this workspace -- so object key order
//! otherwise depends on how the value was built rather than on its contents.

use std::collections::BTreeMap;

use serde_json::Value;

/// Serialize `value` with object keys alphabetized at every nesting level.
///
/// Returns an empty string if serialization fails, which it does not for a
/// `Value` that was parsed from JSON in the first place.
pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&canonicalize(value)).unwrap_or_default()
}

/// Rebuild `value` with every object's keys in sorted order.
///
/// Arrays keep their order -- it is part of the data -- and scalars are copied
/// as they are.
pub fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(canonicalize_map(map)),
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

/// Build a new map whose entries are inserted in sorted key order, with values
/// recursively canonicalized.
///
/// Inserting in sorted order alphabetizes the serialized output whichever map
/// backs `serde_json::Map`: a `BTreeMap` stays sorted, and an `IndexMap`
/// preserves the sorted insertion order it is given.
fn canonicalize_map(map: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    let sorted: BTreeMap<&String, &Value> = map.iter().collect();
    let mut out = serde_json::Map::with_capacity(map.len());
    for (k, v) in sorted {
        out.insert(k.clone(), canonicalize(v));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn alphabetizes_keys() {
        let value = serde_json::json!({"b": 1, "a": 2});
        assert_eq!(canonical_json(&value), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn alphabetizes_every_nesting_level() {
        let value = serde_json::json!({"b": {"d": 1, "c": 2}, "a": [{"f": 3, "e": 4}]});
        assert_eq!(
            canonical_json(&value),
            r#"{"a":[{"e":4,"f":3}],"b":{"c":2,"d":1}}"#
        );
    }

    #[test]
    fn keeps_array_order() {
        let value = serde_json::json!({"a": [3, 1, 2]});
        assert_eq!(canonical_json(&value), r#"{"a":[3,1,2]}"#);
    }

    #[test]
    fn two_orderings_of_the_same_object_agree() {
        let one: Value = serde_json::from_str(r#"{"z":1,"a":{"y":2,"b":3}}"#).expect("valid JSON");
        let other: Value =
            serde_json::from_str(r#"{"a":{"b":3,"y":2},"z":1}"#).expect("valid JSON");
        assert_eq!(canonical_json(&one), canonical_json(&other));
    }
}
