use std::fs;
use std::path::PathBuf;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::transaction_string_to_visual_sign;

// Helper function to get fixture path
fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    path
}

static FIXTURES: [&str; 2] = ["1559", "legacy"];

#[test]
fn test_with_fixtures() {
    // Get paths for all test cases
    let fixtures_dir = fixture_path("");

    for test_name in FIXTURES {
        let input_path = fixtures_dir.join(format!("{}.input", test_name));

        // Read input file contents
        let input_contents = fs::read_to_string(&input_path)
            .unwrap_or_else(|_| panic!("Failed to read input file: {:?}", input_path));

        // Parse the input to extract transaction data
        let transaction_hex = input_contents.trim();

        // Create options for the transaction
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
        };

        let result = transaction_string_to_visual_sign(transaction_hex, options);

        let actual_output = match result {
            Ok(payload) => {
                // Format the payload as a debug string or custom format
                format!("{:#?}", payload)
            }
            Err(error) => {
                format!("Error: {:?}", error)
            }
        };

        // Construct expected output path
        let expected_path = fixtures_dir.join(format!("{}.expected", test_name));

        // Read expected output
        let expected_output = fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("Expected output file not found: {:?}", expected_path));

        assert_eq!(
            actual_output.trim(),
            expected_output.trim(),
            "Test case '{}' failed",
            test_name
        );
    }
}
