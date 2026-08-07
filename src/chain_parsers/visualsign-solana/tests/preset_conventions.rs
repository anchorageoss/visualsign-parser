#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Convention gate for Solana program visualizers.
//!
//! Every Anchor/IDL preset is expected to surface the instruction discriminator
//! as a `create_text_field("Discriminator", ...)` in its expanded view, matching
//! the established Solana program-visualization convention (drift, kamino_*,
//! meteora_*, metadao_*, jupiter_{perps,earn,borrow}, exponent_finance,
//! onre_app, neutral_trade, squads_multisig, swig_wallet, dflow_aggregator, ...).
//!
//! This is a source-level gate: it scans `src/presets/*/mod.rs` so any FUTURE
//! preset is held to the convention automatically. Presets that legitimately do
//! not have an Anchor discriminator (native programs) are allowlisted, as are a
//! small number of grandfathered Anchor presets pending their own tickets.

use std::fs;
use std::path::Path;

/// Presets exempt from the discriminator convention.
const DISCRIMINATOR_ALLOWLIST: &[&str] = &[
    // Native programs: no Anchor 8-byte discriminator (1-byte/native tags).
    "associated_token_account",
    "compute_budget",
    "spl_token",
    "stakepool",
    "system",
    "token_2022",
    // Grandfathered Anchor presets that don't yet surface a Discriminator field.
    // Tracked to be brought into line (see tickets); remove from this list then.
    "jupiter_swap",
    "orca_whirlpool",
];

#[test]
fn all_idl_presets_surface_discriminator() {
    let presets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presets");
    let mut missing: Vec<String> = Vec::new();

    for entry in fs::read_dir(&presets_dir).expect("read presets dir") {
        let entry = entry.expect("dir entry");
        if !entry.file_type().expect("file type").is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if DISCRIMINATOR_ALLOWLIST.contains(&name.as_str()) {
            continue;
        }
        let mod_rs = entry.path().join("mod.rs");
        if !mod_rs.exists() {
            continue;
        }
        let source = fs::read_to_string(&mod_rs).expect("read preset mod.rs");
        if !source.contains("\"Discriminator\"") {
            missing.push(name);
        }
    }

    assert!(
        missing.is_empty(),
        "these Solana presets must surface a \"Discriminator\" field \
         (create_text_field(\"Discriminator\", ...)) per convention, or be added to \
         DISCRIMINATOR_ALLOWLIST with a ticket: {missing:?}"
    );
}

#[test]
fn discriminator_allowlist_entries_still_exist() {
    // Keep the allowlist honest: every entry must name a real preset directory,
    // so stale exemptions don't silently linger after a preset is renamed/removed.
    let presets_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presets");
    for name in DISCRIMINATOR_ALLOWLIST {
        assert!(
            presets_dir.join(name).join("mod.rs").exists(),
            "allowlisted preset '{name}' no longer exists; remove it from DISCRIMINATOR_ALLOWLIST"
        );
    }
}
