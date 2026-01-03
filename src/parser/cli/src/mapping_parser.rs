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

/// Load and validate JSON file from path
pub fn load_json_file(path: &str) -> Result<String, String> {
    let json_content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read file at {path}: {e}"))?;

    // Validate JSON format
    serde_json::from_str::<serde_json::Value>(&json_content)
        .map_err(|e| format!("Invalid JSON in file {path}: {e}"))?;

    Ok(json_content)
}

#[cfg(test)]
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
}
