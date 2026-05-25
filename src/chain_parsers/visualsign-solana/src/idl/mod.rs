//! IDL Registry for Solana instruction parsing
//!
//! This module provides utilities for managing Anchor IDLs and integrating them
//! with the solana_parser library for instruction decoding.

pub mod builtin_programs;
pub mod signature;

use crate::idl::builtin_programs::canonical_name;
use solana_parser::{CustomIdl, CustomIdlConfig, Idl, ProgramType, decode_idl_data};
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;

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
    configs: BTreeMap<String, CustomIdlConfig>,
    /// Maps program_id -> human-readable name (extracted from IDL or provided by user)
    names: BTreeMap<String, String>,
    /// Maps program_id -> IDL name from metadata.name in JSON
    idl_names: BTreeMap<String, String>,
}

impl IdlRegistry {
    /// Create empty registry (built-in IDLs handled by solana_parser directly)
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
            names: BTreeMap::new(),
            idl_names: BTreeMap::new(),
        }
    }

    /// Create registry with custom IDL mappings from IDL JSON strings
    ///
    /// # Arguments
    /// * `idl_mappings` - Map of program_id (base58) to (IDL JSON string, user-provided name)
    ///
    /// # Returns
    /// * `Ok(IdlRegistry)` populated only with mappings for programs that have
    ///   no canonical identity (see `canonical_name`). Trusted built-ins
    ///   (native runtime programs, core SPL, and programs with a built-in IDL
    ///   in `solana_parser`) are intentionally not overrideable and are
    ///   dropped here.
    /// * `Err` if any IDL JSON is invalid
    ///
    /// # Security
    ///
    /// Mappings whose `program_id` resolves to a trusted built-in are dropped:
    /// the registry must never hold a caller-supplied IDL body for a program
    /// with a canonical identity. The attacker-controlled body would otherwise
    /// drive instruction decoding (arg/account names, value formatting) via
    /// the `unknown_program` IDL path even though the displayed program name
    /// is canonical (see PRS-237). The upstream `extract_idl_mappings` filter
    /// is the primary defence; this filter is a registry-level invariant in
    /// case a future caller bypasses extraction.
    pub fn from_idl_mappings(
        idl_mappings: BTreeMap<String, (String, String)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut configs = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut idl_names = BTreeMap::new();

        for (program_id, (idl_json, program_name)) in idl_mappings {
            // Refuse IDL overrides for trusted built-ins. See doc comment above.
            if canonical_name(&program_id).is_some() {
                continue;
            }

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
    pub fn get_all_configs(&self) -> &BTreeMap<String, CustomIdlConfig> {
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
    /// Returns the program name if found, otherwise returns a truncated program ID.
    ///
    /// # Security
    ///
    /// Caller-supplied IDL `program_name` values never override the canonical
    /// name of a trusted built-in program (native runtime programs, core SPL
    /// programs, or any program shipped with a built-in IDL in `solana_parser`).
    /// This prevents a compromised wallet from mislabeling, for example, the
    /// System Program as "Phantom Wallet" via crafted `idl_mappings`
    /// (see PRS-237).
    pub fn get_program_name(&self, program_id: &Pubkey) -> String {
        let program_id_str = program_id.to_string();

        // Canonical names for trusted built-ins always win. This is the
        // primary defence against the PRS-237 mislabeling attack.
        if let Some(name) = canonical_name(&program_id_str) {
            return name.to_string();
        }

        // For non-trusted programs, a caller-supplied name is safe to show
        // because there is no canonical identity to confuse a signer with.
        if let Some(name) = self.names.get(&program_id_str) {
            return name.clone();
        }

        // Unknown program with no IDL or no name
        format!("Program {}", &program_id_str[..8])
    }

    /// Get the IDL name from metadata.name in the IDL JSON
    ///
    /// Returns the IDL name if found in metadata.
    ///
    /// # Security
    ///
    /// For trusted built-in programs this returns `None` regardless of any
    /// caller-supplied IDL `metadata.name`. The rendered "Program (name: ...)"
    /// label must not be influenced by untrusted metadata when the program
    /// has a canonical identity (see PRS-237).
    pub fn get_idl_name(&self, program_id: &Pubkey) -> Option<String> {
        let program_id_str = program_id.to_string();
        if canonical_name(&program_id_str).is_some() {
            return None;
        }
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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

    /// Regression test for PRS-237.
    ///
    /// A caller-supplied IDL for the System Program with a custom
    /// `program_name` must NOT replace the canonical "System Program" label.
    /// Otherwise an attacker controlling `chain_metadata.solana.idl_mappings`
    /// could mislabel a trusted program (e.g. as "Phantom Wallet") in the
    /// rendered signing payload.
    #[test]
    fn test_prs237_user_name_does_not_override_system_program() {
        let system_program_id = "11111111111111111111111111111111";
        let attacker_idl = r#"{"metadata":{"name":"Phantom Wallet"},"instructions":[]}"#;

        let mut mappings = BTreeMap::new();
        mappings.insert(
            system_program_id.to_string(),
            (attacker_idl.to_string(), "Phantom Wallet".to_string()),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        let system_pk = Pubkey::from_str(system_program_id).unwrap();

        // The canonical name must win; the attacker-supplied name must not be
        // returned.
        assert_eq!(registry.get_program_name(&system_pk), "System Program");

        // The IDL-derived display name must also be suppressed for trusted
        // built-ins so the "Program (name: ...)" rendering cannot leak the
        // attacker-controlled string either.
        assert_eq!(registry.get_idl_name(&system_pk), None);
    }

    /// Regression test for PRS-237.
    ///
    /// Same protection must apply to the 13 programs with built-in IDLs
    /// shipped by `solana_parser`. Even though those programs have their own
    /// canonical IDL, the attack vector (an `idl_mappings` override) still
    /// reaches `get_program_name` via the `unknown_program` preset fallback.
    #[test]
    fn test_prs237_user_name_does_not_override_builtin_idl_program() {
        let jupiter_id = "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB";
        let attacker_idl = r#"{"metadata":{"name":"Phantom Wallet"}}"#;

        let mut mappings = BTreeMap::new();
        mappings.insert(
            jupiter_id.to_string(),
            (attacker_idl.to_string(), "Phantom Wallet".to_string()),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        let jupiter_pk = Pubkey::from_str(jupiter_id).unwrap();

        assert_eq!(registry.get_program_name(&jupiter_pk), "Jupiter Swap");
        assert_eq!(registry.get_idl_name(&jupiter_pk), None);
    }

    /// Regression test for PRS-237 follow-up.
    ///
    /// The canonical-name guard alone is not enough: the IDL *body* drives
    /// instruction decoding (argument names, account names, value formatting)
    /// via the `unknown_program` IDL fallback. The registry must therefore
    /// drop attacker-supplied IDL bodies for trusted built-in programs so a
    /// crafted IDL cannot relabel `lamports`, hide a destination, or fabricate
    /// account names for, say, the System Program.
    #[test]
    fn test_prs237_idl_body_dropped_for_trusted_program() {
        let system_program_id = "11111111111111111111111111111111";
        let jupiter_id = "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB";
        let attacker_idl = r#"{"metadata":{"name":"Phantom Wallet"},"instructions":[]}"#;

        let mut mappings = BTreeMap::new();
        mappings.insert(
            system_program_id.to_string(),
            (attacker_idl.to_string(), "Phantom Wallet".to_string()),
        );
        mappings.insert(
            jupiter_id.to_string(),
            (attacker_idl.to_string(), "Phantom Wallet".to_string()),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();

        // The registry must not surface any caller-supplied IDL body for a
        // trusted program.
        assert!(registry.get_idl(system_program_id).is_none());
        assert!(registry.get_idl(jupiter_id).is_none());
        assert!(registry.get_all_configs().is_empty());

        // `has_idl` for Jupiter must still be true via the upstream built-in
        // IDL path in `solana_parser`, so legitimate decoding keeps working.
        let jupiter_pk = Pubkey::from_str(jupiter_id).unwrap();
        assert!(registry.has_idl(&jupiter_pk));
    }

    /// A caller-supplied name for a program that has NO canonical identity is
    /// still allowed to render through. This preserves the legitimate wallet
    /// use case (label a custom program that the parser has never heard of).
    #[test]
    fn test_user_name_allowed_for_untrusted_program() {
        let custom_id = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
        let custom_idl = r#"{"metadata":{"name":"My Custom Program"},"instructions":[]}"#;

        let mut mappings = BTreeMap::new();
        mappings.insert(
            custom_id.to_string(),
            (custom_idl.to_string(), "My Custom Program".to_string()),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        let custom_pk = Pubkey::from_str(custom_id).unwrap();
        assert_eq!(registry.get_program_name(&custom_pk), "My Custom Program");
        assert_eq!(
            registry.get_idl_name(&custom_pk),
            Some("My Custom Program".to_string())
        );
    }
}
