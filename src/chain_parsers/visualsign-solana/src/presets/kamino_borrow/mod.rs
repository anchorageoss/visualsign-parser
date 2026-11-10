//! Kamino Borrow preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, NamedIdlAccounts, SolanaIntegrationConfig,
    VisualizerContext, VisualizerKind, name_idl_accounts, remaining_account_label,
};
use config::KaminoBorrowConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const KAMINO_BORROW_PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";

const KAMINO_BORROW_IDL_JSON: &str = include_str!("kamino_borrow.json");

static KAMINO_BORROW_CONFIG: KaminoBorrowConfig = KaminoBorrowConfig;

pub struct KaminoBorrowVisualizer;

impl InstructionVisualizer for KaminoBorrowVisualizer {
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

        let parsed = parse_kamino_borrow_instruction(data, &view.accounts);

        let (title, condensed_fields, expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id),
            Err(_) => build_fallback_fields(&view.program_id),
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        let expanded_with_raw = append_raw_data(expanded_fields, data, &instruction_data_hex);
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_with_raw,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {}", context.instruction_index() + 1),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&KAMINO_BORROW_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending("Kamino Borrow")
    }
}

fn get_kamino_borrow_idl() -> Option<Idl> {
    decode_idl_data(KAMINO_BORROW_IDL_JSON).ok()
}

fn parse_kamino_borrow_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<KaminoBorrowParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_kamino_borrow_idl().ok_or("Kamino Borrow IDL not available")?;
    let parsed = parse_instruction_with_idl(data, KAMINO_BORROW_PROGRAM_ID, &idl)?;

    let named_accounts = build_named_accounts(data, &idl, accounts);

    Ok(KaminoBorrowParsedInstruction {
        parsed,
        named_accounts,
    })
}

/// Name the instruction's accounts against the IDL, flattening account groups
/// (composite `Accounts` structs) so members and the accounts after a group
/// land on the right addresses.
fn build_named_accounts(data: &[u8], idl: &Idl, accounts: &[String]) -> NamedIdlAccounts {
    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.len() >= disc.len() && data[..disc.len()] == *disc)
    });

    match idl_instruction {
        Some(instruction) => name_idl_accounts(instruction, accounts),
        None => NamedIdlAccounts::default(),
    }
}

struct KaminoBorrowParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: NamedIdlAccounts,
}

fn build_parsed_fields(
    instruction: &KaminoBorrowParsedInstruction,
    program_id: &str,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let parsed = &instruction.parsed;
    let title = format!("Kamino Borrow: {}", parsed.instruction_name);

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    if let Ok(f) = create_text_field("Program", "Kamino Borrow") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", &parsed.instruction_name) {
        condensed_fields.push(f);
    }
    for (key, value) in &parsed.program_call_args {
        if let Ok(f) = create_text_field(key, &format_arg_value(value)) {
            condensed_fields.push(f);
        }
    }

    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Instruction", &parsed.instruction_name) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Discriminator", &parsed.discriminator) {
        expanded_fields.push(f);
    }

    for (account_name, account_address) in &instruction.named_accounts.named {
        if let Ok(f) = create_text_field(account_name, account_address) {
            expanded_fields.push(f);
        }
    }

    for (index, pubkey) in instruction.named_accounts.remaining.iter().enumerate() {
        if let Ok(f) = create_text_field(&remaining_account_label(index), pubkey) {
            expanded_fields.push(f);
        }
    }

    for (key, value) in &parsed.program_call_args {
        if let Ok(f) = create_text_field(key, &format_arg_value(value)) {
            expanded_fields.push(f);
        }
    }

    (title, condensed_fields, expanded_fields)
}

fn build_fallback_fields(
    program_id: &str,
) -> (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
) {
    let title = "Kamino Borrow: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    if let Ok(f) = create_text_field("Program", "Kamino Borrow") {
        condensed_fields.push(f);
    }
    if let Ok(f) = create_text_field("Status", "Unknown instruction type") {
        condensed_fields.push(f);
    }

    if let Ok(f) = create_text_field("Program ID", program_id) {
        expanded_fields.push(f);
    }
    if let Ok(f) = create_text_field("Status", "Unknown instruction type") {
        expanded_fields.push(f);
    }

    (title, condensed_fields, expanded_fields)
}

fn append_raw_data(
    mut fields: Vec<AnnotatedPayloadField>,
    data: &[u8],
    hex_str: &str,
) -> Vec<AnnotatedPayloadField> {
    if let Ok(f) = create_raw_data_field(data, Some(hex_str.to_string())) {
        fields.push(f);
    }
    fields
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_kamino_borrow_idl_loads() {
        let idl = get_kamino_borrow_idl();
        assert!(idl.is_some(), "Kamino Borrow IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_kamino_borrow_idl_has_discriminators() {
        let idl = get_kamino_borrow_idl().unwrap();
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
        let result = parse_kamino_borrow_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_kamino_borrow_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }

    #[test]
    fn test_group_members_and_following_accounts_land_on_right_addresses() {
        // socializeLossV2 nests two composite account groups
        // (socializeLossAccounts, farmsAccounts) before the trailing
        // lendingMarketAuthority and farmsProgram entries. Pairing top-level
        // IDL accounts by position (the old bug) shifted every name from
        // lendingMarketAuthority onward onto the wrong address.
        let idl = get_kamino_borrow_idl().unwrap();
        let ix = idl
            .instructions
            .iter()
            .find(|i| i.name == "socializeLossV2")
            .unwrap();
        let accounts: Vec<String> = (0..9).map(|i| format!("addr{i}")).collect();

        let named = build_named_accounts(ix.discriminator.as_ref().unwrap(), &idl, &accounts);

        // Group members take the group's prefix and the group's positions.
        assert_eq!(
            named.named.get("socializeLossAccounts.lendingMarketOwner"),
            Some(&"addr0".to_string())
        );
        assert_eq!(
            named.named.get("farmsAccounts.reserveFarmState"),
            Some(&"addr6".to_string())
        );
        // Accounts after the groups keep their own names at the flattened
        // positions (5 + 2 = 7 group members come first).
        assert_eq!(
            named.named.get("lendingMarketAuthority"),
            Some(&"addr7".to_string())
        );
        assert_eq!(named.named.get("farmsProgram"), Some(&"addr8".to_string()));
        assert!(named.remaining.is_empty());
    }
}
