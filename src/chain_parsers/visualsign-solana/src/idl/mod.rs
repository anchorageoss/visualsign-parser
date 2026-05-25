//! IDL Registry for Solana instruction parsing
//!
//! This module provides utilities for managing Anchor IDLs and integrating them
//! with the solana_parser library for instruction decoding.

use crate::core::available_visualizers;
use solana_parser::{CustomIdl, CustomIdlConfig, Idl, ProgramType, decode_idl_data};
use solana_sdk::pubkey::Pubkey;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use tracing::warn;

/// Well-known native Solana program IDs that ship with built-in decoders.
///
/// These are baseline programs (System, SPL Token, ATA, etc.) that may not have
/// a dedicated preset visualizer in this crate but must still be protected from
/// caller-supplied IDL overrides. They are the most common attack surface for UI
/// spoofing because they appear on practically every Solana transaction.
const NATIVE_BUILTIN_PROGRAM_IDS: &[&str] = &[
    // System program (SOL transfers, account creation)
    "11111111111111111111111111111111",
    // SPL Token program
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    // SPL Token 2022 program
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    // Associated Token Account program
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    // Compute Budget program
    "ComputeBudget111111111111111111111111111111",
    // SPL Memo program v1
    "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo",
    // SPL Memo program v2
    "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
];

/// Returns the complete set of program IDs that must not be overridable by
/// caller-supplied IDLs. The set is the union of:
///
/// 1. `NATIVE_BUILTIN_PROGRAM_IDS` (System, SPL Token, ATA, Memo, ComputeBudget),
/// 2. All program IDs in `solana_parser::ProgramType` (Jupiter, Kamino, Drift,
///    Orca, Raydium, etc.) which carry built-in IDLs upstream,
/// 3. All program IDs registered by built-in preset visualizers in this crate
///    (e.g. `swig_wallet`, `dflow_aggregator`, the Kamino/Meteora suites).
///
/// The set is computed once at first use and cached. Using a `BTreeSet` keeps
/// iteration deterministic for any future logging or diagnostics.
fn protected_program_ids() -> &'static BTreeSet<String> {
    static PROTECTED: OnceLock<BTreeSet<String>> = OnceLock::new();
    PROTECTED.get_or_init(|| {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for id in NATIVE_BUILTIN_PROGRAM_IDS {
            set.insert((*id).to_string());
        }
        // Pull program IDs from every registered preset visualizer. This keeps
        // the protected set in sync as new presets land.
        for visualizer in available_visualizers() {
            if let Some(config) = visualizer.get_config() {
                for program_id in config.data().programs.keys() {
                    set.insert((*program_id).to_string());
                }
            }
        }
        // Also include every IDL that `solana_parser` ships built-in.
        for program_type in ProgramType::all() {
            set.insert(program_type.program_id().to_string());
        }
        set
    })
}

