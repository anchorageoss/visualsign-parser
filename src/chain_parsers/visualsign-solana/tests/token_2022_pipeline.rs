#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Full-pipeline tests for the Token-2022 preset (#574): real mainnet
//! transactions, account indices that do not resolve, and multisig signer
//! labelling, through the public `transaction_*_to_visual_sign` APIs.
//!
//! The preset-level adversarial properties live in
//! `src/presets/token_2022/tests/adversarial_test.rs`.

mod common;

use std::str::FromStr;

use solana_sdk::hash::Hash;
use solana_sdk::instruction::{AccountMeta, CompiledInstruction, Instruction};
use solana_sdk::message::v0::{Message as V0Message, MessageAddressTableLookup};
use solana_sdk::message::{Message, MessageHeader, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::{Transaction as SolanaTransaction, VersionedTransaction};
use visualsign::SignablePayload;
use visualsign::errors::VisualSignError;
use visualsign_solana::{
    transaction_string_to_visual_sign, transaction_to_visual_sign,
    versioned_transaction_to_visual_sign,
};

use common::{find_text, instruction_fields, options_no_idl};

const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

#[derive(serde::Deserialize)]
struct MainnetFixture {
    signature: String,
    full_transaction_base64: String,
    #[serde(default)]
    expected_fields: Option<serde_json::Map<String, serde_json::Value>>,
}

fn load(name: &str) -> MainnetFixture {
    let path = format!(
        "{}/tests/fixtures/token_2022/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn token_2022() -> Pubkey {
    Pubkey::from_str(TOKEN_2022).unwrap()
}

fn titles(payload: &SignablePayload) -> Vec<String> {
    instruction_fields(payload)
        .iter()
        .filter_map(|l| l.title.as_ref().map(|t| t.text.clone()))
        .collect()
}

/// The Token-2022 instruction must not be rendered from an address it could
/// not resolve: the whole decode fails (the default), or, with diagnostics,
/// the instruction is reported as an error and has no layout.
fn assert_not_rendered(result: Result<SignablePayload, VisualSignError>) {
    if let Ok(payload) = result {
        for title in titles(&payload) {
            assert!(
                !title.starts_with("Transfer") && !title.starts_with("Token 2022"),
                "rendered {title}"
            );
        }
    }
}

/// Real mainnet Token-2022 transfers (legacy, v0 without lookups, and with a
/// memo and an ATA create), whole transactions as signed: the transfer is
/// decoded with the amount, decimals and accounts the instruction carries.
#[test]
fn mainnet_transfer_checked_transactions_render() {
    for name in [
        "transfer_checked_mainnet_legacy",
        "transfer_checked_mainnet_v0",
        "transfer_checked_mainnet_memo",
    ] {
        let fixture = load(name);
        let payload =
            transaction_string_to_visual_sign(&fixture.full_transaction_base64, options_no_idl())
                .unwrap_or_else(|e| panic!("{name} ({}): {e}", fixture.signature));
        let layouts = instruction_fields(&payload);
        let transfer = layouts
            .iter()
            .find(|l| {
                l.title
                    .as_ref()
                    .is_some_and(|t| t.text.starts_with("Transfer Checked:"))
            })
            .unwrap_or_else(|| panic!("{name}: no Transfer Checked in {:?}", titles(&payload)));
        let expanded = &transfer.expanded.as_ref().unwrap().fields;
        let expected = fixture.expected_fields.as_ref().unwrap();
        for (label, key) in [
            ("Amount", "amount"),
            ("Source", "source"),
            ("Mint", "mint"),
            ("Destination", "destination"),
            ("Owner/Delegate", "owner/delegate"),
        ] {
            assert_eq!(
                find_text(expanded, label).as_deref(),
                expected[key].as_str(),
                "{name}: {label}"
            );
        }
    }
}

/// The real transaction behind #574's `transfer_checked.json` fixture. On
/// chain its TransferChecked carries a fifth account (the authority again),
/// which the fixture drops. As signed, it renders as unparsed, by design:
/// extra accounts on a checked transfer may be transfer-hook accounts.
#[test]
fn pr_fixture_transaction_as_signed_is_unparsed() {
    let fixture = load("transfer_checked_full_transaction");
    let payload =
        transaction_string_to_visual_sign(&fixture.full_transaction_base64, options_no_idl())
            .unwrap();
    let layouts = instruction_fields(&payload);
    let token = layouts
        .iter()
        .find(|l| {
            l.title
                .as_ref()
                .is_some_and(|t| t.text == "Token 2022 (unparsed)")
        })
        .unwrap_or_else(|| panic!("{:?}", titles(&payload)));
    let reason = find_text(&token.expanded.as_ref().unwrap().fields, "Reason").unwrap();
    assert!(reason.contains("1 extra accounts"), "{reason}");
}

/// A v0 transfer whose destination comes from an address lookup table is
/// not resolvable offline. It fails closed: never shown with another
/// account's address.
#[test]
fn lookup_table_account_fails_closed() {
    let payer = Pubkey::new_unique();
    let (source, mint, authority) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let mut data = vec![12];
    data.extend_from_slice(&1_000_000u64.to_le_bytes());
    data.push(6);
    let message = V0Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 3,
        },
        account_keys: vec![payer, source, token_2022(), mint, authority],
        recent_blockhash: Hash::default(),
        instructions: vec![CompiledInstruction {
            program_id_index: 2,
            // destination = index 5, the first lookup-table address.
            accounts: vec![1, 3, 5, 4],
            data,
        }],
        address_table_lookups: vec![MessageAddressTableLookup {
            account_key: Pubkey::new_unique(),
            writable_indexes: vec![0],
            readonly_indexes: vec![],
        }],
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(message),
    };
    assert_not_rendered(versioned_transaction_to_visual_sign(tx, options_no_idl()));
}

