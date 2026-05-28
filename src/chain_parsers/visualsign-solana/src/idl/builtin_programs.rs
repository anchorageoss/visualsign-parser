//! Curated set of Solana program IDs whose identity must not be
//! overridable by caller-supplied IDL metadata.
//!
//! Three sources feed this set:
//! 1. Native runtime programs and core SPL programs with a well-known
//!    canonical name (System, Stake, Vote, ComputeBudget, AddressLookupTable,
//!    BPF Loader variants, SPL Token, SPL Token-2022, Associated Token
//!    Account, Memo v1/v2, signature-verify precompiles, Config, Metaplex
//!    Token Metadata, SPL Stake Pool).
//! 2. The 13 dApp programs with built-in IDLs shipped by `solana_parser`
//!    (queried via `ProgramType::from_program_id`).
//! 3. Every program ID registered by an in-crate preset visualizer
//!    (`swig_wallet`, `dflow_aggregator`, the Kamino/Meteora suites, etc.),
//!    enumerated lazily via `available_visualizers()` and cached in a
//!    `OnceLock`. This keeps the trusted set in sync as new presets land
//!    without requiring a parallel hand-maintained list.
//!
//! For any program ID in any of these sources, callers may NOT replace the
//! displayed name or instruction-decoding IDL via `idl_mappings`. Sources
//! 1 and 2 carry a canonical human-readable name (returned by
//! `canonical_name`). Source 3 carries no canonical name (the preset itself
//! drives rendering); these IDs are still "trusted" for the purpose of
//! refusing caller IDL overrides (`is_trusted_program`).

use crate::core::available_visualizers;
use solana_parser::ProgramType;
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Canonical names for native Solana runtime programs and core SPL programs.
///
/// These program IDs always resolve to their canonical name regardless of any
/// user-supplied IDL `program_name`. The list is intentionally kept narrow:
/// only programs whose identity is universal across mainnet and whose
/// mislabeling would be deceptive to a signer.
///
/// Sorted by program ID string for easier auditing.
const NATIVE_PROGRAM_NAMES: &[(&str, &str)] = &[
    // System program. Note the base58 representation is all '1's (32 zero bytes).
    ("11111111111111111111111111111111", "System Program"),
    (
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
        "Associated Token Account Program",
    ),
    (
        "AddressLookupTab1e1111111111111111111111111",
        "Address Lookup Table Program",
    ),
    ("BPFLoader1111111111111111111111111111111111", "BPF Loader"),
    (
        "BPFLoader2111111111111111111111111111111111",
        "BPF Loader 2",
    ),
    (
        "BPFLoaderUpgradeab1e11111111111111111111111",
        "BPF Loader Upgradeable",
    ),
    (
        "ComputeBudget111111111111111111111111111111",
        "Compute Budget Program",
    ),
    (
        "Config1111111111111111111111111111111111111",
        "Config Program",
    ),
    (
        "Ed25519SigVerify111111111111111111111111111",
        "Ed25519 Signature Verify Program",
    ),
    (
        "KeccakSecp256k11111111111111111111111111111",
        "Secp256k1 Signature Verify Program",
    ),
    (
        "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo",
        "Memo Program v1",
    ),
    (
        "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
        "Memo Program",
    ),
    (
        "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
        "SPL Stake Pool Program",
    ),
    (
        "Secp256r1SigVerify1111111111111111111111111",
        "Secp256r1 Signature Verify Program",
    ),
    (
        "Stake11111111111111111111111111111111111111",
        "Stake Program",
    ),
    (
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "SPL Token Program",
    ),
    (
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
        "SPL Token-2022 Program",
    ),
    (
        "Vote111111111111111111111111111111111111111",
        "Vote Program",
    ),
    (
        "hausS13jsjafwWwGqZTUQRmWyvyxn9EQpqMwV1PBBmk",
        "Metaplex Auction House Program",
    ),
    (
        "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
        "Metaplex Token Metadata Program",
    ),
    (
        "namesLPneVptA9Z5rqUDD9tMTWEJwofgaYwp8cawRkX",
        "SPL Name Service Program",
    ),
];

