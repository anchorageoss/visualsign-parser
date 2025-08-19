use serde::Deserialize;
use std::collections::HashMap;
use std::{env, fs, io::Write, path::PathBuf};

fn main() {
    // Directory containing the JSON registry specs
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let registry_dir = manifest_dir.join("static/eip7730/registry");
    println!("cargo:rerun-if-changed={}", registry_dir.display());

    let mut entries: Vec<RegistryEntry> = Vec::new();

    visit_dir(&registry_dir, &mut |path, contents| {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext != "json" {
                return;
            }
        } else {
            return;
        }
        // Primary parse via adapter
        let mut parsed_any = false;
        if let Ok(spec) = visualsign_erc7730_adapter::types::ERC7730::from_json(contents) {
            if let Some(display) = spec.display {
                for (selector, format) in display.formats.into_iter() {
                    if selector.len() == 10 && selector.starts_with("0x") {
                        let fields: Vec<_> = format
                            .fields
                            .into_iter()
                            .map(|f| SimpleField {
                                label: f.label,
                                path: f.path,
                            })
                            .collect();
                        entries.push(RegistryEntry {
                            selector,
                            format_id: format.id,
                            source_file: path
                                .strip_prefix(&registry_dir)
                                .unwrap()
                                .to_string_lossy()
                                .to_string(),
                            fields,
                        });
                        parsed_any = true;
                    }
                }
            }
        }
        if !parsed_any {
            // Fallback lightweight parse of display.formats[*].fields
            #[derive(Deserialize)]
            struct FbField {
                label: Option<String>,
                path: Option<String>,
            }
            #[derive(Deserialize)]
            struct FbFormat {
                #[serde(rename = "$id")]
                id: Option<String>,
                fields: Option<Vec<FbField>>,
            }
            #[derive(Deserialize)]
            struct FbDisplay {
                formats: HashMap<String, FbFormat>,
            }
            #[derive(Deserialize)]
            struct FbSpec {
                display: Option<FbDisplay>,
            }
            if let Ok(fb) = serde_json::from_str::<FbSpec>(contents) {
                if let Some(display) = fb.display {
                    for (selector, fmt) in display.formats.into_iter() {
                        if selector.len() == 10 && selector.starts_with("0x") {
                            let fields: Vec<_> = fmt
                                .fields
                                .unwrap_or_default()
                                .into_iter()
                                .map(|f| SimpleField {
                                    label: f.label.unwrap_or_default(),
                                    path: f.path.unwrap_or_default(),
                                })
                                .collect();
                            entries.push(RegistryEntry {
                                selector,
                                format_id: fmt.id,
                                source_file: path
                                    .strip_prefix(&registry_dir)
                                    .unwrap()
                                    .to_string_lossy()
                                    .to_string(),
                                fields,
                            });
                        }
                    }
                }
            }
        }
    });

    // Generate Rust code
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest_path = out_dir.join("erc7730_registry_gen.rs");
    let mut file = fs::File::create(&dest_path).unwrap();

    // De-duplicate selectors grouping indexes
    let mut selector_map: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, e) in entries.iter().enumerate() {
        selector_map
            .entry(e.selector.clone())
            .or_default()
            .push(idx);
    }

    writeln!(
        file,
        "// @generated automatically by build.rs; DO NOT EDIT\n"
    )
    .unwrap();
    writeln!(
        file,
        "#[derive(Debug)] pub struct GenField {{ pub label: &'static str, pub path: &'static str }}"
    )
    .unwrap();
    writeln!(file, "#[derive(Debug)] pub struct GenFormat {{ pub source_file: &'static str, pub selector: &'static str, pub format_id: Option<&'static str>, pub fields: &'static [GenField] }}").unwrap();

    // Emit fields and formats as separate static arrays for reuse
    for (i, entry) in entries.iter().enumerate() {
        write!(
            file,
            "static FIELDS_{i}: [GenField; {}] = [",
            entry.fields.len()
        )
        .unwrap();
        for f in &entry.fields {
            write!(
                file,
                "GenField {{ label: \"{}\", path: \"{}\" }},",
                escape(&f.label),
                escape(&f.path)
            )
            .unwrap();
        }
        writeln!(file, "];\n").unwrap();
        let format_id = entry
            .format_id
            .as_ref()
            .map(|s| format!("Some(\"{}\")", escape(s)))
            .unwrap_or_else(|| "None".to_string());
        writeln!(file, "static FORMAT_{i}: GenFormat = GenFormat {{ source_file: \"{}\", selector: \"{}\", format_id: {format_id}, fields: &FIELDS_{i} }};\n", escape(&entry.source_file), escape(&entry.selector)).unwrap();
    }

    // Build per-selector format slices
    let mut grouped: Vec<(&String, &Vec<usize>)> = selector_map.iter().collect();
    grouped.sort_by(|a, b| a.0.cmp(b.0));
    for (idx, (_sel, list)) in grouped.iter().enumerate() {
        write!(
            file,
            "static FORMATS_FOR_{idx}: [&GenFormat; {}] = [",
            list.len()
        )
        .unwrap();
        for fi in *list {
            write!(file, "&FORMAT_{fi},").unwrap();
        }
        writeln!(file, "];\n").unwrap();
    }

    // phf map: selector -> slice of &GenFormat
    writeln!(file, "pub static SELECTOR_MAP: phf::Map<&'static str, &'static [&'static GenFormat]> = phf::phf_map! {{").unwrap();
    for (idx, (sel, _)) in grouped.iter().enumerate() {
        writeln!(file, "    \"{}\" => &FORMATS_FOR_{idx},", escape(sel)).unwrap();
    }
    writeln!(file, "}};\n").unwrap();
}

#[derive(Debug)]
struct RegistryEntry {
    selector: String,
    format_id: Option<String>,
    source_file: String,
    fields: Vec<SimpleField>,
}
#[derive(Debug)]
struct SimpleField {
    label: String,
    path: String,
}

fn visit_dir<F: FnMut(&std::path::Path, &str)>(dir: &std::path::Path, cb: &mut F) {
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_dir(&path, cb);
            } else if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(s) = String::from_utf8(bytes) {
                    cb(&path, &s);
                }
            }
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('"', "\\\"")
}