/// Is the given program_id a built-in we refuse to let callers override?
fn is_builtin_program_id(program_id: &str) -> bool {
    protected_program_ids().contains(program_id)
}

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
    /// # Security
    /// Caller-supplied IDLs targeting a built-in program ID (System Program, SPL Token,
    /// Associated Token Account, Compute Budget, Memo, plus every program listed in
    /// `solana_parser::ProgramType` and every program covered by a registered preset)
    /// are silently dropped from the `configs` map. Allowing those overrides would let
    /// a caller relabel native SOL transfers or SPL Token operations and spoof the
    /// signed UI (PRS-223). The user-provided `program_name` is left in `names` so the
    /// separate display-name override path (tracked by PRS-237) stays unaffected.
    ///
    /// # Returns
    /// * `Ok(IdlRegistry)` with non-builtin custom IDLs configured to override
    ///   any non-builtin IDL `solana_parser` may ship
    /// * `Err` if any IDL JSON is invalid
    pub fn from_idl_mappings(
        idl_mappings: BTreeMap<String, (String, String)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut configs = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut idl_names = BTreeMap::new();

        for (program_id, (idl_json, program_name)) in idl_mappings {
            // Refuse to let caller-supplied IDLs override built-in decoders.
            // Drop the IDL but keep `program_name` so display-name behavior is
            // untouched (PRS-237 owns the program_name override question).
            if is_builtin_program_id(&program_id) {
                warn!(
                    program_id = %program_id,
                    "ignoring caller-supplied IDL for built-in program (PRS-223)"
                );
                names.insert(program_id, program_name);
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
    /// Built-in IDLs always win over caller-supplied ones for protected program
    /// IDs (System Program, SPL Token, ATA, Compute Budget, Memo, every
    /// `solana_parser::ProgramType`, and every preset-registered program).
    /// See `from_idl_mappings` for the rationale (PRS-223).
    pub fn has_idl(&self, program_id: &Pubkey) -> bool {
        let program_id_str = program_id.to_string();

        // Built-in IDLs (13 programs from solana_parser) take precedence.
        if ProgramType::from_program_id(&program_id_str).is_some() {
            return true;
        }

        // Defense in depth: ignore caller-supplied IDLs for any built-in program
        // ID even if one somehow ended up in `configs`.
        if is_builtin_program_id(&program_id_str) {
            return false;
        }

        self.configs.contains_key(&program_id_str)
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
    ///
    /// Caller-supplied IDLs for built-in program IDs are never returned (PRS-223).
    /// Built-in IDLs handled by `solana_parser::ProgramType` are not stored here
    /// and are fetched separately by the upstream parser.
    pub fn get_idl(&self, program_id: &str) -> Option<Idl> {
        if is_builtin_program_id(program_id) {
            return None;
        }
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

    /// Sanity check: the protected set covers every program ID listed in the
    /// PRS-223 ticket and matches what we expect to enumerate from the
    /// presets + ProgramType sources.
    #[test]
    fn test_protected_program_ids_contains_expected_builtins() {
        let protected = protected_program_ids();
        // Native programs from NATIVE_BUILTIN_PROGRAM_IDS
        for native in NATIVE_BUILTIN_PROGRAM_IDS {
            assert!(
                protected.contains(*native),
                "protected set missing native program {native}"
            );
        }
        // A program with a built-in IDL via solana_parser::ProgramType
        assert!(protected.contains("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB"));
        // A program that only has a preset visualizer (no ProgramType entry)
        assert!(protected.contains("swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB"));
    }

    /// Caller-supplied IDLs targeting the System Program must be dropped at
    /// registry construction. This is the core PRS-223 invariant.
    #[test]
    fn test_from_idl_mappings_drops_system_program_override() {
        let mut mappings = BTreeMap::new();
        // A bogus IDL keyed to System Program (the attacker payload)
        mappings.insert(
            "11111111111111111111111111111111".to_string(),
            (
                r#"{"instructions":[{"name":"attacker_relabeled_transfer","accounts":[],"args":[]}],"types":[]}"#.to_string(),
                "Definitely Not System".to_string(),
            ),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        let system_pid = Pubkey::from_str("11111111111111111111111111111111").unwrap();

        // The bogus IDL must NOT be retrievable by any of the registry accessors.
        assert!(
            !registry
                .configs
                .contains_key("11111111111111111111111111111111"),
            "configs still holds caller IDL for System Program"
        );
        assert!(
            registry
                .get_idl("11111111111111111111111111111111")
                .is_none(),
            "attacker IDL leaked through get_idl for System Program"
        );

        // `has_idl` returns false: System has no IDL we can hand out (the
        // System preset decodes it directly via bincode), and we refuse to
        // surface the attacker's IDL.
        assert!(
            !registry.has_idl(&system_pid),
            "has_idl must not promise an IDL for System Program when none ships"
        );
    }

    /// Drift is in `solana_parser::ProgramType` (built-in IDL) but has no
    /// preset visualizer in this crate. Pre-fix, attacker IDLs for Drift were
    /// the actually-exploitable path via `unknown_program -> has_idl -> get_idl`.
    #[test]
    fn test_from_idl_mappings_drops_program_type_override() {
        let mut mappings = BTreeMap::new();
        mappings.insert(
            "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH".to_string(),
            (
                r#"{"instructions":[{"name":"fake_drift_ix","accounts":[],"args":[]}],"types":[]}"#
                    .to_string(),
                "Not Drift".to_string(),
            ),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        assert!(
            registry
                .get_idl("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH")
                .is_none(),
            "caller IDL leaked for ProgramType program (Drift)"
        );
    }

    /// Custom IDLs for non-builtin program IDs (typical Anchor programs the
    /// caller actually owns) must still be honored. This is the negative case
    /// that proves the filter is not over-eager.
    #[test]
    fn test_from_idl_mappings_keeps_non_builtin_override() {
        let custom_pid = Pubkey::new_unique();
        let mut mappings = BTreeMap::new();
        mappings.insert(
            custom_pid.to_string(),
            (
                r#"{"instructions":[{"name":"my_custom_ix","accounts":[],"args":[]}],"types":[]}"#
                    .to_string(),
                "My Custom Program".to_string(),
            ),
        );

        let registry = IdlRegistry::from_idl_mappings(mappings).unwrap();
        assert!(registry.has_idl(&custom_pid));
        assert!(registry.get_idl(&custom_pid.to_string()).is_some());
    }
}
