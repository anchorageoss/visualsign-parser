//! Full-pipeline integration tests for IDL-based instruction visualization.
//!
//! These tests drive the complete stack end-to-end:
//!
//!   SolanaTransaction
//!     → transaction_to_visual_sign          (public API)
//!       → create_idl_registry_from_options  (options → IdlRegistry)
//!       → decode_instructions               (SolanaTransaction × IdlRegistry)
//!         → UnknownProgramVisualizer        (catch-all visualizer)
//!           → try_idl_parsing               (IDL path when registered)
//!             → try_parse_with_idl          (discriminator match + arg decode)
//!       → SignablePayload                   (inspectable output)
//!
//! Contrast with fuzz_idl_parsing.rs, which calls parse_instruction_with_idl
//! directly and never exercises IdlRegistry, the visualizer dispatch, or the
//! SignablePayloadField wrapping.

use std::collections::HashMap;

use generated::parser::{ChainMetadata, Idl as ProtoIdl, SolanaMetadata, chain_metadata};
use proptest::prelude::*;
use solana_parser::arb;
use solana_parser::decode_idl_data;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction as SolanaTransaction;
use visualsign::{
    AnnotatedPayloadField, SignablePayload, SignablePayloadField, SignablePayloadFieldPreviewLayout,
};
use visualsign::vsptrait::VisualSignOptions;
use visualsign_solana::transaction_to_visual_sign;

// ── Transaction builders ──────────────────────────────────────────────────────

fn build_transaction(program_id: Pubkey, extra_accounts: Vec<Pubkey>, data: Vec<u8>) -> SolanaTransaction {
    let fee_payer = Pubkey::new_unique();
    let account_metas: Vec<AccountMeta> = extra_accounts
        .iter()
        .map(|pk| AccountMeta::new_readonly(*pk, false))
        .collect();
    let ix = Instruction::new_with_bytes(program_id, &data, account_metas);
    SolanaTransaction::new_unsigned(Message::new(&[ix], Some(&fee_payer)))
}

fn build_multi_instruction_transaction(pairs: Vec<(Pubkey, Vec<u8>)>) -> SolanaTransaction {
    let fee_payer = Pubkey::new_unique();
    let ixs: Vec<Instruction> = pairs
        .into_iter()
        .map(|(pid, data)| Instruction::new_with_bytes(pid, &data, vec![]))
        .collect();
    SolanaTransaction::new_unsigned(Message::new(&ixs, Some(&fee_payer)))
}

// ── VisualSignOptions builders ────────────────────────────────────────────────

fn options_with_idl(program_id: &Pubkey, idl_json: &str, name: &str) -> VisualSignOptions {
    let mut idl_mappings = HashMap::new();
    idl_mappings.insert(
        program_id.to_string(),
        ProtoIdl {
            value: idl_json.to_string(),
            program_name: Some(name.to_string()),
            idl_type: None,
            idl_version: None,
            signature: None,
        },
    );
    VisualSignOptions {
        metadata: Some(ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Solana(SolanaMetadata {
                idl_mappings,
                network_id: None,
                idl: None,
            })),
        }),
        decode_transfers: false,
        transaction_name: None,
        developer_config: None,
        abi_registry: None,
    }
}

fn options_no_idl() -> VisualSignOptions {
    VisualSignOptions {
        metadata: None,
        decode_transfers: false,
        transaction_name: None,
        developer_config: None,
        abi_registry: None,
    }
}

// ── Field inspection helpers ──────────────────────────────────────────────────

/// Returns the PreviewLayout for every instruction field in the payload.
/// Instruction fields have label "Instruction N"; the Accounts summary uses "Accounts".
fn instruction_fields(payload: &SignablePayload) -> Vec<&SignablePayloadFieldPreviewLayout> {
    payload.fields.iter().filter_map(|f| {
        if let SignablePayloadField::PreviewLayout { common, preview_layout } = f {
            if common.label.starts_with("Instruction") {
                return Some(preview_layout);
            }
        }
        None
    }).collect()
}

/// Searches a flat slice of AnnotatedPayloadFields for a TextV2 field with the given label.
fn find_text(fields: &[AnnotatedPayloadField], label: &str) -> Option<String> {
    fields.iter().find_map(|f| {
        if let SignablePayloadField::TextV2 { common, text_v2 } = &f.signable_payload_field {
            if common.label == label {
                return Some(text_v2.text.clone());
            }
        }
        None
    })
}

// ── Concrete integration tests ────────────────────────────────────────────────

