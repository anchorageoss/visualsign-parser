//! Name a transaction's accounts from an instruction's IDL account list.
//!
//! Anchor IDLs can nest a group of accounts inside one instruction entry (a
//! composite `Accounts` struct used as a field); the transaction then lists the
//! group's members inline, in order, rather than one account for the group.
//! Pairing the transaction's accounts with the instruction's *top-level* IDL
//! entries by position silently shifts every name after a group onto the wrong
//! address. The helpers here flatten groups first (members are named
//! `group.member`, nested groups `a.b.c`), mirroring upstream
//! `solana_parser::solana::idl_parser::create_accounts_map`, so presets can name
//! accounts correctly by position without depending on that function's
//! unordered-HashMap/error-on-shortfall behavior.
//!
//! Presets must use [`name_idl_accounts`] rather than hand-rolled positional
//! pairing; a crate-level parity test pins this module's naming against
//! `create_accounts_map` for every preset IDL.

use std::collections::BTreeMap;

use solana_parser::solana::structs::{IdlAccount, IdlInstruction};

/// Flatten an instruction's IDL account list to leaf names in transaction
/// order.
///
/// A plain account yields its own name. A composite group contributes no
/// entry of its own: its members are appended in order, each prefixed with
/// `"{group}."` (nested groups nest their prefixes, `a.b.c`). The returned
/// list is what the transaction's account list pairs with by position.
pub(crate) fn flattened_account_names(instruction: &IdlInstruction) -> Vec<String> {
    let mut names = Vec::new();
    flatten_leaf_names(&instruction.accounts, "", &mut names);
    names
}

/// Named accounts of an instruction, split by whether the IDL names them.
///
/// `named` is a `BTreeMap` for deterministic iteration order (rendered output
/// feeds the payload hash). `remaining` holds the transaction's accounts past
/// the IDL's flattened list, in order, for presets to render as
/// [`remaining_account_label`] fields.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct NamedIdlAccounts {
    pub named: BTreeMap<String, String>,
    pub remaining: Vec<String>,
}

/// Name an instruction's accounts by position against the flattened IDL list.
///
/// `accounts` are the `InstructionView::accounts` display strings, so
/// `unresolved(N)` lookup-table placeholders pass through unchanged.
///
/// Accounts past the flattened IDL list are kept, in order, as
/// [`NamedIdlAccounts::remaining`]. Fewer accounts than names is tolerated:
/// only the available ones are named (no panic, no error) -- a malformed
/// instruction must not abort the render path.
pub(crate) fn name_idl_accounts(
    instruction: &IdlInstruction,
    accounts: &[String],
) -> NamedIdlAccounts {
    let names = flattened_account_names(instruction);
    let mut named = BTreeMap::new();
    for (name, account) in names.iter().zip(accounts.iter()) {
        named.insert(name.clone(), account.clone());
    }
    let remaining = accounts.iter().skip(names.len()).cloned().collect();
    NamedIdlAccounts { named, remaining }
}

/// Human-readable label for an account the IDL does not name.
///
/// Takes the account's 0-based position in the remaining list and renders it
/// 1-based, matching the label the dflow_aggregator preset has always used
/// for its extra accounts; presets rendering
/// [`NamedIdlAccounts::remaining`] use this so the label stays identical
/// everywhere.
pub(crate) fn remaining_account_label(position: usize) -> String {
    format!("Remaining Account {}", position + 1)
}

