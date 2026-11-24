//! Receipt-token test helpers: instructions assembled from a profile's IDL, and the check
//! that a profile names only arguments and accounts its IDL has.

use super::{Flow, ReceiptTokenProgram, RecipientRule, Seed};
use solana_instruction::{AccountMeta, Instruction};
use solana_parser::solana::structs::IdlType;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// `name` from the program's IDL: discriminator plus the `u64` args, over one account per
/// IDL account from `accounts` (name to key, first match wins) or a fresh key.
pub(crate) fn synthetic_instruction(
    program: &ReceiptTokenProgram,
    name: &str,
    args: &[u64],
    accounts: &[(&str, &str)],
) -> Instruction {
    let idl = program.idl().unwrap();
    let idl_instruction = idl
        .instructions
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("IDL has no instruction {name}"));
    let mut data = idl_instruction.discriminator.clone().unwrap();
    for arg in args {
        data.extend_from_slice(&arg.to_le_bytes());
    }
    let metas = idl_instruction
        .accounts
        .iter()
        .map(|account| {
            let pubkey = accounts
                .iter()
                .find(|(n, _)| *n == account.name)
                .map(|(_, key)| Pubkey::from_str(key).unwrap())
                .unwrap_or_else(Pubkey::new_unique);
            AccountMeta {
                pubkey,
                is_signer: account.is_signer,
                is_writable: account.is_mut,
            }
        })
        .collect();
    Instruction {
        program_id: Pubkey::from_str(program.program_id).unwrap(),
        accounts: metas,
        data,
    }
}

/// Every argument and account a profile names must exist in its IDL in the shape the
/// renderer reads; a mismatch would otherwise degrade silently to the generic view.
pub(crate) fn assert_profile_matches_idl(program: &ReceiptTokenProgram) {
    let idl = program
        .idl()
        .unwrap_or_else(|| panic!("{} IDL should load", program.display_name));
    for action in program.actions {
        let context = format!("{} {}", program.display_name, action.instruction);
        let instruction = idl
            .instructions
            .iter()
            .find(|i| i.name == action.instruction)
            .unwrap_or_else(|| panic!("{context}: not in the IDL"));
        let has_account = |name: &str| instruction.accounts.iter().any(|a| a.name == name);
        let has_u64_arg = |name: &str| {
            instruction
                .args
                .iter()
                .any(|a| a.name == name && a.r#type == IdlType::U64)
        };

        for name in [
            program.signer,
            action.asset_mint_name(program),
            program.receipt_mint,
            program.token_program,
        ] {
            assert!(has_account(name), "{context}: no account {name}");
        }
        for (_, name) in action.account_rows {
            assert!(has_account(name), "{context}: no account {name}");
        }
        match &action.recipient {
            RecipientRule::SignerTokenAccount(name) => {
                assert!(has_account(name), "{context}: no account {name}");
                assert!(
                    action.flow.received().is_some(),
                    "{context}: a request escrows; use RecipientRule::SignerEscrow"
                );
            }
            RecipientRule::SignerEscrow {
                account,
                request,
                seeds,
                ..
            } => {
                assert!(has_account(account), "{context}: no account {account}");
                assert!(has_account(request), "{context}: no account {request}");
                for seed in *seeds {
                    if let Seed::Account(name) = seed {
                        assert!(has_account(name), "{context}: no seed account {name}");
                    }
                }
                assert!(
                    seeds
                        .iter()
                        .any(|seed| matches!(seed, Seed::Account(name) if *name == program.signer)),
                    "{context}: escrow seeds must include Seed::Account({:?})",
                    program.signer
                );
            }
        }

        assert!(
            has_u64_arg(action.amount_arg),
            "{context}: no u64 arg {}",
            action.amount_arg
        );
        if let Some(bound) = action.bound {
            assert!(has_u64_arg(bound), "{context}: no u64 arg {bound}");
            assert!(
                action.flow.counter_leg().is_some(),
                "{context}: {:?} has no counter leg, so a bound never renders",
                action.flow
            );
        }
        if action.full_position_title.is_some() {
            assert!(
                action.flow == Flow::Withdraw && action.bound.is_none(),
                "{context}: a full-position title needs a plain withdraw"
            );
        }
    }
}
