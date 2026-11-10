//! Test helpers shared across presets: fixture loading, rendered-field lookups and
//! transaction builders.

use crate::core::{InstructionVisualizer, VisualizerContext};
use crate::transaction_string_to_visual_sign;
use base64::Engine;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::compiled_instruction::CompiledInstruction;
use solana_message::{AddressLookupTableAccount, Message, VersionedMessage, v0};
use solana_parser::solana::structs::SolanaAccount;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::Transaction;
use solana_transaction::versioned::VersionedTransaction;
use std::str::FromStr;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayload, SignablePayloadField, SignablePayloadFieldPreviewLayout};

pub fn payload_from_b64(data: &str) -> SignablePayload {
    transaction_string_to_visual_sign(
        data,
        VisualSignOptions {
            include_intermediate_output: false,
            metadata: None,
            decode_transfers: true,
            transaction_name: None,
            developer_config: None,
        },
    )
    .expect("Failed to visualize tx commands")
}

/// Owned wire data for one `VisualizerContext` built from a resolved `Instruction`:
/// program at `account_keys[0]`, accounts after it, first account as the sender.
pub struct InstructionTestContext {
    sender: SolanaAccount,
    compiled: CompiledInstruction,
    account_keys: Vec<Pubkey>,
    registry: crate::idl::IdlRegistry,
}

impl InstructionTestContext {
    pub fn from_instruction(instruction: &Instruction) -> Self {
        let mut account_keys = vec![instruction.program_id];
        account_keys.extend(instruction.accounts.iter().map(|m| m.pubkey));
        let compiled = CompiledInstruction {
            program_id_index: 0,
            accounts: (1..=instruction.accounts.len() as u8).collect(),
            data: instruction.data.clone(),
        };
        let sender = SolanaAccount {
            account_key: instruction
                .accounts
                .first()
                .map(|m| m.pubkey.to_string())
                .unwrap_or_default(),
            signer: true,
            writable: true,
        };
        Self {
            sender,
            compiled,
            account_keys,
            registry: crate::idl::IdlRegistry::new(),
        }
    }

    /// For tests that point an account at an index outside `account_keys` (unresolved ALT entry).
    pub fn compiled_mut(&mut self) -> &mut CompiledInstruction {
        &mut self.compiled
    }

    pub fn context(&self) -> VisualizerContext<'_> {
        VisualizerContext::new(
            &self.sender,
            &self.compiled,
            &self.account_keys,
            &self.registry,
            0,
        )
    }
}

pub fn assert_has_field(payload: &SignablePayload, label: &str) {
    payload
        .fields
        .iter()
        .find(|f| f.label() == label)
        .unwrap_or_else(|| panic!("Should have a {label} field"));
}

#[derive(Debug, serde::Deserialize)]
pub struct TestFixture {
    pub description: String,
    pub signature: String,
    pub source: String,
    pub cluster: String,
    #[serde(default)]
    pub full_transaction_note: Option<String>,
    pub instruction_index: usize,
    pub instruction_data: String,
    pub program_id: String,
    pub accounts: Vec<TestAccount>,
    /// Absent on older fixtures that check fields only.
    #[serde(default)]
    pub expected_title: Option<String>,
    pub expected_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TestAccount {
    pub pubkey: String,
    pub signer: bool,
    pub writable: bool,
    pub description: String,
}

/// Loads `tests/fixtures/<dir>/<name>.json`.
pub fn load_fixture(dir: &str, name: &str) -> TestFixture {
    let fixture_path = format!(
        "{}/tests/fixtures/{dir}/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {fixture_path}: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {fixture_path}: {e}"))
}

pub fn instruction_from_fixture(fixture: &TestFixture) -> Instruction {
    let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
    let accounts = fixture
        .accounts
        .iter()
        .map(|acc| AccountMeta {
            pubkey: Pubkey::from_str(&acc.pubkey).unwrap(),
            is_signer: acc.signer,
            is_writable: acc.writable,
        })
        .collect();
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .expect("fixture instruction_data is base58");
    Instruction {
        program_id,
        accounts,
        data,
    }
}

/// Text rendered for a field, whatever its type: `text_v2` text, `amount_v2`
/// amount, or `address_v2` address.
pub fn rendered_value(field: &SignablePayloadField) -> Option<(String, String)> {
    match field {
        SignablePayloadField::TextV2 { common, text_v2 } => {
            Some((common.label.clone(), text_v2.text.clone()))
        }
        SignablePayloadField::AmountV2 { common, amount_v2 } => {
            Some((common.label.clone(), amount_v2.amount.clone()))
        }
        SignablePayloadField::AddressV2 { common, address_v2 } => {
            Some((common.label.clone(), address_v2.address.clone()))
        }
        _ => None,
    }
}

fn normalized(label: &str) -> String {
    label.to_lowercase().replace(' ', "_")
}

