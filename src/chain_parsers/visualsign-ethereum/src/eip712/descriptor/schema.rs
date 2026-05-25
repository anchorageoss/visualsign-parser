//! Minimal Draft 7 JSON Schema validator — the subset used by `erc7730-v1.schema.json`.
//!
//! We avoid pulling in a full crate (e.g., `jsonschema`) to keep build/test surface small.
//! Supported keywords: `type`, `properties`, `required`, `items`, `pattern`, `enum`,
//! `anyOf`, `$ref` (local `#/definitions/...` only).

use serde_json::Value;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{path}: {message}")]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

pub fn validate(instance: &Value, schema: &Value) -> Result<(), Vec<ValidationError>> {
    let mut errs = Vec::new();
    check(instance, schema, schema, "$", &mut errs);
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn check(
    instance: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    errs: &mut Vec<ValidationError>,
) {
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        match resolve_ref(r, root) {
            Some(resolved) => check(instance, &resolved, root, path, errs),
            None => errs.push(ValidationError {
                path: path.into(),
                message: format!("unresolved $ref: {r}"),
            }),
        }
        return;
    }

    if let Some(types) = schema.get("type") {
        let type_strs: Vec<&str> = match types {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };
        if !type_strs.is_empty() && !type_matches(&type_strs, instance) {
            errs.push(ValidationError {
                path: path.into(),
                message: format!("type mismatch: expected {type_strs:?}"),
            });
            return;
        }
    }

    if let (Some(props), Some(obj)) = (
        schema.get("properties").and_then(|v| v.as_object()),
        instance.as_object(),
    ) {
        for (k, sub) in props {
            if let Some(v) = obj.get(k) {
                check(v, sub, root, &format!("{path}.{k}"), errs);
            }
        }
    }

    if let (Some(req), Some(obj)) = (
        schema.get("required").and_then(|v| v.as_array()),
        instance.as_object(),
    ) {
        for r in req {
            if let Some(name) = r.as_str() {
                if !obj.contains_key(name) {
                    errs.push(ValidationError {
                        path: path.into(),
                        message: format!("missing required: {name}"),
                    });
                }
            }
        }
    }

    if let (Some(items_schema), Some(arr)) = (schema.get("items"), instance.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            check(v, items_schema, root, &format!("{path}[{i}]"), errs);
        }
    }

    if let Some(pattern) = schema.get("pattern").and_then(|v| v.as_str()) {
        if let Some(s) = instance.as_str() {
            if !simple_regex_match(pattern, s) {
                errs.push(ValidationError {
                    path: path.into(),
                    message: format!("does not match pattern {pattern}"),
                });
            }
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_values.iter().any(|v| v == instance) {
            errs.push(ValidationError {
                path: path.into(),
                message: format!("not in enum: {enum_values:?}"),
            });
        }
    }

    if let Some(any_of) = schema.get("anyOf").and_then(|v| v.as_array()) {
        let any_pass = any_of.iter().any(|sub| {
            let mut local = Vec::new();
            check(instance, sub, root, path, &mut local);
            local.is_empty()
        });
        if !any_pass {
            errs.push(ValidationError {
                path: path.into(),
                message: "anyOf: no branch matched".into(),
            });
        }
    }
}

fn type_matches(expected: &[&str], v: &Value) -> bool {
    let actual = match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    expected
        .iter()
        .any(|t| *t == actual || (actual == "integer" && *t == "number"))
}

fn resolve_ref(r: &str, root: &Value) -> Option<Value> {
    // Only support local refs: "#/definitions/Foo"
    let r = r.strip_prefix("#/")?;
    let mut cur = root.clone();
    for seg in r.split('/') {
        cur = cur.get(seg)?.clone();
    }
    Some(cur)
}

fn simple_regex_match(pattern: &str, input: &str) -> bool {
    match regex::Regex::new(pattern) {
        Ok(re) => re.is_match(input),
        Err(_) => true, // tolerate unparseable patterns rather than fail
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_minimum_object() {
        let schema =
            json!({"type": "object", "required": ["a"], "properties": {"a": {"type": "string"}}});
        let good = json!({"a": "hi"});
        let bad = json!({});
        assert!(validate(&good, &schema).is_ok());
        assert!(validate(&bad, &schema).is_err());
    }

    #[test]
    fn validates_erc2612_against_v1_schema() {
        use crate::eip712::descriptor::read_optional_static;
        let (Some(schema_text), Some(descriptor_text)) = (
            read_optional_static("specs/erc7730-v1.schema.json"),
            read_optional_static("ercs/eip712-erc2612-permit.json"),
        ) else {
            eprintln!("skip: static/eip7730 not present");
            return;
        };
        let schema: Value = serde_json::from_str(&schema_text).unwrap();
        let descriptor: Value = serde_json::from_str(&descriptor_text).unwrap();
        if let Err(errs) = validate(&descriptor, &schema) {
            panic!("erc2612 descriptor failed validation: {errs:#?}");
        }
    }
}
