use std::{env, fs, path::Path, path::PathBuf};

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/presets");
    println!("cargo:rerun-if-changed=src/integrations");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let visualizers = collect_visualizers()?;

    // We operate on instructions at a transaction level even though Solana uses programs
    // and that's what we want to create the modules around, but each instruction may
    // individually be special and has to be handled properly. This should allow us to
    // functionally compose instructions at the time of display too.
    let code = format!(
        "pub fn available_visualizers() -> Vec<Box<dyn InstructionVisualizer>> {{
            vec![
                {}
            ]
        }}",
        visualizers.join(",\n")
    );
    fs::write(out_dir.join("generated_visualizers.rs"), code)?;

    // Auto-generate `pub mod <name>;` declarations for the `presets/` and `integrations/`
    // module trees so adding a new preset is a directory drop-in (no `mod.rs` edit). This
    // eliminates the per-PR conflict surface that bit us during the Kyle PR rebase batch.
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
    let mut all_visualizers: Vec<(String, String)> = Vec::new();
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
            let visualizer_string = format!(
                "Box::new({}::{}::{}Visualizer)",
                module_root,
                dir_name,
                to_pascal_case(&dir_name)
            );
            all_visualizers.push((dir_name, visualizer_string));
        }
    }

    // Partition: specific visualizers first, unknown_program visualizer last (it's a catch-all).
    let (unknown, specific): (Vec<_>, Vec<_>) = all_visualizers
        .into_iter()
        .partition(|(name, _)| name == "unknown_program");

    Ok(specific
        .into_iter()
        .map(|(_, vis)| vis)
        .chain(unknown.into_iter().map(|(_, vis)| vis))
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("system"), "System");
        assert_eq!(to_pascal_case("unknown_program"), "UnknownProgram");
        assert_eq!(to_pascal_case("jupiter_swap"), "JupiterSwap");
        assert_eq!(
            to_pascal_case("associated_token_account"),
            "AssociatedTokenAccount"
        );
    }

    #[test]
    fn test_collect_visualizers_unknown_program_last() -> BuildResult<()> {
        let visualizers = collect_visualizers()?;
        if let Some(last) = visualizers.last() {
            assert!(
                last.contains("unknown_program") || last.contains("UnknownProgram"),
                "Unknown program visualizer should be last, but got: {last}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_collect_visualizers_not_empty() -> BuildResult<()> {
        let visualizers = collect_visualizers()?;
        assert!(
            !visualizers.is_empty(),
            "Should have at least one visualizer"
        );
        Ok(())
    }
}
