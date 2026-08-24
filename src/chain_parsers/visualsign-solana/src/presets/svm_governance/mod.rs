//! SVM Governance preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::SvmGovernanceConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const SVM_GOVERNANCE_PROGRAM_ID: &str = "govYkyQ3ePtGULAtY6V75qjWE8UH4vCUVQ1W4HdCAZU";

const SVM_GOVERNANCE_DISPLAY_NAME: &str = "SVM Governance";

const SVM_GOVERNANCE_IDL_JSON: &str = include_str!("svm_governance.json");

static SVM_GOVERNANCE_CONFIG: SvmGovernanceConfig = SvmGovernanceConfig;

pub struct SvmGovernanceVisualizer;

impl InstructionVisualizer for SvmGovernanceVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text = format!(
            "Program ID: {}\nData: {instruction_data_hex}",
            view.program_id
        );

        let parsed = parse_svm_governance_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                let index = context.instruction_index();
                tracing::warn!(
                    "Failed to parse SVM Governance instruction {index} with IDL: {e}"
                );
                build_fallback_fields(&view.program_id)?
            }
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        expanded_fields.push(create_raw_data_field(data, Some(instruction_data_hex))?);
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        let index = context.instruction_index() + 1;
        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {index}"),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SVM_GOVERNANCE_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Governance(SVM_GOVERNANCE_DISPLAY_NAME)
    }
}

fn get_svm_governance_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(SVM_GOVERNANCE_IDL_JSON).ok())
        .as_ref()
}

fn parse_svm_governance_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<SvmGovernanceParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_svm_governance_idl().ok_or("SVM Governance IDL not available")?;
    let parsed = parse_instruction_with_idl(data, SVM_GOVERNANCE_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(SvmGovernanceParsedInstruction {
        parsed,
        named_accounts,
        extra_accounts,
    })
}

fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[String],
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut named_accounts = BTreeMap::new();
    let mut extra_accounts = Vec::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_str) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_str.clone());
            } else {
                extra_accounts.push(account_str.clone());
            }
        }
    }

    (named_accounts, extra_accounts)
}

struct SvmGovernanceParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &SvmGovernanceParsedInstruction,
    program_id: &str,
) -> Result<
    (
        String,
        Vec<AnnotatedPayloadField>,
        Vec<AnnotatedPayloadField>,
    ),
    VisualSignError,
