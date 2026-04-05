//! Structural invariant tests for real production IDLs.
//!
//! These tests assert properties of the decoded IDL structure itself —
//! no random input generation, no proptest. They verify that `decode_idl_data`
//! produces well-formed IDLs from production JSON files.
//!
//! Run: `IDL_FILE=/path/to/idl.json cargo test --test real_idl_validation`
//! All IDLs: `scripts/fuzz_all_idls.sh`

mod common;

use common::load_idl_from_env;

/// Every instruction in the decoded IDL must have a discriminator computed by
/// decode_idl_data (either provided explicitly or derived via Anchor's SHA256
/// scheme). A missing discriminator means the instruction is unreachable.
#[test]
fn real_idl_all_instructions_have_discriminators() {
    let Some((_, idl)) = load_idl_from_env() else {
        return;
    };
    for inst in &idl.instructions {
        let disc = inst
            .discriminator
            .as_ref()
            .unwrap_or_else(|| panic!("instruction '{}' has no discriminator", inst.name));
        assert_eq!(
            disc.len(),
            8,
            "instruction '{}' discriminator must be 8 bytes, got {}",
            inst.name,
            disc.len()
        );
    }
}

/// No two instructions in the IDL may share a discriminator — a collision would
/// make them indistinguishable at parse time.
#[test]
fn real_idl_discriminators_are_unique() {
    let Some((_, idl)) = load_idl_from_env() else {
        return;
    };
    let mut seen: std::collections::HashMap<Vec<u8>, &str> = std::collections::HashMap::new();
    for inst in &idl.instructions {
        if let Some(disc) = &inst.discriminator {
            if let Some(existing) = seen.get(disc) {
                panic!(
                    "discriminator collision between '{}' and '{}': {:?}",
                    existing, inst.name, disc
                );
            }
            seen.insert(disc.clone(), &inst.name);
        }
    }
}

/// No two instructions may share a name — duplicate names make dispatch results
/// ambiguous and hint at an IDL construction error.
#[test]
fn real_idl_instruction_names_are_unique() {
    let Some((_, idl)) = load_idl_from_env() else {
        return;
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for inst in &idl.instructions {
        assert!(
            seen.insert(inst.name.as_str()),
            "duplicate instruction name: '{}'",
            inst.name
        );
    }
}

/// compute_idl_hash must be deterministic — the same JSON must produce the
/// same hash on every call.
#[test]
fn real_idl_idl_hash_is_stable() {
    let Some((json, _)) = load_idl_from_env() else {
        return;
    };
    let h1 = solana_parser::compute_idl_hash(&json);
    let h2 = solana_parser::compute_idl_hash(&json);
    assert_eq!(h1, h2, "IDL hash must be deterministic");
    assert!(!h1.is_empty(), "IDL hash must not be empty");
}
