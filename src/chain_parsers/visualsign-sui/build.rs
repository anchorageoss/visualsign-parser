use std::{env, fs, path::Path, path::PathBuf};

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/presets");
    println!("cargo:rerun-if-changed=src/integrations");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let visualizers = collect_visualizers()?;

    let code = format!(
        "pub fn available_visualizers() -> Vec<Box<dyn CommandVisualizer>> {{
            vec![
                {}
            ]
        }}",
        visualizers.join(",\n")
    );
    fs::write(out_dir.join("generated_visualizers.rs"), code)?;

    // Auto-generate `pub mod <name>;` declarations for the `presets/` and `integrations/`
    // module trees so adding a new preset is a directory drop-in (no `mod.rs` edit). This
    // keeps `presets/mod.rs` conflict-free across PRs that add new presets.
    emit_module_declarations("src/presets", &out_dir.join("generated_presets_mod.rs"))?;
    emit_module_declarations(
        "src/integrations",
        &out_dir.join("generated_integrations_mod.rs"),
    )?;

    Ok(())
}

/// Walks `folder` and writes `#[path = "<abs>/mod.rs"] pub mod <dir_name>;` for every
/// immediate subdirectory, alphabetically sorted, to `out_path`. The sorted output keeps
/// the generated file stable across filesystem iteration order on different platforms.
///
/// Each entry uses `#[path]` with an absolute path because `include!` causes Rust to
/// resolve `pub mod` declarations relative to the included file's location (`OUT_DIR`),
/// not the file invoking `include!` (`src/presets/mod.rs`). Without `#[path]`, Rust
/// would look for the preset sources inside `OUT_DIR/`.
fn emit_module_declarations(folder: &str, out_path: &Path) -> BuildResult<()> {
    let crate_root = env::var("CARGO_MANIFEST_DIR")?;
    let abs_folder = Path::new(&crate_root).join(folder);

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&abs_folder)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("non-utf8 directory name in {}", abs_folder.display()))?
            .to_string();
        let mod_rs = path.join("mod.rs");
        entries.push((name, mod_rs));
    }
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut body = String::new();
    for (name, mod_rs) in &entries {
        let path_str = mod_rs
            .to_str()
            .ok_or_else(|| format!("non-utf8 path for module file: {}", mod_rs.display()))?;
        body.push_str(&format!("#[path = {path_str:?}]\npub mod {name};\n"));
    }

    fs::write(out_path, body)?;
    Ok(())
}

fn collect_visualizers() -> BuildResult<Vec<String>> {
    let mut visualizers = Vec::new();
    for (folder_name, module_root) in [
        ("src/presets", "crate::presets"),
        ("src/integrations", "crate::integrations"),
    ] {
        for entry in fs::read_dir(folder_name)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("non-utf8 directory name in {folder_name}"))?
                .to_string();
            if dir_name == "coin_transfer" {
                continue;
            }
            visualizers.push(format!(
                "Box::new({}::{}::{}Visualizer)",
                module_root,
                dir_name,
                to_pascal_case(&dir_name)
            ));
        }
    }
    Ok(visualizers)
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}
