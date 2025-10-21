use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/presets");
    println!("cargo:rerun-if-changed=src/integrations");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let visualizers = collect_visualizers();

    // We operate on instructions at a transaction level even though Solana uses programs and that's what we want to create the modules around
    // but each instruction may individually be special and has to be handled properly. This should allow use to functionally compose instructions
    // at the time of display too
    let code = format!(
        "pub fn available_visualizers() -> Vec<Box<dyn InstructionVisualizer>> {{
            vec![
                {}
            ]
        }}",
        visualizers.join(",\n")
    );

    fs::write(out_dir.join("generated_visualizers.rs"), code).unwrap();
}

fn collect_visualizers() -> Vec<String> {
    let all_visualizers: Vec<(String, String)> = [
        ("src/presets", "crate::presets"),
        ("src/integrations", "crate::integrations"),
    ]
    .iter()
    .flat_map(|(folder_name, module_root)| {
        fs::read_dir(folder_name)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();

                if path.is_dir() {
                    let dir_name = path.file_name()?.to_str()?.to_string();
                    let visualizer_string = format!(
                        "Box::new({}::{}::{}Visualizer)",
                        module_root,
                        dir_name,
                        to_pascal_case(&dir_name)
                    );
                    Some((dir_name, visualizer_string))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    })
    .collect();

    // Partition: specific visualizers first, unknown_program visualizer last (it's a catch-all)
    let (unknown, specific): (Vec<_>, Vec<_>) = all_visualizers
        .into_iter()
        .partition(|(name, _)| name == "unknown_program");

    specific
        .into_iter()
        .map(|(_, vis)| vis)
        .chain(unknown.into_iter().map(|(_, vis)| vis))
        .collect()
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
    fn test_collect_visualizers_unknown_program_last() {
        let visualizers = collect_visualizers();

        // unknown_program should be last since it's a catch-all
        if let Some(last) = visualizers.last() {
            assert!(
                last.contains("unknown_program") || last.contains("UnknownProgram"),
                "Unknown program visualizer should be last, but got: {}",
                last
            );
        }
    }

    #[test]
    fn test_collect_visualizers_not_empty() {
        let visualizers = collect_visualizers();
        assert!(
            !visualizers.is_empty(),
            "Should have at least one visualizer"
        );
    }
}
