#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Helper to run the parser_cli binary with given args and return (stdout, stderr).
fn run_cli_full(args: &[&str]) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
        .args(args)
        .output()
        .expect("Failed to execute parser_cli");
    assert!(
        output.status.success(),
        "CLI exited with error. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("Invalid UTF-8 output"),
        String::from_utf8(output.stderr).expect("Invalid UTF-8 stderr"),
    )
}

/// Helper to run the parser_cli binary with given args and return stdout.
fn run_cli(args: &[&str]) -> String {
    run_cli_full(args).0
}

/// Helper to write a temp JSON file and return its path.
/// Duplicated from `test_utils::write_temp_json` because `crate::test_utils`
/// is behind `#[cfg(test)]` in `lib.rs` and therefore not compiled for
/// integration tests.
fn write_temp_json(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("vsp_cli_tests");
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(format!(
        "{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::write(&path, content).expect("write temp file");
    path
}

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
        let test_name = input_path.file_stem().unwrap().to_str().unwrap();

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

        let expected_display = fs::read_to_string(&display_path)
            .unwrap_or_else(|_| panic!("Display fixture not found: {display_path:?}"));

        // Try JSON parsing; fall back to string comparison for text/human output
        match serde_json::from_str::<serde_json::Value>(actual_output.trim()) {
            Ok(actual_json) => {
                // JSON output: filter diagnostics and check membership
                let mut display_payload = actual_json.clone();
                if let Some(fields) = display_payload
                    .get_mut("Fields")
                    .and_then(|f| f.as_array_mut())
                {
                    fields.retain(|f| f.get("Type").and_then(|t| t.as_str()) != Some("diagnostic"));
                }

                let expected_json: serde_json::Value =
                    serde_json::from_str(expected_display.trim()).unwrap_or_else(|e| {
                        panic!("Failed to parse display fixture as JSON for '{test_name}': {e}")
                    });

                assert_json_contains(test_name, &expected_json, &display_payload, "");

                // Diagnostics fixture: compare rule, level, and instruction_index
                let diagnostics_path =
                    fixtures_dir.join(format!("{test_name}.diagnostics.expected"));
                if diagnostics_path.exists() {
                    let expected_diags: Vec<serde_json::Value> = serde_json::from_str(
                        &fs::read_to_string(&diagnostics_path)
                            .unwrap_or_else(|_| panic!("Failed to read: {diagnostics_path:?}")),
                    )
                    .unwrap_or_else(|e| panic!("Failed to parse diagnostics fixture: {e}"));

                    let actual_diags: Vec<(String, String, Option<u32>)> = actual_json
                        .get("Fields")
                        .and_then(|f| f.as_array())
                        .map(|fields| {
                            fields
                                .iter()
                                .filter(|f| {
                                    f.get("Type").and_then(|t| t.as_str()) == Some("diagnostic")
                                })
                                .map(|f| {
                                    let diag = &f["Diagnostic"];
                                    (
                                        diag["Rule"].as_str().unwrap().to_string(),
                                        diag["Level"].as_str().unwrap().to_string(),
                                        diag["InstructionIndex"].as_u64().map(|n| n as u32),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    // Every expected diagnostic must be present (rule + level + optional instruction_index)
                    for expected in &expected_diags {
                        let rule = expected["rule"].as_str().unwrap();
                        let level = expected["level"].as_str().unwrap();
                        let expected_idx = expected
                            .get("instruction_index")
                            .and_then(|v| v.as_u64())
                            .map(|n| n as u32);
                        assert!(
                            actual_diags.iter().any(|(r, l, idx)| {
                                r == rule
                                    && l == level
                                    && (expected_idx.is_none() || *idx == expected_idx)
                            }),
                            "Test '{test_name}': missing diagnostic rule={rule}, level={level}, instruction_index={expected_idx:?}"
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
            Err(_) => {
                // Non-JSON output (text/human): plain string comparison
                assert_strings_match(
                    test_name,
                    "display",
                    expected_display.trim(),
                    actual_output.trim(),
                );
            }
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

/// Recursively checks that every field in `expected` is present in `actual`.
/// Objects: every key in expected must exist in actual with a matching value.
/// Arrays: must have the same length and each element must match.
/// Scalars: must be equal.
fn assert_json_contains(
    test_name: &str,
    expected: &serde_json::Value,
    actual: &serde_json::Value,
    path: &str,
) {
    match (expected, actual) {
        (serde_json::Value::Object(exp_map), serde_json::Value::Object(act_map)) => {
            for (key, exp_val) in exp_map {
                let field_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let act_val = act_map.get(key).unwrap_or_else(|| {
                    panic!("Test '{test_name}': missing key '{field_path}' in actual output")
                });
                assert_json_contains(test_name, exp_val, act_val, &field_path);
            }
        }
        (serde_json::Value::Array(exp_arr), serde_json::Value::Array(act_arr)) => {
            assert_eq!(
                exp_arr.len(),
                act_arr.len(),
                "Test '{test_name}': array length mismatch at '{path}'"
            );
            for (i, (exp_val, act_val)) in exp_arr.iter().zip(act_arr.iter()).enumerate() {
                assert_json_contains(test_name, exp_val, act_val, &format!("{path}[{i}]"));
            }
        }
        _ => {
            assert_eq!(
                expected, actual,
                "Test '{test_name}': value mismatch at '{path}'"
            );
        }
    }
}

/// ERC-20 transfer(address,uint256) to an unknown contract 0x1111...1111.
/// Without a custom ABI mapping the built-in ERC-20 visualizer decodes it as
/// "ERC20 Transfer". With a custom ABI the dynamic decoder takes over and
/// shows the raw function signature "transfer(address,uint256)".
const ERC20_TRANSFER_TX: &str = "02f86c0180830f4240843b9aca00830186a094111111111111111111111111111111111111111180b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead00000000000000000000000000000000000000000000000000000000000f4240c0";

const ERC20_TRANSFER_ABI: &str = r#"[{
    "type": "function",
    "name": "transfer",
    "inputs": [
        {"name": "to", "type": "address"},
        {"name": "amount", "type": "uint256"}
    ],
    "outputs": [{"name": "", "type": "bool"}],
    "stateMutability": "nonpayable"
}]"#;

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_abi_json_mappings() {
    let abi_path = write_temp_json("erc20_transfer.json", ERC20_TRANSFER_ABI);
    let mapping = format!(
        "TestERC20:{}:0x1111111111111111111111111111111111111111",
        abi_path.display()
    );

    let output = run_cli(&[
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "--abi-json-mappings",
        &mapping,
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    // Dynamic ABI decoder produces "transfer" as the function name in the title
    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");

    let fields = json["Fields"]
        .as_array()
        .expect("Fields should be an array");

    // Find the preview_layout field produced by the dynamic ABI decoder
    let abi_field = fields.iter().find(|f| {
        f["Type"] == "preview_layout"
            && f["PreviewLayout"]["Title"]["Text"]
                .as_str()
                .is_some_and(|t| t == "transfer")
    });
    assert!(
        abi_field.is_some(),
        "Expected a preview_layout with Title 'transfer' from dynamic ABI decoding, got: {output}"
    );

    // Verify the function signature appears in the subtitle
    let subtitle = abi_field.unwrap()["PreviewLayout"]["Subtitle"]["Text"]
        .as_str()
        .unwrap();
    assert!(
        subtitle.contains("transfer(address,uint256)"),
        "Subtitle should contain function signature, got: {subtitle}"
    );
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_without_abi_uses_builtin_visualizer() {
    let output = run_cli(&[
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");

    let fields = json["Fields"]
        .as_array()
        .expect("Fields should be an array");

    // Without a custom ABI, the built-in ERC-20 visualizer should handle it
    let erc20_field = fields
        .iter()
        .find(|f| f["Label"].as_str().is_some_and(|l| l == "ERC20 Transfer"));
    assert!(
        erc20_field.is_some(),
        "Expected built-in 'ERC20 Transfer' field without custom ABI, got: {output}"
    );
}

#[test]
#[cfg(feature = "ethereum")]
fn test_cli_ethereum_abi_invalid_file_still_parses() {
    // Pointing to a nonexistent ABI file should not crash the CLI —
    // it should fall back to the built-in visualizer.
    let mapping = "Bad:/nonexistent/abi.json:0x1111111111111111111111111111111111111111";

    let output = run_cli(&[
        "--chain",
        "ethereum",
        "--network",
        "ETHEREUM_MAINNET",
        "-o",
        "json",
        "--abi-json-mappings",
        mapping,
        "-t",
        ERC20_TRANSFER_TX,
    ]);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Ethereum Transaction");
}

#[test]
#[cfg(feature = "solana")]
fn test_cli_solana_idl_json_mappings() {
    // Minimal Anchor IDL — enough to verify the flag is wired through.
    // The actual IDL won't match the transaction's program, but it should
    // still load without error and be present in metadata.
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "instructions": []
    }"#;
    let idl_path = write_temp_json("test_idl.json", idl_json);
    let mapping = format!(
        "TestProgram:{}:TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        idl_path.display()
    );

    // Use the same Solana transaction from the existing fixture
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let input =
        fs::read_to_string(fixtures_dir.join("solana-json.input")).expect("read solana-json.input");

    let mut args: Vec<&str> = Vec::new();
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in &lines {
        args.push(line);
    }
    args.push("--idl-json-mappings");
    args.push(&mapping);

    let (stdout, stderr) = run_cli_full(&args);

    // Verify the mapping was actually loaded — check the per-entry success line
    assert!(
        stderr.contains("Loaded IDL 'TestProgram'"),
        "Expected per-mapping 'Loaded IDL' line in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("Successfully loaded 1/1 IDL mappings"),
        "Expected 1/1 IDL mappings loaded, got: {stderr}"
    );

    // The transaction should still parse successfully
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Solana Transaction");
    assert!(json["Fields"].as_array().is_some_and(|f| !f.is_empty()));
}

#[test]
#[cfg(feature = "solana")]
fn test_cli_solana_idl_invalid_file_still_parses() {
    let mapping = "Bad:/nonexistent/idl.json:11111111111111111111111111111111";

    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let input =
        fs::read_to_string(fixtures_dir.join("solana-json.input")).expect("read solana-json.input");

    let mut args: Vec<&str> = Vec::new();
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in &lines {
        args.push(line);
    }
    args.push("--idl-json-mappings");
    args.push(mapping);

    let output = run_cli(&args);

    let json: serde_json::Value =
        serde_json::from_str(&output).expect("CLI output should be valid JSON");
    assert_eq!(json["Title"], "Solana Transaction")
}
