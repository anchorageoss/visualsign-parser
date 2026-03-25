/// Parsed components of a mapping string: name:path:identifier
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingComponents {
    /// User-provided descriptive name for the contract/program
    pub name: String,
    /// File path to JSON file (ABI or IDL)
    pub path: String,
    /// Program ID (Solana base58) or Contract Address (Ethereum 0x hex)
    pub identifier: String,
}

/// Parse mapping format: `<Name:/path/to/file.json:Identifier>`
///
/// Splits from the right to handle file paths containing colons (e.g., Windows paths
/// like `C:/path/to/file.json`). The last colon separates the identifier, the middle
/// section is the file path, and the first part is the name.
///
/// Returns: `MappingComponents` { name, path, identifier }
pub fn parse_mapping(mapping_str: &str) -> Result<MappingComponents, String> {
    // Split from right to get identifier (last : separator)
    let (name_and_path, identifier) = mapping_str.rsplit_once(':').ok_or_else(|| {
        format!("Invalid mapping format (expected name:path:identifier): {mapping_str}")
    })?;

    // Split name_and_path to get name and path
    let (name, path) = name_and_path.split_once(':').ok_or_else(|| {
        format!("Invalid mapping format (expected name:path:identifier): {mapping_str}")
    })?;

    if name.is_empty() || path.is_empty() || identifier.is_empty() {
        return Err(format!("Mapping components cannot be empty: {mapping_str}"));
    }

    Ok(MappingComponents {
        name: name.to_string(),
        path: path.to_string(),
        identifier: identifier.to_string(),
    })
}