/// Look a label up across the condensed and expanded lists.
pub fn find_value(layout: &SignablePayloadFieldPreviewLayout, key: &str) -> Option<String> {
    layout
        .condensed
        .iter()
        .chain(layout.expanded.iter())
        .flat_map(|list| list.fields.iter())
        .filter_map(|f| rendered_value(&f.signable_payload_field))
        .find(|(label, _)| normalized(label) == normalized(key))
        .map(|(_, value)| value)
}

pub fn condensed_value(layout: &SignablePayloadFieldPreviewLayout, key: &str) -> Option<String> {
    layout
        .condensed
        .iter()
        .flat_map(|list| list.fields.iter())
        .filter_map(|f| rendered_value(&f.signable_payload_field))
        .find(|(label, _)| label == key)
        .map(|(_, value)| value)
}

/// The condensed row with this exact label; panics when it is missing.
pub fn condensed_field<'a>(
    layout: &'a SignablePayloadFieldPreviewLayout,
    label: &str,
) -> &'a SignablePayloadField {
    layout
        .condensed
        .as_ref()
        .expect("condensed view")
        .fields
        .iter()
        .map(|f| &f.signable_payload_field)
        .find(|f| f.label() == label)
        .unwrap_or_else(|| panic!("{label} row"))
}

pub fn visualize(
    visualizer: &dyn InstructionVisualizer,
    instruction: &Instruction,
) -> SignablePayloadFieldPreviewLayout {
    visualize_context(
        visualizer,
        &InstructionTestContext::from_instruction(instruction),
    )
}

/// For tests that edit the compiled instruction (unresolved lookup-table entries).
pub fn visualize_context(
    visualizer: &dyn InstructionVisualizer,
    data: &InstructionTestContext,
) -> SignablePayloadFieldPreviewLayout {
    let field = visualizer
        .visualize_tx_commands(&data.context())
        .expect("visualize failed");
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected a preview_layout");
    };
    preview_layout
}

pub fn title_of(layout: &SignablePayloadFieldPreviewLayout) -> String {
    layout
        .title
        .as_ref()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

/// Renders `tests/fixtures/<dir>/<name>.json` and checks its title and expected fields.
pub fn assert_fixture(visualizer: &dyn InstructionVisualizer, dir: &str, name: &str) {
    let fixture = load_fixture(dir, name);
    let expected_title = fixture
        .expected_title
        .as_deref()
        .unwrap_or_else(|| panic!("{dir}/{name}: fixture has no expected_title"));
    let layout = visualize(visualizer, &instruction_from_fixture(&fixture));
    assert_eq!(
        title_of(&layout),
        expected_title,
        "{} ({}): title",
        fixture.description,
        fixture.signature
    );
    for (key, expected) in &fixture.expected_fields {
        let expected = expected
            .as_str()
            .unwrap_or_else(|| panic!("expected field {key} must be a string"));
        let actual = find_value(&layout, key)
            .unwrap_or_else(|| panic!("{}: field {key} not rendered", fixture.description));
        assert_eq!(actual, expected, "{}: field {key}", fixture.description);
    }
}

pub fn legacy_transaction_b64(instructions: &[Instruction], payer: &Pubkey) -> String {
    let message = Message::new(instructions, Some(payer));
    let tx = Transaction::new_unsigned(message);
    let bytes = bincode::serialize(&tx).expect("serialize transaction");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn v0_transaction_b64(
    instructions: &[Instruction],
    payer: &Pubkey,
    lookup_tables: &[AddressLookupTableAccount],
) -> String {
    let message = v0::Message::try_compile(payer, instructions, lookup_tables, Hash::default())
        .expect("compile v0 message");
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(message),
    };
    let bytes = bincode::serialize(&tx).expect("serialize versioned transaction");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn payload_for(instructions: &[Instruction], payer: &Pubkey) -> SignablePayload {
    payload_from_b64(&legacy_transaction_b64(instructions, payer))
}

pub fn top_level_value(payload: &SignablePayload, label: &str) -> Option<String> {
    payload
        .fields
        .iter()
        .filter_map(rendered_value)
        .find(|(l, _)| l == label)
        .map(|(_, v)| v)
}

pub fn top_level_labels(payload: &SignablePayload) -> Vec<&str> {
    payload.fields.iter().map(|f| f.label().as_str()).collect()
}

pub fn preview_titles(payload: &SignablePayload) -> Vec<String> {
    payload
        .fields
        .iter()
        .filter_map(|f| match f {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                preview_layout.title.as_ref().map(|t| t.text.clone())
            }
            _ => None,
        })
        .collect()
}

/// The default title, no subtitle and none of the rows a transaction summary hoists.
pub fn assert_no_summary(payload: &SignablePayload, default_title: &str) {
    assert_eq!(payload.title, default_title);
    assert_eq!(payload.subtitle, None);
    for label in [
        "From",
        "Program",
        "Amount",
        "Recipient",
        "Escrow",
        "Instruction",
        "Receive",
        "Burn",
        "Pay",
    ] {
        assert!(
            top_level_value(payload, label).is_none(),
            "no hoisted {label} row expected, got layout {:?}",
            top_level_labels(payload)
        );
    }
}
