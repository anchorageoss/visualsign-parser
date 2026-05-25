//! Walks `static/eip7730/`, validates each descriptor against the v1 schema,
//! and emits an embedded static lookup table to `$OUT_DIR/erc7730_embedded.rs`.
//!
//! This script duplicates the (very small) JSON Schema validator from
//! `src/eip712/descriptor/schema.rs` because build scripts cannot depend on
//! the crate they build.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let crate_root = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let static_root = PathBuf::from(&crate_root).join("static/eip7730");

    println!("cargo:rerun-if-changed=static/eip7730");

    if !static_root.exists() {
        // Submodule not initialized: emit an empty table so the crate still compiles.
        write_out(EMPTY_TABLE);
        println!(
            "cargo:warning=static/eip7730/ missing - embedded descriptor registry is empty. \
             Run `git submodule update --init` to populate."
        );
        return;
    }

    let schema_path = static_root.join("specs/erc7730-v1.schema.json");
    let schema_text = match fs::read_to_string(&schema_path) {
        Ok(s) => s,
        Err(e) => {
            write_out(EMPTY_TABLE);
            println!(
                "cargo:warning=could not read schema {}: {e}; registry left empty",
                schema_path.display()
            );
            return;
        }
    };
    let schema: Value = match serde_json::from_str(&schema_text) {
        Ok(v) => v,
        Err(e) => {
            write_out(EMPTY_TABLE);
            println!("cargo:warning=could not parse schema: {e}; registry left empty");
            return;
        }
    };

    let mut entries = Vec::<EntryDraft>::new();

    for dir in ["registry", "ercs"] {
        let root = static_root.join(dir);
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    println!("cargo:warning=walkdir: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            if name.starts_with("common-")
                || name.starts_with("template-")
                || name.ends_with(".schema.json")
            {
                continue;
            }

            let kind = if name.starts_with("eip712-") {
                DescriptorKind::Eip712
            } else if name.starts_with("calldata-") {
                DescriptorKind::Calldata
            } else {
                continue;
            };

            let raw = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    println!("cargo:warning=read {}: {e}", path.display());
                    continue;
                }
            };
            let json: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    println!("cargo:warning=parse {}: {e}", path.display());
                    continue;
                }
            };

            if let Err(errs) = validate(&json, &schema) {
                println!(
                    "cargo:warning=descriptor {} failed schema validation: {:?}",
                    path.display(),
                    errs.first()
                );
                continue;
            }

            let mut deployments: Vec<(u64, String)> = Vec::new();
            let mut primary_types = BTreeSet::new();

            let ctx = json.get("context");
            if let Some(eip712) = ctx.and_then(|c| c.get("eip712")) {
                for d in eip712
                    .get("deployments")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let (Some(c), Some(a)) = (
                        d.get("chainId").and_then(|v| v.as_u64()),
                        d.get("address").and_then(|v| v.as_str()),
                    ) {
                        deployments.push((c, a.to_lowercase()));
                    }
                }
                for s in eip712
                    .get("schemas")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(pt) = s.get("primaryType").and_then(|v| v.as_str()) {
                        primary_types.insert(pt.to_string());
                    }
                }
            }
            if let Some(contract) = ctx.and_then(|c| c.get("contract")) {
                for d in contract
                    .get("deployments")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let (Some(c), Some(a)) = (
                        d.get("chainId").and_then(|v| v.as_u64()),
                        d.get("address").and_then(|v| v.as_str()),
                    ) {
                        deployments.push((c, a.to_lowercase()));
                    }
                }
            }

            let entity = path
                .strip_prefix(&static_root)
                .ok()
                .and_then(|rel| rel.iter().nth(1))
                .and_then(|os| os.to_str())
                .unwrap_or("unknown")
                .to_string();

            let rel_source = path
                .strip_prefix(&crate_root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            entries.push(EntryDraft {
                entity,
                source: rel_source,
                kind,
                deployments,
                primary_types: primary_types.into_iter().collect(),
            });
        }
    }

    entries.sort_by(|a, b| {
        a.deployments
            .cmp(&b.deployments)
            .then(a.source.cmp(&b.source))
    });

    let out = render_table(&crate_root, &entries);
    write_out(&out);
}

const EMPTY_TABLE: &str = "// AUTO-GENERATED by build.rs (empty: static/eip7730/ missing)\n\
pub(super) const EMBEDDED_DESCRIPTORS: &[EmbeddedDescriptorEntry] = &[];\n";

#[derive(Debug)]
struct EntryDraft {
    entity: String,
    source: String,
    kind: DescriptorKind,
    deployments: Vec<(u64, String)>,
    primary_types: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum DescriptorKind {
    Eip712,
    Calldata,
}

fn render_table(crate_root: &str, entries: &[EntryDraft]) -> String {
    let mut s = String::new();
    s.push_str("// AUTO-GENERATED by build.rs. DO NOT EDIT.\n\n");
    s.push_str("pub(super) const EMBEDDED_DESCRIPTORS: &[EmbeddedDescriptorEntry] = &[\n");
    for e in entries {
        s.push_str("    EmbeddedDescriptorEntry {\n");
        s.push_str(&format!("        entity: {:?},\n", e.entity));
        s.push_str(&format!("        source_path: {:?},\n", e.source));
        s.push_str(&format!(
            "        kind: DescriptorKind::{},\n",
            match e.kind {
                DescriptorKind::Eip712 => "Eip712",
                DescriptorKind::Calldata => "Calldata",
            }
        ));
        s.push_str("        deployments: &[\n");
        for (c, a) in &e.deployments {
            s.push_str(&format!("            ({c}, {a:?}),\n"));
        }
        s.push_str("        ],\n");
        s.push_str("        primary_types: &[\n");
        for pt in &e.primary_types {
            s.push_str(&format!("            {pt:?},\n"));
        }
        s.push_str("        ],\n");
        s.push_str(&format!(
            "        json: include_str!({:?}),\n",
            format!("{crate_root}/{}", e.source)
        ));
        s.push_str("    },\n");
    }
    s.push_str("];\n");
    s
}

fn write_out(contents: &str) {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    let dest = Path::new(&out_dir).join("erc7730_embedded.rs");
    let mut f = fs::File::create(&dest).expect("create OUT_DIR/erc7730_embedded.rs");
    f.write_all(contents.as_bytes()).expect("write");
}

// ----- Inline schema validator (mirrors src/eip712/descriptor/schema.rs) -----

fn validate(instance: &Value, schema: &Value) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    check(instance, schema, schema, "$", &mut errs);
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn check(instance: &Value, schema: &Value, root: &Value, path: &str, errs: &mut Vec<String>) {
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        if let Some(resolved) = resolve_ref(r, root) {
            check(instance, &resolved, root, path, errs);
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
            errs.push(format!("{path}: type {type_strs:?}"));
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
            if let Some(n) = r.as_str() {
                if !obj.contains_key(n) {
                    errs.push(format!("{path}: missing {n}"));
                }
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), instance.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            check(v, items, root, &format!("{path}[{i}]"), errs);
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
    let r = r.strip_prefix("#/")?;
    let mut cur = root.clone();
    for seg in r.split('/') {
        cur = cur.get(seg)?.clone();
    }
    Some(cur)
}
