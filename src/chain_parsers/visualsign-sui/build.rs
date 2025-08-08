use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/presets");
    println!("cargo:rerun-if-changed=src/integrations");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let mut visualizers = Vec::new();

    for (folder_name, module_root) in [
        ("src/presets", "crate::presets"),
        ("src/integrations", "crate::integrations"),
    ] {
        for entry in fs::read_dir(folder_name).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();

            if path.is_dir() {
                let dir_name = path.file_name().unwrap().to_str().unwrap();

                if dir_name == "coin_transfer" {
                    continue;
                }

                visualizers.push(format!(
                    "Box::new({}::{}::{}Visualizer)",
                    module_root,
                    dir_name,
                    to_pascal_case(dir_name)
                ));
            }
        }
    }

    let code = format!(
        "pub fn available_visualizers() -> Vec<Box<dyn CommandVisualizer>> {{
            vec![
                {}
            ]
        }}",
        visualizers.join(",\n")
    );

    fs::write(out_dir.join("generated_visualizers.rs"), code).unwrap();
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
