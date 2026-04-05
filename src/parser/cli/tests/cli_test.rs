use similar::{ChangeTag, TextDiff};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_cli_with_fixtures() {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");

    let test_cases = fs::read_dir(&fixtures_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".input")
        });

    // Fixture names are prefixed with their chain (e.g. "solana-json", "ethereum-text").
    // Build the skip list once from compile-time feature flags.
    let disabled_chain_prefixes: &[&str] = &[
        #[cfg(not(feature = "ethereum"))]
        "ethereum",
        #[cfg(not(feature = "solana"))]
        "solana",
    ];

    for input_file in test_cases {
        let input_path = input_file.path();
        let test_name = input_path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .replace(".input", "");

        if disabled_chain_prefixes
            .iter()
            .any(|p| test_name.starts_with(p))
        {
            println!("Skipping fixture '{test_name}' (chain feature not enabled)");
            continue;
        }

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {input_path:?}"));

        let mut command = Command::new(env!("CARGO_BIN_EXE_parser_cli"));
        for line in input_contents.lines() {
            if !line.trim().is_empty() {
                command.arg(line);
            }
        }

        // Run the CLI program with the input file
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("Failed to execute CLI: {e}"));

        let actual_output = String::from_utf8(output.stdout)
            .unwrap_or_else(|e| panic!("Invalid UTF-8 output: {e}"));

        // Display fixture: compare non-diagnostic fields
        let display_path = fixtures_dir.join(format!("{test_name}.display.expected"));
        assert!(
            display_path.exists(),
            "Display fixture not found: {display_path:?}"
        );

        let actual_json: serde_json::Value = serde_json::from_str(actual_output.trim())
            .unwrap_or_else(|e| {
                panic!("Failed to parse CLI output as JSON for '{test_name}': {e}")
            });

        // Filter to display fields only
        let mut display_payload = actual_json.clone();
        if let Some(fields) = display_payload
            .get_mut("Fields")
            .and_then(|f| f.as_array_mut())
        {
            fields.retain(|f| f.get("Type").and_then(|t| t.as_str()) != Some("diagnostic"));
        }
        let actual_display =
            serde_json::to_string_pretty(&display_payload).expect("failed to serialize");

        let expected_display = fs::read_to_string(&display_path)
            .unwrap_or_else(|_| panic!("Display fixture not found: {display_path:?}"));

        assert_strings_match(
            &test_name,
            "display",
            expected_display.trim(),
            &actual_display,
        );

        // Diagnostics fixture: compare rule/level pairs
        let diagnostics_path = fixtures_dir.join(format!("{test_name}.diagnostics.expected"));
        if diagnostics_path.exists() {
            let expected_diags: Vec<serde_json::Value> = serde_json::from_str(
                &fs::read_to_string(&diagnostics_path)
                    .unwrap_or_else(|_| panic!("Failed to read: {diagnostics_path:?}")),
            )
            .unwrap_or_else(|e| panic!("Failed to parse diagnostics fixture: {e}"));

            let actual_diags: Vec<(String, String)> = actual_json
                .get("Fields")
                .and_then(|f| f.as_array())
                .map(|fields| {
                    fields
                        .iter()
                        .filter(|f| f.get("Type").and_then(|t| t.as_str()) == Some("diagnostic"))
                        .map(|f| {
                            let diag = &f["Diagnostic"];
                            (
                                diag["Rule"].as_str().unwrap().to_string(),
                                diag["Level"].as_str().unwrap().to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Every expected diagnostic must be present
            for expected in &expected_diags {
                let rule = expected["rule"].as_str().unwrap();
                let level = expected["level"].as_str().unwrap();
                assert!(
                    actual_diags.iter().any(|(r, l)| r == rule && l == level),
                    "Test '{test_name}': missing diagnostic rule={rule}, level={level}"
                );
            }

            // No unexpected diagnostics
            assert_eq!(
                expected_diags.len(),
                actual_diags.len(),
                "Test '{test_name}': expected {} diagnostics, got {}. Actual: {:?}",
                expected_diags.len(),
                actual_diags.len(),
                actual_diags
            );
        }
    }
}

fn assert_strings_match(test_name: &str, fixture_type: &str, expected: &str, actual: &str) {
    if expected != actual {
        let diff = TextDiff::from_lines(expected, actual);
        let mut diff_output = String::new();

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            diff_output.push_str(&format!("{sign}{change}"));
        }

        panic!("Test case '{test_name}' ({fixture_type}) failed:\n{diff_output}");
    }
}
