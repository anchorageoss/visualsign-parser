//! Curated list of well-known Solana program IDs whose displayed name must
//! never be overridden by caller-supplied IDL metadata.
//!
//! Two sources feed this list:
//! 1. The 13 dApp programs with built-in IDLs shipped by `solana_parser`
//!    (queried via `ProgramType::from_program_id`).
//! 2. Native runtime programs and core SPL programs (System, Stake, Vote,
//!    ComputeBudget, AddressLookupTable, BPF Loader variants, SPL Token,
//!    SPL Token-2022, Associated Token Account, Memo v1/v2).
//!
//! For any program ID in either source, callers should treat the canonical
//! name returned here as authoritative. User-supplied `program_name` values
//! from `idl_mappings` must NOT replace a canonical name -- doing so would
//! allow a compromised wallet to mislabel a trusted program in the rendered
//! "Program" field of the signing payload (see PRS-237).

use solana_parser::ProgramType;

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
        "AddressLookupTab1e1111111111111111111111111",
        "Address Lookup Table Program",
    ),
    (
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
        "Associated Token Account Program",
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
];

/// Returns the canonical name for a program ID if it belongs to the curated
/// trusted set (native runtime programs, core SPL programs, or a program with
/// a built-in IDL in `solana_parser`).
///
/// Returns `None` for any program not on the trusted list -- such programs are
/// safe to label with a caller-supplied name, because there is no canonical
/// identity to confuse a signer with.
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
    ProgramType::from_program_id(program_id_str).map(builtin_idl_program_name)
}

/// Returns the canonical `&'static str` name for a program with a built-in
/// IDL in `solana_parser`. Kept in sync with `ProgramType::program_name()`
/// upstream; the duplication is intentional because the upstream signature
/// returns `&str` tied to the receiver's lifetime.
fn builtin_idl_program_name(p: ProgramType) -> &'static str {
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
    }
}