/// A legacy transaction whose Token-2022 instruction names an account index
/// past the end of the account keys fails closed.
#[test]
fn out_of_range_account_index_fails_closed() {
    let payer = Pubkey::new_unique();
    let mut data = vec![3];
    data.extend_from_slice(&5u64.to_le_bytes());
    let message = Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        account_keys: vec![payer, Pubkey::new_unique(), token_2022()],
        recent_blockhash: Hash::default(),
        instructions: vec![CompiledInstruction {
            program_id_index: 2,
            accounts: vec![1, 0, 200],
            data,
        }],
    };
    let tx = SolanaTransaction {
        signatures: vec![Signature::default()],
        message,
    };
    assert_not_rendered(transaction_to_visual_sign(tx, options_no_idl()));
}

/// FINDING: accounts after a Transfer's authority are labelled "Multisig
/// Signer N" even when the transaction does not mark them as signers. The
/// program ignores such accounts (and all of them when the authority is not
/// a multisig), so the label claims a co-signer that is not there.
#[test]
#[ignore = "finding on #574: non-signer extra accounts are labelled 'Multisig Signer N'"]
fn non_signer_extra_account_is_not_labelled_a_signer() {
    let payer = Pubkey::new_unique();
    let (source, destination, extra) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let mut data = vec![3];
    data.extend_from_slice(&5u64.to_le_bytes());
    let ix = Instruction {
        program_id: token_2022(),
        accounts: vec![
            AccountMeta::new(source, false),
            AccountMeta::new(destination, false),
            AccountMeta::new_readonly(payer, true),
            AccountMeta::new_readonly(extra, false),
        ],
        data,
    };
    let tx = SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&payer)));
    let payload = transaction_to_visual_sign(tx, options_no_idl()).unwrap();
    let layouts = instruction_fields(&payload);
    let transfer = layouts
        .iter()
        .find(|l| l.title.as_ref().is_some_and(|t| t.text.starts_with("Transfer:")))
        .unwrap_or_else(|| panic!("{:?}", titles(&payload)));
    let expanded = &transfer.expanded.as_ref().unwrap().fields;
    assert_ne!(
        find_text(expanded, "Multisig Signer 1"),
        Some(extra.to_string()),
        "{extra} is not a signer of this transaction"
    );
}
