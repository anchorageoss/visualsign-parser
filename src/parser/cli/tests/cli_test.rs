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

    for input_file in test_cases {
        let input_path = input_file.path();
        let test_name = input_path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .replace(".input", "");

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {:?}", input_path));

        let mut command = Command::new(env!("CARGO_BIN_EXE_parser_cli"));
        for line in input_contents.lines() {
            if !line.trim().is_empty() {
                command.arg(line);
            }
        }

        // Run the CLI program with the input file
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("Failed to execute CLI: {}", e));
        println!("Output {:?}: {:?}", test_name, output);

        // Construct expected output path
        let expected_path = fixtures_dir.join(format!("{}.expected", test_name));

        // Read expected output
        let expected_output = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {:?}", expected_path));

        let actual_output = String::from_utf8(output.stdout)
            .unwrap_or_else(|e| panic!("Invalid UTF-8 output: {}", e));

        let expected = expected_output.trim();
        let actual = actual_output.trim();

        if expected != actual {
            let diff = TextDiff::from_lines(expected, actual);
            let mut diff_output = String::new();

            for change in diff.iter_all_changes() {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                diff_output.push_str(&format!("{}{}", sign, change));
            }

            panic!("Test case '{}' failed:\n{}", test_name, diff_output);
        }
    }
}
