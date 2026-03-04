use std::collections::HashMap;

use generated::parser::{
    ChainMetadata, Idl, SolanaIdlType, SolanaMetadata, chain_metadata::Metadata,
};

use crate::mapping_parser;

fn parse_idl_file_mapping(mapping_str: &str) -> Option<(String, String, String)> {
    match mapping_parser::parse_mapping(mapping_str) {
        Ok(components) => Some((components.name, components.identifier, components.path)),
        Err(e) => {
            eprintln!("Error parsing IDL mapping: {e}");
            eprintln!("Expected format: Name:/path/to/idl.json:ProgramId");
            eprintln!(
                "Example: JupiterSwap:/home/user/jupiter.json:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
            );
            None
        }
    }
}

fn load_idl_from_file(file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let json_str = std::fs::read_to_string(file_path)?;
    let _: serde_json::Value = serde_json::from_str(&json_str)?;
    Ok(json_str)
}

fn build_idl_mappings_from_files(idl_json_mappings: &[String]) -> (HashMap<String, Idl>, usize) {
    let mut mappings = HashMap::new();
    let mut valid_count = 0;

    for mapping in idl_json_mappings {
        match parse_idl_file_mapping(mapping) {
            Some((idl_name, program_id, file_path)) => match load_idl_from_file(&file_path) {
                Ok(idl_json) => {
                    let idl = Idl {
                        value: idl_json,
                        idl_type: Some(SolanaIdlType::Anchor as i32),
                        idl_version: None,
                        signature: None,
                        program_name: Some(idl_name.clone()),
                    };
                    mappings.insert(program_id.clone(), idl);
                    valid_count += 1;
                    eprintln!(
                        "  Loaded IDL '{idl_name}' from {file_path} and mapped to {program_id}"
                    );
                }
                Err(e) => {
                    eprintln!("  Warning: Failed to load IDL '{idl_name}' from '{file_path}': {e}");
                }
            },
            None => {
                eprintln!(
                    "  Warning: Invalid IDL mapping '{mapping}' (expected format: Name:ProgramId:/path/to/file.json)"
                );
            }
        }
    }

    (mappings, valid_count)
}

/// Creates Solana chain metadata from IDL mappings.
/// Returns `None` if no IDL mappings are provided.
#[must_use]
pub fn create_chain_metadata(idl_json_mappings: &[String]) -> Option<ChainMetadata> {
    if idl_json_mappings.is_empty() {
        return None;
    }

    eprintln!("Loading custom IDLs:");
    let (idl_mappings, valid_count) = build_idl_mappings_from_files(idl_json_mappings);
    eprintln!(
        "Successfully loaded {}/{} IDL mappings\n",
        valid_count,
        idl_json_mappings.len()
    );

    Some(ChainMetadata {
        metadata: Some(Metadata::Solana(SolanaMetadata {
            network_id: None,
            idl: None,
            idl_mappings,
        })),
    })
}
