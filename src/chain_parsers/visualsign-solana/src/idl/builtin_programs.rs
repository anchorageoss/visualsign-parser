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
//!
//! `registered_source` answers a narrower, separate question for simulated
//! (inner/CPI) instructions: which of these sources, if any, do we ourselves
//! vouch for this program ID through. It reports source 2 (`ProgramType`) as
//! `ThirdParty` and a caller-supplied `idl_mappings` entry as
//! `CallerSupplied`, rather than folding either into the trusted set -- see
//! [`crate::intermediate::RegisteredSource`].

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

/// Where `program_id_str` is registered, if at all -- see
/// [`crate::intermediate::RegisteredSource`]. `caller_idl_program_ids` is the
/// set of program IDs the caller supplied a custom IDL for (the keys of
/// `IdlRegistry::get_all_configs()`), used only to detect `CallerSupplied`;
/// pass an empty map if unavailable.
pub fn registered_source(
    program_id_str: &str,
    caller_idl_program_ids: &std::collections::BTreeMap<String, solana_parser::CustomIdlConfig>,
) -> crate::intermediate::RegisteredSource {
    use crate::intermediate::RegisteredSource;

    if NATIVE_PROGRAM_NAMES
        .iter()
        .any(|(id, _)| *id == program_id_str)
    {
        RegisteredSource::Native
    } else if preset_program_ids().contains(program_id_str) {
        RegisteredSource::Preset
    } else if ProgramType::from_program_id(program_id_str).is_some() {
        RegisteredSource::ThirdParty
    } else if caller_idl_program_ids.contains_key(program_id_str) {
        RegisteredSource::CallerSupplied
    } else {
        RegisteredSource::Unregistered
    }
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

/// The IDL-backed in-crate presets' Anchor IDL JSON, keyed by program ID
/// included here so that decode path can use these idls.
fn preset_idl_configs()
-> &'static std::collections::BTreeMap<String, solana_parser::CustomIdlConfig> {
    use solana_parser::{CustomIdl, CustomIdlConfig};

    static PRESET_IDL_CONFIGS: OnceLock<std::collections::BTreeMap<String, CustomIdlConfig>> =
        OnceLock::new();
    PRESET_IDL_CONFIGS.get_or_init(|| {
        const ENTRIES: &[(&str, &str)] = &[
            (
                "DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH",
                include_str!("../presets/dflow_aggregator/dflow_aggregator.json"),
            ),
            (
                "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH",
                include_str!("../presets/drift/drift.json"),
            ),
            (
                "ExponentnaRg3CQbW6dqQNZKXp7gtZ9DGMp1cwC4HAS7",
                include_str!("../presets/exponent_finance/exponent_finance.json"),
            ),
            (
                "jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi",
                include_str!("../presets/jupiter_borrow/jupiter_borrow.json"),
            ),
            (
                "jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9",
                include_str!("../presets/jupiter_earn/jupiter_earn.json"),
            ),
            (
                "PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu",
                include_str!("../presets/jupiter_perps/jupiter_perps.json"),
            ),
            (
                "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                include_str!("../presets/jupiter_swap/jupiter_agg_v6.json"),
            ),
            (
                "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
                include_str!("../presets/kamino_borrow/kamino_borrow.json"),
            ),
            (
                "FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr",
                include_str!("../presets/kamino_farms/kamino_farms.json"),
            ),
            (
                "LiMoM9rMhrdYrfzUCxQppvxCSG1FcrUK9G8uLq4A1GF",
                include_str!("../presets/kamino_limit/kamino_limit.json"),
            ),
            (
                "KvauGMspG5k6rtzrqqn7WNn3oZdyKqLKwK2XWQ8FLjd",
                include_str!("../presets/kamino_vault/kamino_vault.json"),
            ),
            (
                "VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg",
                include_str!("../presets/metadao_conditional_vault/metadao_conditional_vault.json"),
            ),
            (
                "FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq",
                include_str!("../presets/metadao_futarchy/metadao_futarchy.json"),
            ),
            (
                "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG",
                include_str!("../presets/meteora_damm_v2/meteora_damm_v2.json"),
            ),
            (
                "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo",
                include_str!("../presets/meteora_dlmm/meteora_dlmm.json"),
            ),
            (
                "BUNDDh4P5XviMm1f3gCvnq2qKx6TGosAGnoUK12e7cXU",
                include_str!("../presets/neutral_trade/neutral_trade.json"),
            ),
            (
                "onreuGhHHgVzMWSkj2oQDLDtvvGvoepBPkqyaubFcwe",
                include_str!("../presets/onre_app/onre_app.json"),
            ),
            (
                "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc",
                include_str!("../presets/orca_whirlpool/orca_whirlpool.json"),
            ),
            (
                "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
                include_str!("../presets/squads_multisig/squads_multisig_program.json"),
            ),
        ];

        ENTRIES
            .iter()
            .map(|(program_id, json)| {
                (
                    (*program_id).to_string(),
                    CustomIdlConfig {
                        idl: CustomIdl::Json((*json).to_string()),
                        override_builtin: true,
                    },
                )
            })
            .collect()
    })
}

/// Merges the 19 preset IDL configs (see [`preset_idl_configs`]) into
/// `caller_configs`, without overwriting an entry the caller explicitly
/// supplied for the same program ID -- a genuine caller override still wins
/// over our own preset default.
pub fn merge_preset_idl_configs(
    caller_configs: &std::collections::BTreeMap<String, solana_parser::CustomIdlConfig>,
) -> std::collections::BTreeMap<String, solana_parser::CustomIdlConfig> {
    let mut merged = preset_idl_configs().clone();
    for (program_id, config) in caller_configs {
        merged.insert(program_id.clone(), config.clone());
    }
    merged
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