/// Happy path: valid discriminator + correctly serialized args.
/// Verifies the IDL code path is taken and arg values appear in condensed fields.
#[test]
fn pipeline_idl_path_correct_data() {
    let program_id = Pubkey::new_unique();

    let idl_json = serde_json::json!({
        "instructions": [{"name": "deposit", "accounts": [], "args": [
            {"name": "amount", "type": "u64"}
        ]}],
        "types": []
    }).to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let mut data = disc.clone();
    data.extend_from_slice(&42u64.to_le_bytes());

    let payload = transaction_to_visual_sign(
        build_transaction(program_id, vec![], data),
        options_with_idl(&program_id, &idl_json, "My Program"),
    ).unwrap();

    let inst_fields = instruction_fields(&payload);
    assert_eq!(inst_fields.len(), 1);

    let layout = inst_fields[0];
    let title = layout.title.as_ref().unwrap().text.as_str();
    assert!(title.contains("(IDL)"), "expected IDL title, got: {title}");

    let condensed = layout.condensed.as_ref().unwrap();
    assert_eq!(find_text(&condensed.fields, "Instruction"), Some("deposit".into()));
    assert_eq!(find_text(&condensed.fields, "amount"), Some("42".into()));
}

/// IDL is registered but the instruction data has a non-matching discriminator.
/// The IDL path is attempted and gracefully falls back to raw data display.
#[test]
fn pipeline_idl_discriminator_miss() {
    let program_id = Pubkey::new_unique();

    let idl_json = serde_json::json!({
        "instructions": [{"name": "deposit", "accounts": [], "args": []}],
        "types": []
    }).to_string();

    // Discriminator that will never match "deposit"
    let data = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03];

    let payload = transaction_to_visual_sign(
        build_transaction(program_id, vec![], data),
        options_with_idl(&program_id, &idl_json, "My Program"),
    ).unwrap();

    let inst_fields = instruction_fields(&payload);
    let layout = inst_fields[0];

    // IDL was registered so the IDL path is attempted — title still shows "(IDL)"
    let title = layout.title.as_ref().unwrap().text.as_str();
    assert!(title.contains("(IDL)"), "IDL attempted, got: {title}");

    // Expanded fields report the parse failure
    let expanded = layout.expanded.as_ref().unwrap();
    assert_eq!(
        find_text(&expanded.fields, "Status"),
        Some("IDL parsing failed - showing raw data".into()),
    );
}

/// No IDL registered for the program.
/// Falls back to raw hex layout; title is the program ID, no "(IDL)" marker.
#[test]
fn pipeline_no_idl_registered() {
    let program_id = Pubkey::new_unique();

    let payload = transaction_to_visual_sign(
        build_transaction(program_id, vec![], vec![1, 2, 3]),
        options_no_idl(),
    ).unwrap();

    let inst_fields = instruction_fields(&payload);
    let layout = inst_fields[0];

    let title = layout.title.as_ref().unwrap().text.as_str();
    assert!(!title.contains("(IDL)"), "no IDL registered, got: {title}");
    assert_eq!(title, program_id.to_string());
}

/// Named accounts from the IDL appear in the expanded fields with their pubkey values.
#[test]
fn pipeline_named_accounts() {
    let program_id = Pubkey::new_unique();
    let depositor = Pubkey::new_unique();

    let idl_json = serde_json::json!({
        "instructions": [{"name": "deposit",
            "accounts": [{"name": "depositor", "isMut": false, "isSigner": true}],
            "args": []
        }],
        "types": []
    }).to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc = idl.instructions[0].discriminator.as_ref().unwrap();

    let payload = transaction_to_visual_sign(
        build_transaction(program_id, vec![depositor], disc.clone()),
        options_with_idl(&program_id, &idl_json, "Test Program"),
    ).unwrap();

    let inst_fields = instruction_fields(&payload);
    let expanded = inst_fields[0].expanded.as_ref().unwrap();

    assert_eq!(
        find_text(&expanded.fields, "depositor"),
        Some(depositor.to_string()),
    );
}

/// One field is emitted per instruction — the field count invariant holds.
#[test]
fn pipeline_field_count_equals_instruction_count() {
    let program_id = Pubkey::new_unique();

    let tx = build_multi_instruction_transaction(vec![
        (program_id, vec![1]),
        (program_id, vec![2]),
        (program_id, vec![3]),
    ]);

    let payload = transaction_to_visual_sign(tx, options_no_idl()).unwrap();
    assert_eq!(instruction_fields(&payload).len(), 3);
}