/// Load JSON files from CLI mapping strings and build a `BTreeMap`.
///
/// Each mapping string is parsed, the JSON file is loaded, and `build_value` converts
/// the loaded JSON + components into the target value type. The identifier from the
/// mapping becomes the map key.
///
/// Returns the populated map and the count of successfully loaded entries.
pub fn load_mappings<V>(
    mappings: &[String],
    kind: &str,
    example: &str,
    identifier_label: &str,
    validate_identifier: impl Fn(&str) -> Result<(), String>,
    build_value: impl Fn(&MappingComponents, String) -> V,
) -> (std::collections::BTreeMap<String, V>, usize) {
    let mut map = std::collections::BTreeMap::new();
    let mut valid_count = 0;

    for mapping in mappings {
        match parse_mapping(mapping) {
            Ok(components) => {
                if let Err(e) = validate_identifier(&components.identifier) {
                    eprintln!(
                        "  Warning: Skipping {kind} '{}': invalid {identifier_label} '{}': {e}",
                        components.name, components.identifier
                    );
                    continue;
                }
                match load_json_file(&components.path) {
                    Ok(json) => {
                        let value = build_value(&components, json);
                        let previous = map.insert(components.identifier.clone(), value);
                        if previous.is_some() {
                            eprintln!(
                                "  Warning: Duplicate {identifier_label} '{}' for {kind} '{}'; overwriting previous entry",
                                components.identifier, components.name
                            );
                        } else {
                            valid_count += 1;
                            eprintln!(
                                "  Loaded {kind} '{}' from {} and mapped to {}",
                                components.name, components.path, components.identifier
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  Warning: Failed to load {kind} '{}' from '{}': {e}",
                            components.name, components.path
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!("Error parsing {kind} mapping: {e}");
                eprintln!("Expected format: Name:/path/to/file.json:{identifier_label}");
                eprintln!("Example: {example}");
            }
        }
    }

    (map, valid_count)
}

/// Maximum allowed size for ABI/IDL JSON files (10 MB).
const MAX_JSON_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Load and validate JSON file from path
pub fn load_json_file(path: &str) -> Result<String, String> {
    use std::io::Read;

    let file =
        std::fs::File::open(path).map_err(|e| format!("Failed to read file at {path}: {e}"))?;

    // Use a bounded reader to prevent reading more than MAX_JSON_FILE_SIZE,
    // even if the file grows between the open and the read.
    let mut bounded = file.take(MAX_JSON_FILE_SIZE + 1);
    let mut json_content = String::new();
    bounded
        .read_to_string(&mut json_content)
        .map_err(|e| format!("Failed to read file at {path}: {e}"))?;

    if json_content.len() as u64 > MAX_JSON_FILE_SIZE {
        return Err(format!(
            "File {path} exceeds maximum size (> {MAX_JSON_FILE_SIZE} bytes)"
        ));
    }

    // Validate JSON format
    serde_json::from_str::<serde_json::Value>(&json_content)
        .map_err(|e| format!("Invalid JSON in file {path}: {e}"))?;

    Ok(json_content)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mapping_valid() {
        let result = parse_mapping("MyToken:/path/to/file.json:0xabcd1234")
            .expect("Valid mapping should parse successfully");
        assert_eq!(result.name, "MyToken");
        assert_eq!(result.path, "/path/to/file.json");
        assert_eq!(result.identifier, "0xabcd1234");
    }

    #[test]
    fn test_parse_mapping_windows_path() {
        let result = parse_mapping("MyToken:C:/Users/name/file.json:0xabcd1234")
            .expect("Windows path mapping should parse successfully");
        assert_eq!(result.name, "MyToken");
        assert_eq!(result.path, "C:/Users/name/file.json");
        assert_eq!(result.identifier, "0xabcd1234");
    }

    #[test]
    fn test_parse_mapping_solana_program_id() {
        let result =
            parse_mapping("Jupiter:/path/to/idl.json:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
                .expect("Solana program ID mapping should parse successfully");
        assert_eq!(result.name, "Jupiter");
        assert_eq!(result.path, "/path/to/idl.json");
        assert_eq!(
            result.identifier,
            "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
        );
    }

    #[test]
    fn test_parse_mapping_invalid_format() {
        assert!(parse_mapping("NoColons").is_err());
        assert!(parse_mapping("OnlyOne:Colon").is_err());
        assert!(parse_mapping("::EmptyComponents").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_name() {
        assert!(parse_mapping(":/path/to/file.json:0xabcd").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_path() {
        assert!(parse_mapping("MyToken::0xabcd").is_err());
    }

    #[test]
    fn test_parse_mapping_empty_identifier() {
        assert!(parse_mapping("MyToken:/path/to/file.json:").is_err());
    }

    fn write_temp_json(name: &str, content: &str) -> std::path::PathBuf {
        crate::test_utils::write_temp_json("vsp_tests", name, content)
    }

    #[test]
    fn test_load_json_file_valid() {
        let path = write_temp_json("valid.json", r#"{"a": 1}"#);
        let result = load_json_file(path.to_str().unwrap());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), r#"{"a": 1}"#);
    }

    #[test]
    fn test_load_json_file_invalid_json() {
        let path = write_temp_json("invalid.json", "not json {{{");
        let result = load_json_file(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_load_json_file_missing_file() {
        let result = load_json_file("/nonexistent/path/file.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read file"));
    }

    #[test]
    fn test_load_mappings_valid_files() {
        let path1 = write_temp_json("abi1.json", r#"[{"type":"function"}]"#);
        let path2 = write_temp_json("abi2.json", r#"{"name":"token"}"#);

        let mappings = vec![
            format!("Token1:{}:0xAddr1", path1.display()),
            format!("Token2:{}:0xAddr2", path2.display()),
        ];

        let (map, count) = load_mappings(
            &mappings,
            "ABI",
            "example",
            "Address",
            |_| Ok(()),
            |_comp, json| json.to_uppercase(),
        );

        assert_eq!(count, 2);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("0xAddr1"));
        assert!(map.contains_key("0xAddr2"));
    }

    #[test]
    fn test_load_mappings_empty_input() {
        let mappings: Vec<String> = vec![];
        let (map, count) = load_mappings::<String>(
            &mappings,
            "ABI",
            "example",
            "Address",
            |_| Ok(()),
            |_, json| json,
        );
        assert_eq!(count, 0);
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_mappings_invalid_parse_skipped() {
        let mappings = vec!["bad-format-no-colons".to_string()];
        let (map, count) = load_mappings::<String>(
            &mappings,
            "ABI",
            "example",
            "Address",
            |_| Ok(()),
            |_, json| json,
        );
        assert_eq!(count, 0);
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_mappings_missing_file_skipped() {
        let mappings = vec!["Name:/nonexistent/file.json:0xAddr".to_string()];
        let (map, count) = load_mappings::<String>(
            &mappings,
            "ABI",
            "example",
            "Address",
            |_| Ok(()),
            |_, json| json,
        );
        assert_eq!(count, 0);
        assert!(map.is_empty());
    }

    #[test]
    fn test_load_mappings_mixed_valid_and_invalid() {
        let path = write_temp_json("good.json", r#"{"ok": true}"#);
        let mappings = vec![
            "bad-format".to_string(),
            format!("Good:{}:0xGood", path.display()),
            "Also:/nonexistent/missing.json:0xBad".to_string(),
        ];

        let (map, count) = load_mappings::<String>(
            &mappings,
            "ABI",
            "example",
            "Address",
            |_| Ok(()),
            |_, json| json,
        );
        assert_eq!(count, 1);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("0xGood"));
    }

    #[test]
    fn test_load_mappings_build_value_receives_components_and_json() {
        let path = write_temp_json("comp_test.json", r#"{"data": "value"}"#);
        let mappings = vec![format!("MyName:{}:MyId", path.display())];

        let (map, count) = load_mappings(
            &mappings,
            "TEST",
            "ex",
            "Id",
            |_| Ok(()),
            |components, json| format!("{}|{}|{}", components.name, components.identifier, json),
        );

        assert_eq!(count, 1);
        let value = map.get("MyId").unwrap();
        assert!(value.starts_with("MyName|MyId|"));
        assert!(value.contains(r#""data"#));
    }

    #[test]
    fn test_load_mappings_duplicate_identifier_uses_last() {
        let path1 = write_temp_json("dup1.json", r#"{"v": 1}"#);
        let path2 = write_temp_json("dup2.json", r#"{"v": 2}"#);
        let mappings = vec![
            format!("First:{}:0xSame", path1.display()),
            format!("Second:{}:0xSame", path2.display()),
        ];

        let (map, count) =
            load_mappings::<String>(&mappings, "ABI", "ex", "Addr", |_| Ok(()), |_, json| json);
        assert_eq!(count, 1); // Duplicate not counted
        assert_eq!(map.len(), 1);
        // Last write wins
        assert!(map.get("0xSame").unwrap().contains(r#""v": 2"#));
    }

    #[test]
    fn test_load_json_file_too_large() {
        use std::io::Write;

        let dir = std::env::temp_dir().join("vsp_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("large_{}_{}.json", std::process::id(), "too_big"));
        let mut file = std::fs::File::create(&path).unwrap();
        // Write just over MAX_JSON_FILE_SIZE bytes, derived from the constant
        let bytes_to_write = super::MAX_JSON_FILE_SIZE + 1;
        let chunk = vec![b'x'; bytes_to_write as usize];
        file.write_all(&chunk).unwrap();
        drop(file);

        let result = load_json_file(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum size"));

        std::fs::remove_file(&path).ok();
    }
}