> {
    let parsed = &instruction.parsed;
    let instruction_name = &parsed.instruction_name;
    let title = format!("{SVM_GOVERNANCE_DISPLAY_NAME}: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", SVM_GOVERNANCE_DISPLAY_NAME)?);
    condensed_fields.push(create_text_field("Instruction", instruction_name)?);
    for (key, value) in &parsed.program_call_args {
        condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Instruction", instruction_name)?);
    expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (index, pubkey) in instruction.extra_accounts.iter().enumerate() {
        expanded_fields.push(create_text_field(
            &format!("Remaining Account {}", index + 1),
            pubkey,
        )?);
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    Ok((title, condensed_fields, expanded_fields))
}

fn build_fallback_fields(
    program_id: &str,
) -> Result<
    (
        String,
        Vec<AnnotatedPayloadField>,
        Vec<AnnotatedPayloadField>,
    ),
    VisualSignError,
> {
    let title = format!("{SVM_GOVERNANCE_DISPLAY_NAME}: Unknown Instruction");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", SVM_GOVERNANCE_DISPLAY_NAME)?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

    Ok((title, condensed_fields, expanded_fields))
}

/// Render a single program-call argument as one field value.
///
/// Each top-level argument becomes exactly ONE field. Objects and arrays render
/// as a compact, JSON-like string WITHOUT the `"` quotes of real JSON. Two
/// reasons, both of which this IDL exercises:
///
/// 1. **No field explosion.** `cast_vote_override.stake_merkle_proof` is a
///    `Vec<[u8; 32]>`. Recursing into per-element fields would bury the vote
///    weights (`for_votes_bp`, `against_votes_bp`, `abstain_votes_bp`) under
///    hundreds of per-byte entries.
/// 2. **Charset safety.** `create_proposal` takes attacker-controlled `title`
///    and `description` strings straight from the transaction. Non-ASCII and
///    control bytes there would fail `SignablePayload::validate_charset`, so
///    every leaf string passes through `charset_safe`.
///
/// Arrays whose elements are all byte-sized integers (0..=255) render as a
/// single `0x`-prefixed hex string. A `Vec<[u8; 32]>` therefore renders as a
/// bracketed list of 32-byte hex strings, one per Merkle proof node.
fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => charset_safe(s),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(items) => {
            if let Some(hex) = bytes_as_hex(items) {
                hex
            } else {
                let inner: Vec<String> = items.iter().map(format_arg_value).collect();
                format!("[{}]", inner.join(","))
            }
        }
        serde_json::Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", charset_safe(k), format_arg_value(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// Keep printable ASCII and spaces; drop everything else. `"` and `\` go
/// because a leaf string carrying them would reintroduce the JSON quoting that
/// `format_arg_value` renders without. Tabs, carriage returns, other control
/// bytes, and non-ASCII go because `SignablePayload::validate_charset` rejects
/// them.
///
/// Load-bearing for this program, not defensive: `create_proposal` carries a
/// free-form `title` and `description` supplied by the proposer.
fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
}

/// If every element is an integer in `0..=255`, render the array as a single
/// `0x`-prefixed hex string. Returns `None` for empty or non-byte arrays so the
/// caller falls back to a bracketed list.
fn bytes_as_hex(items: &[serde_json::Value]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(items.len());
    for item in items {
        let byte = item.as_u64().filter(|n| *n <= u8::MAX as u64)? as u8;
        bytes.push(byte);
    }
    Some(format!("0x{}", hex::encode(bytes)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use crate::{SolanaTransactionWrapper, SolanaVisualSignConverter};
    use serde_json::json;
    use solana_parser::solana::structs::SolanaAccount;
    use solana_sdk::instruction::{AccountMeta, CompiledInstruction, Instruction};
    use solana_sdk::message::Message;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::transaction::Transaction;
    use std::str::FromStr;
    use visualsign::vsptrait::{
        Transaction as _, VisualSignConverter as _, VisualSignOptions,
    };

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

    /// Look the discriminator up from the bundled IDL instead of hard-coding
    /// the bytes, so the tests stay correct across IDL regenerations.
    fn discriminator_for(instruction_name: &str) -> Vec<u8> {
        let idl = get_svm_governance_idl().unwrap();
        idl.instructions
            .iter()
            .find(|ix| ix.name == instruction_name)
            .unwrap_or_else(|| panic!("{instruction_name} exists in the bundled IDL"))
            .discriminator
            .as_ref()
            .expect("instruction has a computed discriminator")
            .clone()
    }

    fn borsh_string(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn test_svm_governance_idl_loads() {
        let idl = get_svm_governance_idl();
        assert!(idl.is_some(), "SVM Governance IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_svm_governance_idl_has_discriminators() {
        let idl = get_svm_governance_idl().unwrap();
        for instruction in &idl.instructions {
            assert!(
                instruction.discriminator.is_some(),
                "Instruction '{}' should have a computed discriminator",
                instruction.name
            );
            let disc = instruction.discriminator.as_ref().unwrap();
            assert_eq!(
                disc.len(),
                8,
                "Discriminator for '{}' should be 8 bytes",
                instruction.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage_data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let accounts = vec![];
        let result = parse_svm_governance_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_svm_governance_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }

    #[test]
    fn test_cast_vote_renders_vote_weights_and_named_accounts() {
        // cast_vote takes three u64 basis-point weights. Its IDL account list is
        // signer, proposal, vote, ... -- so a caller-supplied account list maps
        // positionally onto those names.
        let mut data = discriminator_for("cast_vote");
        data.extend_from_slice(&6_000u64.to_le_bytes());
        data.extend_from_slice(&3_000u64.to_le_bytes());
        data.extend_from_slice(&1_000u64.to_le_bytes());

        let pubkeys: Vec<String> = (0..3).map(|_| Pubkey::new_unique().to_string()).collect();
        let parsed = parse_svm_governance_instruction(&data, &pubkeys)
            .expect("cast_vote should decode against the bundled IDL");

        assert_eq!(parsed.parsed.instruction_name, "cast_vote");
        assert_eq!(
            parsed.named_accounts.get("signer"),
            Some(&pubkeys[0]),
            "first account should be named 'signer'"
        );
        assert_eq!(
            parsed.named_accounts.get("proposal"),
            Some(&pubkeys[1]),
            "second account should be named 'proposal'"
        );

        let (title, condensed, _expanded) =
            build_parsed_fields(&parsed, SVM_GOVERNANCE_PROGRAM_ID).unwrap();
        assert_eq!(title, "SVM Governance: cast_vote");

        let entries: Vec<(String, String)> = condensed.iter().map(field_label_value).collect();
        for (label, expected) in [
            ("for_votes_bp", "6000"),
            ("against_votes_bp", "3000"),
            ("abstain_votes_bp", "1000"),
        ] {
            assert!(
                entries
                    .iter()
                    .any(|(l, v)| l == label && v == expected),
                "condensed view should show {label}={expected}, got: {entries:?}"
            );
        }
    }

    #[test]
    fn test_create_proposal_strings_are_charset_safe() {
        // title and description come straight from the proposer, so they are the
        // one place in this program where a transaction can carry arbitrary
        // UTF-8 into a field value. validate_charset forbids non-ASCII, so
        // charset_safe must strip it before the field is built.
        let mut data = discriminator_for("create_proposal");
        data.extend_from_slice(&7u64.to_le_bytes());
        data.extend_from_slice(&borsh_string("Raise the \"fee\" cap \u{2192} 5%\u{1f600}"));
        data.extend_from_slice(&borsh_string("Body\twith\ncontrol bytes"));

        let parsed = parse_svm_governance_instruction(&data, &[])
            .expect("create_proposal should decode against the bundled IDL");

        let (_title, _condensed, expanded) =
            build_parsed_fields(&parsed, SVM_GOVERNANCE_PROGRAM_ID).unwrap();
        let entries: Vec<(String, String)> = expanded
            .iter()
            .map(field_label_value)
            .filter(|(label, _)| label == "title" || label == "description")
            .collect();
        assert_eq!(entries.len(), 2, "both strings should render as fields");

        for (label, value) in &entries {
            assert!(
                value.is_ascii()
                    && !value.contains('"')
                    && !value.contains('\\')
                    && !value.chars().any(|c| c.is_ascii_control()),
                "{label} must be charset-safe, got: {value}"
            );
        }
        let title_value = entries
            .iter()
            .find(|(label, _)| label == "title")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert!(
            title_value.contains("Raise the fee cap") && title_value.contains("5%"),
            "printable ASCII should survive sanitization, got: {title_value}"
        );
    }

    #[test]
    fn test_merkle_proof_renders_as_hex_list_not_per_byte_fields() {
        // stake_merkle_proof is a Vec<[u8; 32]>. Each node renders as one 0x-hex
        // string and the whole proof stays inside a single field, so the vote
        // weights next to it remain legible.
        let node_a: Vec<u8> = (0u8..32).collect();
        let node_b: Vec<u8> = (32u8..64).collect();
        let rendered = format_arg_value(&json!([node_a, node_b]));

        assert_eq!(
            rendered,
            "[0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f,\
             0x202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f]"
        );
        assert!(
            !rendered.contains("[0,1,2"),
            "proof nodes must not render as per-byte number lists: {rendered}"
        );
    }

    #[test]
    fn test_format_arg_value_renders_scalars_and_objects_quote_free() {
        assert_eq!(format_arg_value(&json!(42)), "42");
        assert_eq!(format_arg_value(&json!(true)), "true");
        assert_eq!(format_arg_value(&serde_json::Value::Null), "null");
        // StakeMerkleLeaf is a struct: one field, rendered quote-free.
        let leaf = format_arg_value(&json!({"active_stake": 100, "stake_account": "Stake111"}));
        assert!(
            !leaf.contains('"') && !leaf.contains('\\'),
            "struct args must render quote-free: {leaf}"
        );
        assert!(
            leaf.contains("active_stake:100") && leaf.contains("stake_account:Stake111"),
            "struct fields should stay legible inline: {leaf}"
        );
    }

    /// End-to-end proof of the two things the unit tests above cannot reach:
    /// that `build.rs` registration actually routes this program to this preset
    /// instead of the `unknown_program` catch-all, and that a hostile proposal
    /// title survives `validate_charset` on the real payload.
    #[test]
    fn test_end_to_end_transaction_routes_here_and_passes_charset() {
        let payer = Pubkey::new_unique();
        let mut data = discriminator_for("create_proposal");
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&borsh_string("Fee cap \u{2192} 5% \u{1f600}"));
        data.extend_from_slice(&borsh_string("Body"));

        let instruction = Instruction {
            program_id: Pubkey::from_str(SVM_GOVERNANCE_PROGRAM_ID).unwrap(),
            accounts: vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new(Pubkey::new_unique(), false),
            ],
            data,
        };
        let message = Message::new(&[instruction], Some(&payer));
        let transaction = Transaction::new_unsigned(message);
        let encoded = BASE64.encode(bincode::serialize(&transaction).unwrap());

        let wrapper = SolanaTransactionWrapper::from_string(&encoded).unwrap();
        let payload = SolanaVisualSignConverter
            .to_visual_sign_payload(
                wrapper,
                VisualSignOptions {
                    include_intermediate_output: false,
                    metadata: None,
                    decode_transfers: true,
                    transaction_name: Some("svm governance create_proposal".to_string()),
                    developer_config: None,
                },
            )
            .unwrap()
            .payload;

        payload
            .validate_anchorage_wallet_renderable()
            .expect("every field this preset emits must be renderable by the wallet");
        let json = payload
            .to_validated_json()
            .expect("payload must pass charset validation with a non-ASCII proposal title");
        assert!(
            json.contains("SVM Governance: create_proposal"),
            "the SVM Governance preset should handle this program, not the catch-all: {json}"
        );
    }

    /// Build a context for one instruction to this program, with `count`
    /// accounts. Mirrors how the converter compiles a transaction: the program
    /// sits at account_keys[0] and the instruction's accounts follow.
    fn context_fixture(data: &[u8], count: u8) -> (CompiledInstruction, Vec<Pubkey>) {
        let mut account_keys = vec![Pubkey::from_str(SVM_GOVERNANCE_PROGRAM_ID).unwrap()];
        account_keys.extend((0..count).map(|_| Pubkey::new_unique()));
        let compiled = CompiledInstruction {
            program_id_index: 0,
            accounts: (1..=count).collect(),
            data: data.to_vec(),
        };
        (compiled, account_keys)
    }

    /// Pins the failure-mode contract this preset takes on by claiming the
    /// program away from the `unknown_program` catch-all: an instruction it
    /// cannot decode still renders, carrying the program id and the raw data.
    /// It must never turn an input that previously rendered into an error.
    #[test]
    fn test_undecodable_instruction_renders_fallback_instead_of_erroring() {
        let garbage = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x01, 0x02, 0x03, 0x04];
        let (compiled, account_keys) = context_fixture(&garbage, 2);
        let sender = SolanaAccount {
            account_key: account_keys[1].to_string(),
            signer: true,
            writable: true,
        };
        let idl_registry = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

        let field = SvmGovernanceVisualizer
            .visualize_tx_commands(&context)
            .expect("an undecodable instruction must still render");

        let SignablePayloadField::PreviewLayout {
            ref preview_layout, ..
        } = field.signable_payload_field
        else {
            panic!("expected a PreviewLayout field");
        };
        assert_eq!(
            preview_layout.title.as_ref().map(|t| t.text.as_str()),
            Some("SVM Governance: Unknown Instruction")
        );
        let expanded = preview_layout.expanded.as_ref().unwrap();
        let entries: Vec<(String, String)> =
            expanded.fields.iter().map(field_label_value).collect();
        assert!(
            entries
                .iter()
                .any(|(label, value)| label == "Raw Data" && value == &hex::encode(garbage)),
            "the raw bytes must survive so the signer sees something: {entries:?}"
        );
    }

    #[test]
    fn test_build_named_accounts_surfaces_extra_accounts() {
        // accept_admin has 2 named accounts; provide 4 entries so the last 2
        // land in extra_accounts.
        let idl = get_svm_governance_idl().unwrap();
        let disc = discriminator_for("accept_admin");
        let pubkeys: Vec<String> = (0..4).map(|_| Pubkey::new_unique().to_string()).collect();

        let (named, extra) = build_named_accounts(&disc, idl, &pubkeys);

        assert_eq!(named.len(), 2, "first 2 accounts should be named");
        assert_eq!(extra, vec![pubkeys[2].clone(), pubkeys[3].clone()]);
    }

    #[test]
    fn test_unknown_instruction_falls_back_without_erroring() {
        let (title, condensed, expanded) = build_fallback_fields(SVM_GOVERNANCE_PROGRAM_ID).unwrap();
        assert_eq!(title, "SVM Governance: Unknown Instruction");
        assert!(
            condensed
                .iter()
                .map(field_label_value)
                .any(|(label, value)| label == "Program" && value == "SVM Governance")
        );
        assert!(
            expanded
                .iter()
                .map(field_label_value)
                .any(|(label, value)| label == "Program ID" && value == SVM_GOVERNANCE_PROGRAM_ID)
        );
    }
}
