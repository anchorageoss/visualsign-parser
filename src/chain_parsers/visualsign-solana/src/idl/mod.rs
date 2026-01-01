//! IDL Registry for Solana instruction parsing
//!
//! This module provides utilities for managing Anchor IDLs and integrating them
//! with the solana_parser library for instruction decoding.

use solana_parser::{CustomIdl, CustomIdlConfig, Idl, ProgramType, decode_idl_data};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

/// Registry for managing program IDLs (program_id -> CustomIdlConfig)
///
/// This registry provides a way to:
/// 1. Store custom user-provided IDLs that override built-in ones
/// 2. Check if a program has an IDL available (custom or built-in)
/// 3. Determine if the IDL visualizer should handle a program
///
/// Built-in IDLs are not stored here - they're checked via solana_parser's ProgramType.
#[derive(Clone, Default, Debug)]
pub struct IdlRegistry {
    /// Maps program_id (base58 string) -> CustomIdlConfig
    /// These are user-provided IDLs that override built-ins
    configs: HashMap<String, CustomIdlConfig>,
    /// Maps program_id -> human-readable name (extracted from IDL or provided by user)
    names: HashMap<String, String>,
    /// Maps program_id -> IDL name from metadata.name in JSON
    idl_names: HashMap<String, String>,
}

impl IdlRegistry {
    /// Create empty registry (built-in IDLs handled by solana_parser directly)
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            names: HashMap::new(),
            idl_names: HashMap::new(),
        }
    }

    /// Create registry with custom IDL mappings from IDL JSON strings
    ///
    /// # Arguments
    /// * `idl_mappings` - Map of program_id (base58) to (IDL JSON string, user-provided name)
    ///
    /// # Returns
    /// * `Ok(IdlRegistry)` with the custom IDLs configured to override built-ins
    /// * `Err` if any IDL JSON is invalid
    pub fn from_idl_mappings(
        idl_mappings: HashMap<String, (String, String)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut configs = HashMap::new();
        let mut names = HashMap::new();
        let mut idl_names = HashMap::new();

        for (program_id, (idl_json, program_name)) in idl_mappings {
            // Extract IDL name from JSON metadata
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&idl_json) {
                if let Some(metadata) = json_value.get("metadata") {
                    if let Some(idl_name) = metadata.get("name").and_then(|n| n.as_str()) {
                        idl_names.insert(program_id.clone(), idl_name.to_string());
                    }
                }
            }

            // Convert IDL JSON to solana_parser CustomIdlConfig
            // override_builtin = true so user IDLs override built-in ones
            let config = CustomIdlConfig::from_json(idl_json, true);
            configs.insert(program_id.clone(), config);
            names.insert(program_id, program_name);
        }

        Ok(Self {
            configs,
            names,
            idl_names,
        })
    }

    /// Get all IDL configs for use with solana_parser
    ///
    /// This returns the custom user-provided IDLs. Built-in IDLs are handled
    /// automatically by solana_parser when these are passed to parse_transaction_with_idls.
    ///
    /// Reserved for future integration with solana_parser's batch transaction parsing.
    #[allow(dead_code)]
    pub fn get_all_configs(&self) -> &HashMap<String, CustomIdlConfig> {
        &self.configs
    }

    /// Check if we have an IDL available (built-in or custom) for a program
    ///
    /// Checks in order:
    /// 1. Custom user-provided IDLs
    /// 2. Built-in IDLs from solana_parser (13 programs)
    pub fn has_idl(&self, program_id: &Pubkey) -> bool {
        let program_id_str = program_id.to_string();

        // Check custom IDLs first
        if self.configs.contains_key(&program_id_str) {
            return true;
        }

        // Check built-in IDLs (13 programs from solana_parser)
        ProgramType::from_program_id(&program_id_str).is_some()
    }

    /// Get a human-readable program name for known programs with built-in IDLs
    ///
    /// Returns the program name if found, otherwise returns a truncated program ID
    pub fn get_program_name(&self, program_id: &Pubkey) -> String {
        let program_id_str = program_id.to_string();

        // First check if we have a custom name for this program
        if let Some(name) = self.names.get(&program_id_str) {
            return name.clone();
        }

        // Then check built-in IDLs
        if let Some(program_type) = ProgramType::from_program_id(&program_id_str) {
            program_type.program_name().to_string()
        } else {
            // Unknown program with no IDL or no name
            format!("Program {}", &program_id_str[..8])
        }
    }

    /// Get the IDL name from metadata.name in the IDL JSON
    ///
    /// Returns the IDL name if found in metadata
    pub fn get_idl_name(&self, program_id: &Pubkey) -> Option<String> {
        let program_id_str = program_id.to_string();
        self.idl_names.get(&program_id_str).cloned()
    }

    /// Get the parsed Idl for a program if available
    pub fn get_idl(&self, program_id: &str) -> Option<Idl> {
        if let Some(config) = self.configs.get(program_id) {
            match &config.idl {
                CustomIdl::Parsed(idl) => Some(idl.clone()),
                CustomIdl::Json(json) => decode_idl_data(json).ok(),
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_new_registry_is_empty() {
        let registry = IdlRegistry::new();
        assert_eq!(registry.get_all_configs().len(), 0);
    }

    #[test]
    fn test_has_builtin_idl() {
        let registry = IdlRegistry::new();

        // Jupiter has a built-in IDL in solana_parser
        let jupiter_id = Pubkey::from_str("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB").unwrap();
        assert!(registry.has_idl(&jupiter_id));
    }

    #[test]
    fn test_get_program_name() {
        let registry = IdlRegistry::new();

        // Jupiter has a built-in IDL
        let jupiter_id = Pubkey::from_str("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB").unwrap();
        let name = registry.get_program_name(&jupiter_id);
        assert!(name.contains("jupiter") || name.contains("Jupiter"));
    }

    #[test]
    fn test_unknown_program_without_idl() {
        let registry = IdlRegistry::new();

        // Random program with no IDL
        let unknown_id = Pubkey::new_unique();

        assert!(!registry.has_idl(&unknown_id));
        assert!(
            registry
                .get_program_name(&unknown_id)
                .starts_with("Program")
        );
    }
}
