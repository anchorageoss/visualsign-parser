#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression tests: caller-supplied IDLs must NOT override built-in decoders
//! for native Solana programs (System, SPL Token, ATA, ...) or for programs
//! that ship a built-in IDL via `solana_parser::ProgramType`.
//!
//! Each test submits a malicious IDL keyed to a built-in program ID, parses
//! a real instruction for that program, and asserts that the built-in
//! decoder still wins (no attacker-controlled label appears in the output).

mod common;

use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use solana_system_interface::instruction as system_instruction;
use solana_system_interface::program as system_program;
use std::str::FromStr;
use visualsign::{SignablePayload, SignablePayloadField};
use visualsign_solana::transaction_to_visual_sign;

use common::{instruction_fields, options_with_idl};

/// Recursive walk of a payload looking for any TextV2 field whose `text` or
/// `label` contains `needle`. Used to assert attacker-supplied strings are
/// absent from the rendered output.
fn payload_text_contains(payload: &SignablePayload, needle: &str) -> bool {
    fn walk(field: &SignablePayloadField, needle: &str) -> bool {
        match field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                common.label.contains(needle)
                    || common.fallback_text.contains(needle)
                    || text_v2.text.contains(needle)
            }
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                if common.label.contains(needle) || common.fallback_text.contains(needle) {
                    return true;
                }
                if let Some(t) = &preview_layout.title {
                    if t.text.contains(needle) {
                        return true;
                    }
                }
                if let Some(t) = &preview_layout.subtitle {
                    if t.text.contains(needle) {
                        return true;
                    }
                }
                if let Some(list) = &preview_layout.condensed {
                    for f in &list.fields {
                        if walk(&f.signable_payload_field, needle) {
                            return true;
                        }
                    }
                }
                if let Some(list) = &preview_layout.expanded {
                    for f in &list.fields {
                        if walk(&f.signable_payload_field, needle) {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
    payload.fields.iter().any(|f| walk(f, needle))
}

/// Build a real System Program SOL transfer instruction wrapped in a
/// signable transaction.
fn build_system_transfer_tx(from: Pubkey, to: Pubkey, lamports: u64) -> SolanaTransaction {
    let ix = system_instruction::transfer(&from, &to, lamports);
    SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&from)))
}

/// Core regression: an attacker submits an IDL keyed to the System Program
/// (11111111111111111111111111111111) that relabels the transfer instruction.
/// The wallet must still display the real built-in System
/// "Transfer: N lamports" view, not the attacker's labels.
#[test]
fn caller_idl_does_not_override_system_program_transfer() {
    let from = Pubkey::new_unique();
    let to = Pubkey::new_unique();
    let lamports: u64 = 12_345;

    let tx = build_system_transfer_tx(from, to, lamports);

    // Attacker IDL keyed to System Program. The instruction name and arg names
    // are picked so we can grep for them in the output; if any survive, the
    // override worked and the test fails.
    let attacker_idl = r#"{
        "metadata": {"name": "ATTACKER_IDL_NAME"},
        "instructions": [
            {"name": "ATTACKER_FAKE_TRANSFER", "accounts": [], "args": [
                {"name": "ATTACKER_AMOUNT_FIELD", "type": "u64"}
            ]}
        ],
        "types": []
    }"#;
    let system_pid = Pubkey::from_str("11111111111111111111111111111111").unwrap();
    assert_eq!(system_pid, system_program::id());
    let options = options_with_idl(&system_pid, attacker_idl, "Definitely Not System");

    let payload = transaction_to_visual_sign(tx, options).unwrap();

    // The real System preset must still have rendered the transfer.
    let fields = instruction_fields(&payload);
    let mut found_transfer = false;
    for layout in &fields {
        if let Some(title) = &layout.title {
            if title
                .text
                .contains(&format!("Transfer: {lamports} lamports"))
            {
                found_transfer = true;
            }
        }
    }
    assert!(
        found_transfer,
        "expected built-in System Transfer rendering, got: {:?}",
        fields.iter().map(|l| &l.title).collect::<Vec<_>>()
    );

    // Attacker-controlled strings must NOT appear anywhere in the payload.
    assert!(
        !payload_text_contains(&payload, "ATTACKER_FAKE_TRANSFER"),
        "attacker instruction name leaked into payload"
    );
    assert!(
        !payload_text_contains(&payload, "ATTACKER_AMOUNT_FIELD"),
        "attacker arg name leaked into payload"
    );
    assert!(
        !payload_text_contains(&payload, "ATTACKER_IDL_NAME"),
        "attacker IDL metadata name leaked into payload"
    );
}

/// Also covers programs whose only built-in decoder is the IDL that
/// `solana_parser::ProgramType` ships (Drift, Raydium, etc.). For these,
/// the attacker IDL would flow through `unknown_program -> has_idl ->
/// get_idl` and be used in place of the built-in. The registry now
/// refuses to surface the attacker's IDL even via `get_idl`.
#[test]
fn caller_idl_does_not_override_program_type_idl_for_drift() {
    // Drift program ID, listed in solana_parser::ProgramType::Drift.
    let drift_pid = Pubkey::from_str("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH").unwrap();
    // Single arbitrary-data instruction targeting Drift.
    let payer = Pubkey::new_unique();
    let ix = Instruction {
        program_id: drift_pid,
        accounts: vec![AccountMeta::new(payer, true)],
        data: vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11],
    };
    let tx = SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&payer)));

    let attacker_idl = r#"{
        "metadata": {"name": "DRIFT_ATTACKER_NAME"},
        "instructions": [
            {"name": "DRIFT_FAKE_INSTRUCTION", "accounts": [], "args": []}
        ],
        "types": []
    }"#;
    let options = options_with_idl(&drift_pid, attacker_idl, "Not Drift");

    let payload = transaction_to_visual_sign(tx, options).unwrap();

    // The attacker's labels must NOT show up regardless of which path
    // (built-in IDL, raw fallback, etc.) ends up rendering.
    assert!(
        !payload_text_contains(&payload, "DRIFT_FAKE_INSTRUCTION"),
        "attacker instruction name leaked for Drift"
    );
    assert!(
        !payload_text_contains(&payload, "DRIFT_ATTACKER_NAME"),
        "attacker IDL metadata name leaked for Drift"
    );
}

/// The fix must not regress the legitimate code path: a caller-supplied IDL
/// for a program ID the caller actually owns (not a built-in) is still
/// honored, and the IDL-labeled fields appear in the payload.
#[test]
fn caller_idl_still_honored_for_non_builtin_program() {
    let custom_pid = Pubkey::new_unique();

    let idl_json = serde_json::json!({
        "instructions": [{"name": "my_deposit", "accounts": [], "args": [
            {"name": "amount", "type": "u64"}
        ]}],
        "types": []
    })
    .to_string();

    let idl = solana_parser::decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap().clone();
    let mut data = disc;
    data.extend_from_slice(&7_777u64.to_le_bytes());

    let payer = Pubkey::new_unique();
    let ix = Instruction {
        program_id: custom_pid,
        accounts: vec![AccountMeta::new(payer, true)],
        data,
    };
    let tx = SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&payer)));
    let options = options_with_idl(&custom_pid, &idl_json, "My Program");

    let payload = transaction_to_visual_sign(tx, options).unwrap();
    assert!(
        payload_text_contains(&payload, "my_deposit"),
        "legit caller IDL for non-builtin program was dropped (over-eager filter)"
    );
}