/// Two instructions for two different programs: one has an IDL, one does not.
/// Each instruction takes the correct path independently.
#[test]
fn pipeline_multi_instruction_mixed_programs() {
    let program_a = Pubkey::new_unique(); // has IDL registered
    let program_b = Pubkey::new_unique(); // no IDL

    let idl_json = serde_json::json!({
        "instructions": [{"name": "swap", "accounts": [], "args": []}],
        "types": []
    }).to_string();

    let idl = decode_idl_data(&idl_json).unwrap();
    let disc_a = idl.instructions[0].discriminator.as_ref().unwrap().clone();

    let tx = build_multi_instruction_transaction(vec![
        (program_a, disc_a),
        (program_b, vec![0xde, 0xad]),
    ]);

    let payload = transaction_to_visual_sign(tx, options_with_idl(&program_a, &idl_json, "A")).unwrap();

    let inst_fields = instruction_fields(&payload);
    assert_eq!(inst_fields.len(), 2);

    let title_a = inst_fields[0].title.as_ref().unwrap().text.as_str();
    assert!(title_a.contains("(IDL)"), "program_a has IDL, got: {title_a}");

    let title_b = inst_fields[1].title.as_ref().unwrap().text.as_str();
    assert!(!title_b.contains("(IDL)"), "program_b has no IDL, got: {title_b}");
    assert_eq!(title_b, program_b.to_string());
}


// ── Property-based pipeline tests ────────────────────────────────────────────

proptest! {
    // Default 256 cases; override with PROPTEST_CASES=N.
    #![proptest_config(ProptestConfig::default())]

    /// Random IDL registered for a program + instruction data that is either
    /// (a) a valid discriminator prefix + random arg bytes, or (b) fully random
    /// bytes — 50/50 split.  The full pipeline must never panic.
    ///
    /// The valid-discriminator half ensures argument-decoding code is exercised,
    /// not just the discriminator-matching paths.
    #[test]
    fn fuzz_pipeline_never_panics(
        idl_json in arb::arb_idl_json(),
        use_valid_disc in any::<bool>(),
        inst_idx in any::<usize>(),
        data in prop::collection::vec(any::<u8>(), 0..1300usize),
    ) {
        let program_id = Pubkey::new_unique();
        let bytes = if use_valid_disc {
            if let Ok(idl) = decode_idl_data(&idl_json) {
                if !idl.instructions.is_empty() {
                    let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                    if let Some(disc) = &inst.discriminator {
                        let mut d = disc.clone();
                        d.extend_from_slice(&data);
                        d
                    } else { data }
                } else { data }
            } else { data }
        } else {
            data
        };
        let tx = build_transaction(program_id, vec![], bytes);
        let _ = transaction_to_visual_sign(tx, options_with_idl(&program_id, &idl_json, "F"));
    }

    /// The number of instruction fields in the output always equals the number
    /// of instructions in the transaction — regardless of valid/invalid discriminator.
    #[test]
    fn fuzz_pipeline_field_count_invariant(
        idl_json in arb::arb_idl_json(),
        use_valid_disc in any::<bool>(),
        inst_idx in any::<usize>(),
        data in prop::collection::vec(any::<u8>(), 0..1300usize),
    ) {
        let program_id = Pubkey::new_unique();
        let bytes = if use_valid_disc {
            if let Ok(idl) = decode_idl_data(&idl_json) {
                if !idl.instructions.is_empty() {
                    let inst = &idl.instructions[inst_idx % idl.instructions.len()];
                    if let Some(disc) = &inst.discriminator {
                        let mut d = disc.clone();
                        d.extend_from_slice(&data);
                        d
                    } else { data }
                } else { data }
            } else { data }
        } else {
            data
        };
        let tx = build_transaction(program_id, vec![], bytes);
        let inst_count = tx.message.instructions.len();
        let options = options_with_idl(&program_id, &idl_json, "F");
        if let Ok(payload) = transaction_to_visual_sign(tx, options) {
            prop_assert_eq!(instruction_fields(&payload).len(), inst_count);
        }
    }

    /// When instruction data begins with a valid discriminator from the IDL,
    /// the IDL code path is always taken — title contains "(IDL)".
    #[test]
    fn fuzz_pipeline_idl_path_taken_on_valid_discriminator(
        idl_json in arb::arb_idl_json(),
        inst_idx in any::<usize>(),
        arg_bytes in prop::collection::vec(any::<u8>(), 0..200usize),
    ) {
        let Ok(idl) = decode_idl_data(&idl_json) else { return Ok(()); };
        let inst = &idl.instructions[inst_idx % idl.instructions.len()];
        let Some(disc) = &inst.discriminator else { return Ok(()); };

        let mut data = disc.clone();
        data.extend_from_slice(&arg_bytes);

        let program_id = Pubkey::new_unique();
        let tx = build_transaction(program_id, vec![], data);
        let options = options_with_idl(&program_id, &idl_json, "F");

        if let Ok(payload) = transaction_to_visual_sign(tx, options) {
            for layout in instruction_fields(&payload) {
                let title = layout.title.as_ref().unwrap().text.as_str();
                prop_assert!(title.contains("(IDL)"), "expected IDL title, got: {title}");
            }
        }
    }
}
