//! The parts of the Associated Token Account program interface this crate uses,
//! inlined from `spl-associated-token-account-client` 2.0.0 (Apache-2.0) so the
//! crate does not depend on the full `spl-associated-token-account` program.

use solana_pubkey::Pubkey;

/// Associated Token Account program id.
pub(crate) const ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Instructions of the Associated Token Account program. The discriminator is
/// the variant index; empty data means `Create`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AssociatedTokenAccountInstruction {
    Create,
    CreateIdempotent,
    RecoverNested,
}

/// The associated token account address of `wallet` for `mint` under
/// `token_program`.
pub(crate) fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    Pubkey::find_program_address(
        &[
            &wallet.to_bytes(),
            &token_program.to_bytes(),
            &mint.to_bytes(),
        ],
        &ID,
    )
    .0
}

/// `CreateIdempotent`, as built by the client crate. Tests only.
#[cfg(test)]
pub(crate) fn create_associated_token_account_idempotent(
    funding: &Pubkey,
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> solana_instruction::Instruction {
    use solana_instruction::AccountMeta;
    solana_instruction::Instruction {
        program_id: ID,
        accounts: vec![
            AccountMeta::new(*funding, true),
            AccountMeta::new(
                get_associated_token_address_with_program_id(wallet, mint, token_program),
                false,
            ),
            AccountMeta::new_readonly(*wallet, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::ID, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: vec![1],
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Expected addresses from `spl-associated-token-account-client` 2.0.0's
    /// `get_associated_token_address_with_program_id`. The Tokenkeg one is
    /// also the writable USDC account of wallet DASPvsk4... in the mainnet
    /// Jupiter Lend deposit fixture (`tests/fixtures/native_programs/
    /// jupiter_earn_deposit_tx.json`, tx 4YNKV2kZ...).
    #[test]
    fn ata_matches_known_addresses() {
        let p = |s: &str| Pubkey::from_str(s).unwrap();
        let wallet = p("DASPvsk4Pt7o3kRKdJGc33Bv43F6Wdx96zsPaqoD3nV1");
        let usdc = p("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(
            get_associated_token_address_with_program_id(
                &wallet,
                &usdc,
                &p("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
            ),
            p("GE8BTcoV5T3SfWmQUB9A3hKhi5RiFmMbmffyux3xWoRL")
        );
        assert_eq!(
            get_associated_token_address_with_program_id(
                &wallet,
                &usdc,
                &p("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")
            ),
            p("2YgVJi8LG9GncDqPVMULSdL6cerpDj25zYFMcPXinRzc")
        );
    }
}