/// Returns the canonical name for a program ID if one is on the curated
/// named list (native runtime programs, core SPL programs, or a program
/// with a built-in IDL in `solana_parser`).
///
/// Returns `None` for any program not on the named list. This includes
/// preset-only programs: a preset such as `swig_wallet` covers its program
/// ID via `is_trusted_program` for the purpose of refusing IDL overrides,
/// but the preset itself drives rendering, so there is no static canonical
/// string to return here.
pub fn canonical_name(program_id_str: &str) -> Option<&'static str> {
    if let Some(name) = NATIVE_PROGRAM_NAMES
        .iter()
        .find(|(id, _)| *id == program_id_str)
        .map(|(_, name)| *name)
    {
        return Some(name);
    }

    // `ProgramType::program_name()` returns a `&str` borrowed from the enum
    // value; the names are static string literals in the upstream crate but
    // the lifetime signature ties them to the receiver. Map each variant to
    // the corresponding `&'static str` literal so callers can use the result
    // without holding onto a `ProgramType` value.
    ProgramType::from_program_id(program_id_str)
        .as_ref()
        .map(builtin_idl_program_name)
}

/// Returns the canonical `&'static str` name for a program with a built-in
/// IDL in `solana_parser`. Kept in sync with `ProgramType::program_name()`
/// upstream; the duplication is intentional because the upstream signature
/// returns `&str` tied to the receiver's lifetime.
///
/// Takes `&ProgramType` because `ProgramType` is not `Copy`; callers iterating
/// over a slice would otherwise need to clone every variant.
fn builtin_idl_program_name(p: &ProgramType) -> &'static str {
    match p {
        ProgramType::ApePro => "Ape Pro",
        ProgramType::CandyMachine => "Metaplex Candy Machine",
        ProgramType::Drift => "Drift Protocol V2",
        ProgramType::JupiterLimit => "Jupiter Limit",
        ProgramType::Jupiter => "Jupiter Swap",
        ProgramType::Kamino => "Kamino",
        ProgramType::Lifinity => "Lifinity Swap V2",
        ProgramType::Meteora => "Meteora",
        ProgramType::Openbook => "Openbook",
        ProgramType::Orca => "Orca Whirlpool",
        ProgramType::Raydium => "Raydium",
        ProgramType::Stabble => "Stabble",
        ProgramType::JupiterAggregatorV6 => "Jupiter Aggregator V6",
    }
}

/// Program IDs registered by in-crate preset visualizers.
///
/// Built once at first use from `available_visualizers()` so new presets
/// stay covered as they land. Each preset declares the program IDs it
/// handles via `SolanaIntegrationConfig::data().programs`. The catch-all
/// `unknown_program` preset has an empty `programs` map and therefore
/// contributes nothing here.
///
/// The set holds `&'static str` because the upstream config keys are
/// `&'static str`; the `Box<dyn InstructionVisualizer>` values returned by
/// `available_visualizers()` are dropped after the set is populated, but
/// the static string references survive.
fn preset_program_ids() -> &'static BTreeSet<&'static str> {
    static PRESET_PROGRAM_IDS: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    PRESET_PROGRAM_IDS.get_or_init(|| {
        let mut set: BTreeSet<&'static str> = BTreeSet::new();
        for visualizer in available_visualizers() {
            if let Some(config) = visualizer.get_config() {
                for program_id in config.data().programs.keys() {
                    set.insert(*program_id);
                }
            }
        }
        set
    })
}

/// Is the program ID one we refuse to let callers override?
///
/// Superset of `canonical_name(...).is_some()` plus every program ID
/// registered by an in-crate preset visualizer (Kamino, Meteora, Drift
/// preset paths, `swig_wallet`, `dflow_aggregator`, etc.). Even when a
/// preset has no canonical string (and thus drives its own rendering),
/// caller IDL bodies for that program ID must still be rejected so a
/// future fallback path that consults the registry cannot be steered by
/// an attacker-controlled IDL body.
pub fn is_trusted_program(program_id_str: &str) -> bool {
    canonical_name(program_id_str).is_some() || preset_program_ids().contains(program_id_str)
}