/// Collect leaf account names in transaction order, prefixing members of a
/// group with the group's name so they stay distinct.
fn flatten_leaf_names(specs: &[IdlAccount], prefix: &str, out: &mut Vec<String>) {
    for spec in specs {
        let name = format!("{prefix}{}", spec.name);
        if spec.accounts.is_empty() {
            out.push(name);
        } else {
            flatten_leaf_names(&spec.accounts, &format!("{name}."), out);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_parser::decode_idl_data;
    use solana_parser::solana::idl_parser::create_accounts_map;
    use solana_parser::solana::structs::{AccountAddress, IdlAccount, SolanaAccount};

    fn account(name: &str) -> IdlAccount {
        IdlAccount {
            name: name.to_string(),
            is_mut: false,
            is_signer: false,
            is_optional: false,
            accounts: Vec::new(),
        }
    }

    fn group(name: &str, members: &[IdlAccount]) -> IdlAccount {
        IdlAccount {
            accounts: members.to_vec(),
            ..account(name)
        }
    }

    fn instruction(accounts: Vec<IdlAccount>) -> IdlInstruction {
        IdlInstruction {
            name: "test_ix".to_string(),
            discriminator: Some(vec![0u8; 8]),
            accounts,
            args: Vec::new(),
        }
    }

    fn addrs(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("addr{i}")).collect()
    }

    #[test]
    fn plain_accounts_keep_their_names() {
        let ix = instruction(vec![account("payer"), account("mint"), account("vault")]);
        assert_eq!(flattened_account_names(&ix), vec!["payer", "mint", "vault"]);

        let named = name_idl_accounts(&ix, &addrs(3));
        assert_eq!(named.named.get("payer"), Some(&"addr0".to_string()));
        assert_eq!(named.named.get("mint"), Some(&"addr1".to_string()));
        assert_eq!(named.named.get("vault"), Some(&"addr2".to_string()));
        assert!(named.remaining.is_empty());
    }

    #[test]
    fn group_members_are_prefixed_and_kept_in_order() {
        // A group in the middle: every top-level name after it used to pair
        // with the wrong address when presets zipped top-level entries.
        let ix = instruction(vec![
            account("authority"),
            group(
                "base",
                &[account("owner"), account("obligation"), account("reserve")],
            ),
            account("farmsProgram"),
        ]);

        assert_eq!(
            flattened_account_names(&ix),
            vec![
                "authority",
                "base.owner",
                "base.obligation",
                "base.reserve",
                "farmsProgram",
            ]
        );

        let named = name_idl_accounts(&ix, &addrs(5));
        assert_eq!(named.named.get("base.owner"), Some(&"addr1".to_string()));
        assert_eq!(named.named.get("farmsProgram"), Some(&"addr4".to_string()));
    }

    #[test]
    fn nested_groups_stack_prefixes() {
        let ix = instruction(vec![
            group(
                "outer",
                &[
                    account("first"),
                    group("inner", &[account("leaf_a"), account("leaf_b")]),
                ],
            ),
            account("trailing"),
        ]);

        assert_eq!(
            flattened_account_names(&ix),
            vec![
                "outer.first",
                "outer.inner.leaf_a",
                "outer.inner.leaf_b",
                "trailing",
            ]
        );

        let named = name_idl_accounts(&ix, &addrs(4));
        assert_eq!(
            named.named.get("outer.inner.leaf_a"),
            Some(&"addr1".to_string())
        );
        assert_eq!(named.named.get("trailing"), Some(&"addr3".to_string()));
    }

    #[test]
    fn fewer_accounts_than_names_names_only_the_available_ones() {
        let ix = instruction(vec![
            account("payer"),
            group("grp", &[account("a"), account("b")]),
            account("system"),
        ]);
        let named = name_idl_accounts(&ix, &addrs(2));
        assert_eq!(named.named.len(), 2);
        assert_eq!(named.named.get("payer"), Some(&"addr0".to_string()));
        assert_eq!(named.named.get("grp.a"), Some(&"addr1".to_string()));
        assert!(!named.named.contains_key("grp.b"));
        assert!(named.remaining.is_empty());
    }

    #[test]
    fn more_accounts_than_names_keeps_remaining_in_order() {
        let ix = instruction(vec![account("payer"), group("grp", &[account("a")])]);
        let named = name_idl_accounts(&ix, &addrs(4));
        assert_eq!(named.named.len(), 2);
        assert_eq!(
            named.remaining,
            vec!["addr2".to_string(), "addr3".to_string()]
        );
    }

    #[test]
    fn unresolved_placeholders_pass_through_unchanged() {
        let ix = instruction(vec![account("payer"), account("vault")]);
        let accounts = vec!["SomeKey1111".to_string(), "unresolved(7)".to_string()];
        let named = name_idl_accounts(&ix, &accounts);
        assert_eq!(named.named.get("vault"), Some(&"unresolved(7)".to_string()));
    }

    #[test]
    fn remaining_account_label_is_one_based() {
        assert_eq!(remaining_account_label(0), "Remaining Account 1");
        assert_eq!(remaining_account_label(2), "Remaining Account 3");
    }

    /// Parity with upstream: for every instruction of every preset IDL, the
    /// names this module produces must equal `create_accounts_map`'s. That
    /// function is the reference naming for the decoded-IDL path in
    /// solana_parser; if the two ever drift (here or upstream), a preset's
    /// labels would disagree with the generic IDL renderer for the same
    /// instruction, so any drift fails this test.
    #[test]
    fn preset_idl_naming_matches_upstream_create_accounts_map() {
        assert!(
            !crate::PRESET_IDLS.is_empty(),
            "build.rs should discover at least one preset IDL"
        );

        for (preset_name, idl_json) in crate::PRESET_IDLS {
            let idl = decode_idl_data(idl_json)
                .unwrap_or_else(|e| panic!("preset {preset_name}: IDL should decode: {e}"));
            for ix in &idl.instructions {
                let names = flattened_account_names(ix);
                let accounts = addrs(names.len());
                let expected: BTreeMap<String, String> = create_accounts_map(
                    &accounts
                        .iter()
                        .map(|key| {
                            AccountAddress::Static(SolanaAccount {
                                account_key: key.clone(),
                                signer: false,
                                writable: false,
                            })
                        })
                        .collect::<Vec<_>>(),
                    ix,
                )
                .unwrap_or_else(|e| panic!("preset {preset_name}, instruction {}: {e}", ix.name))
                .into_iter()
                .collect();

                let actual = name_idl_accounts(ix, &accounts).named;
                assert_eq!(
                    actual, expected,
                    "preset {preset_name}, instruction {}: preset naming \
                     must match upstream create_accounts_map",
                    ix.name
                );
            }
        }
    }
}