/// Is the given string a canonical program name reserved for a specific
/// program ID? Used to block display-name impersonation: a caller may not
/// submit an IDL labeled `"System Program"` against an arbitrary pubkey.
///
/// Lookups are case-sensitive and exact-match; near-misses (e.g. trailing
/// whitespace, alternate casing) are intentionally NOT covered here because
/// they don't collide with a canonical label in the rendered output. The
/// canonical-name set is small (~30 entries) so linear scanning is fine.
pub fn is_reserved_canonical_name(name: &str) -> bool {
    if NATIVE_PROGRAM_NAMES.iter().any(|(_, n)| *n == name) {
        return true;
    }
    [
        ProgramType::ApePro,
        ProgramType::CandyMachine,
        ProgramType::Drift,
        ProgramType::JupiterLimit,
        ProgramType::Jupiter,
        ProgramType::Kamino,
        ProgramType::Lifinity,
        ProgramType::Meteora,
        ProgramType::Openbook,
        ProgramType::Orca,
        ProgramType::Raydium,
        ProgramType::Stabble,
        ProgramType::JupiterAggregatorV6,
    ]
    .iter()
    .any(|p| builtin_idl_program_name(p) == name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn system_program_is_trusted() {
        assert_eq!(
            canonical_name("11111111111111111111111111111111"),
            Some("System Program"),
        );
        assert!(is_trusted_program("11111111111111111111111111111111"));
    }

    #[test]
    fn spl_token_is_trusted() {
        assert_eq!(
            canonical_name("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
            Some("SPL Token Program"),
        );
    }

    #[test]
    fn jupiter_builtin_idl_program_is_trusted() {
        // Covered via solana_parser::ProgramType, not via the native list.
        assert_eq!(
            canonical_name("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB"),
            Some("Jupiter Swap"),
        );
    }

    #[test]
    fn random_program_is_not_trusted() {
        // Random base58 program ID that is neither native nor a built-in dApp.
        assert!(canonical_name("9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin").is_none());
        assert!(!is_trusted_program(
            "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"
        ));
    }

    #[test]
    fn metaplex_token_metadata_is_trusted() {
        // High-value impersonation target: owns every Solana NFT's
        // name/URI/creator metadata.
        assert_eq!(
            canonical_name("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s"),
            Some("Metaplex Token Metadata Program"),
        );
    }

    #[test]
    fn spl_stake_pool_is_trusted() {
        assert_eq!(
            canonical_name("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"),
            Some("SPL Stake Pool Program"),
        );
    }

    #[test]
    fn metaplex_auction_house_is_trusted() {
        assert_eq!(
            canonical_name("hausS13jsjafwWwGqZTUQRmWyvyxn9EQpqMwV1PBBmk"),
            Some("Metaplex Auction House Program"),
        );
    }

    #[test]
    fn spl_name_service_is_trusted() {
        assert_eq!(
            canonical_name("namesLPneVptA9Z5rqUDD9tMTWEJwofgaYwp8cawRkX"),
            Some("SPL Name Service Program"),
        );
    }

    #[test]
    fn preset_only_program_is_trusted() {
        // A preset visualizer ID without a corresponding `ProgramType`
        // entry must still be refused as a target for caller IDL overrides.
        // `swig_wallet` is one such preset; pick its program ID from the
        // preset registry rather than hardcoding so this test stays
        // accurate if the preset's covered IDs change.
        let preset_id = preset_program_ids()
            .iter()
            .find(|id| canonical_name(id).is_none())
            .copied()
            .expect("at least one preset-only program ID should exist");
        assert!(is_trusted_program(preset_id));
        // No canonical name for preset-only programs (preset handles rendering).
        assert_eq!(canonical_name(preset_id), None);
    }

    #[test]
    fn unknown_program_preset_contributes_no_ids() {
        // `unknown_program` is the catch-all preset; its `programs` map is
        // empty so it must not pollute the preset-trusted set.
        let preset_ids = preset_program_ids();
        // Sanity: the set should be non-empty (specific presets exist) but
        // shouldn't contain the trivial-empty-marker behavior of catching
        // every base58 string.
        assert!(!preset_ids.is_empty());
        assert!(!preset_ids.contains("9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"));
    }

    #[test]
    fn reserved_names_are_blocked() {
        assert!(is_reserved_canonical_name("System Program"));
        assert!(is_reserved_canonical_name("SPL Token Program"));
        assert!(is_reserved_canonical_name("Jupiter Swap"));
        assert!(is_reserved_canonical_name(
            "Metaplex Token Metadata Program"
        ));
        // Free-form names that don't match a canonical entry must pass.
        assert!(!is_reserved_canonical_name("My Custom Program"));
        assert!(!is_reserved_canonical_name(""));
        // Exact match: a near-miss is NOT blocked because it does not collide
        // with a rendered canonical label.
        assert!(!is_reserved_canonical_name("system program"));
        assert!(!is_reserved_canonical_name("System Program "));
    }
}
