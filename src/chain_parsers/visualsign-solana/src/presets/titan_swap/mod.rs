//! Titan Swap preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::TitanSwapConfig;
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

pub(crate) const TITAN_SWAP_PROGRAM_ID: &str = "T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT";

const TITAN_SWAP_IDL_JSON: &str = include_str!("titan_swap.json");

static TITAN_SWAP_CONFIG: TitanSwapConfig = TitanSwapConfig;

pub struct TitanSwapVisualizer;

impl InstructionVisualizer for TitanSwapVisualizer {
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

        let parsed = parse_titan_swap_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                tracing::warn!("Failed to parse Titan Swap instruction with IDL: {e}");
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
        Some(&TITAN_SWAP_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("Titan Swap")
    }
}

fn get_titan_swap_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(TITAN_SWAP_IDL_JSON).ok())
        .as_ref()
}

fn parse_titan_swap_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<TitanSwapParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_titan_swap_idl().ok_or("Titan Swap IDL not available")?;
    let parsed = parse_instruction_with_idl(data, TITAN_SWAP_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(TitanSwapParsedInstruction {
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

struct TitanSwapParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &TitanSwapParsedInstruction,
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
    let title = format!("Titan Swap: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Titan Swap")?);
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
    let title = "Titan Swap: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "Titan Swap")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

    Ok((title, condensed_fields, expanded_fields))
}

/// Render a single program-call argument as one field value. Objects and
/// arrays collapse into a compact, quote-free JSON-like string rather than
/// recursing into separate fields, so a large nested arg (e.g. a `swaps`
/// vec of route steps) doesn't explode into dozens of per-field entries.
/// Byte arrays render as a single `0x`-prefixed hex string.
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

/// Strips `"` and `\` to preserve this preset's quote-free rendering contract
/// (not required by `SignablePayload::validate_charset` itself, which allows
/// those escapes), and strips non-printable-ASCII/control bytes, which
/// `validate_charset` does still forbid. IDL strings here (pubkeys, enum
/// names) are already clean; this is a defensive guard so the function's
/// charset-safe contract always holds.
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
    use solana_sdk::pubkey::Pubkey;

    // One `SwapSpecInput` entry: venue = RaydiumAmm (enum discriminant 0, no
    // fields) + from(u8) + to(u8) + weight_bps(u16 LE) + minimum_amount_out(u64 LE)
    // + n_accounts(u8).
    fn swap_spec_input_v1_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8]; // Venue::RaydiumAmm discriminant
        bytes.push(0); // from
        bytes.push(1); // to
        bytes.extend_from_slice(&50u16.to_le_bytes()); // weight_bps
        bytes.extend_from_slice(&900u64.to_le_bytes()); // minimum_amount_out
        bytes.push(4); // n_accounts
        bytes
    }

    // One `SwapSpecInputV2` entry: venue = RaydiumAmm + from(u8) + to(u8) +
    // weight_nanos(u32 LE) + n_accounts(u8).
    fn swap_spec_input_v2_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8]; // Venue::RaydiumAmm discriminant
        bytes.push(0); // from
        bytes.push(1); // to
        bytes.extend_from_slice(&500_000_000u32.to_le_bytes()); // weight_nanos
        bytes.push(4); // n_accounts
        bytes
    }

    fn swap_route_args(spec_bytes: &[u8]) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // amount
        data.extend_from_slice(&900_000u64.to_le_bytes()); // minimum_amount_out
        data.push(2); // mints
        data.extend_from_slice(&10u16.to_le_bytes()); // provider_fee_bps
        data.extend_from_slice(&5u16.to_le_bytes()); // service_fee_bps
        data.extend_from_slice(&1u32.to_le_bytes()); // swaps: Vec len = 1
        data.extend_from_slice(spec_bytes);
        data
    }

    #[test]
    fn test_swap_route_instruction_parses() {
        let idl = get_titan_swap_idl().expect("IDL must load");
        let swap_route_disc = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "swap_route")
            .and_then(|ix| ix.discriminator.clone())
            .expect("swap_route has a discriminator");

        let mut data = swap_route_disc;
        data.extend_from_slice(&swap_route_args(&swap_spec_input_v1_bytes()));

        let parsed =
            parse_instruction_with_idl(&data, TITAN_SWAP_PROGRAM_ID, idl).expect("must parse");
        assert_eq!(parsed.instruction_name, "swap_route");
        assert!(parsed.program_call_args.contains_key("amount"));
        assert!(parsed.program_call_args.contains_key("minimum_amount_out"));
        assert!(parsed.program_call_args.contains_key("swaps"));
    }

    #[test]
    fn test_swap_route_v2_instruction_parses() {
        let idl = get_titan_swap_idl().expect("IDL must load");
        let swap_route_v2_disc = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "swap_route_v2")
            .and_then(|ix| ix.discriminator.clone())
            .expect("swap_route_v2 has a discriminator");

        let mut data = swap_route_v2_disc;
        data.extend_from_slice(&swap_route_args(&swap_spec_input_v2_bytes()));

        let parsed =
            parse_instruction_with_idl(&data, TITAN_SWAP_PROGRAM_ID, idl).expect("must parse");
        assert_eq!(parsed.instruction_name, "swap_route_v2");
        assert!(parsed.program_call_args.contains_key("amount"));
        assert!(parsed.program_call_args.contains_key("swaps"));
    }

    #[test]
    fn test_build_named_accounts_surfaces_extra_accounts() {
        let idl = get_titan_swap_idl().expect("IDL must load");
        let swap_route_disc = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "swap_route")
            .and_then(|ix| ix.discriminator.clone())
            .expect("swap_route has a discriminator");

        // swap_route has 8 named accounts; provide 10 so the last 2 land in
        // extra_accounts.
        let pubkeys: Vec<Pubkey> = (0..10).map(|_| Pubkey::new_unique()).collect();
        let accounts: Vec<String> = pubkeys.iter().map(|pk| pk.to_string()).collect();

        let (named, extra) = build_named_accounts(&swap_route_disc, idl, &accounts);

        assert_eq!(named.len(), 8, "first 8 accounts should be named");
        assert_eq!(extra.len(), 2, "remaining 2 accounts should be extras");
        assert_eq!(extra[0], pubkeys[8].to_string());
        assert_eq!(extra[1], pubkeys[9].to_string());
    }

    #[test]
    fn test_remaining_account_label_is_human_readable() {
        let pubkeys: Vec<String> = (0..2).map(|_| Pubkey::new_unique().to_string()).collect();
        let parsed = TitanSwapParsedInstruction {
            parsed: SolanaParsedInstructionData {
                instruction_name: "test_ix".to_string(),
                discriminator: "00".to_string(),
                named_accounts: Default::default(),
                program_call_args: serde_json::Map::new(),
                idl_source: solana_parser::IdlSource::Custom,
                idl_hash: String::new(),
            },
            named_accounts: BTreeMap::new(),
            extra_accounts: pubkeys.clone(),
        };

        let (_title, _condensed, expanded) = build_parsed_fields(&parsed, "PROGRAM_ID").unwrap();
        let entries: Vec<(String, String)> = expanded
            .iter()
            .filter_map(|field| match &field.signable_payload_field {
                SignablePayloadField::TextV2 { common, text_v2 } => {
                    Some((common.label.clone(), text_v2.text.clone()))
                }
                _ => None,
            })
            .filter(|(label, _)| label.starts_with("Remaining Account"))
            .collect();
        assert_eq!(
            entries,
            vec![
                ("Remaining Account 1".to_string(), pubkeys[0].clone()),
                ("Remaining Account 2".to_string(), pubkeys[1].clone()),
            ]
        );
    }

    #[test]
    fn test_format_arg_value_renders_objects_and_arrays_quote_free() {
        assert_eq!(format_arg_value(&serde_json::json!([])), "[]");
        assert_eq!(format_arg_value(&serde_json::json!({})), "{}");
        let object = format_arg_value(&serde_json::json!({"venue": "RaydiumAmm", "from": 0}));
        assert!(
            object == "{from:0,venue:RaydiumAmm}" || object == "{venue:RaydiumAmm,from:0}",
            "object should render both fields quote-free in some order: {object}"
        );
        assert_eq!(
            format_arg_value(&serde_json::json!([1000000u64, 900000u64])),
            "[1000000,900000]"
        );
    }

    #[test]
    fn test_format_arg_value_renders_byte_arrays_as_hex() {
        assert_eq!(format_arg_value(&serde_json::json!([1, 2, 3])), "0x010203");
        assert_eq!(format_arg_value(&serde_json::json!([0, 255])), "0x00ff");
        // A single out-of-byte-range element disqualifies the whole array.
        assert_eq!(
            format_arg_value(&serde_json::json!([1, 2, 256])),
            "[1,2,256]"
        );
    }

    #[test]
    fn test_format_arg_value_is_charset_safe() {
        let nested = serde_json::json!({
            "swaps": [{"venue": "RaydiumAmm", "minimum_amount_out": 900u64}],
            "amount": 1_000_000u64,
        });
        let rendered = format_arg_value(&nested);
        assert!(
            !rendered.contains('"') && !rendered.contains('\\'),
            "rendered arg must be charset-safe (no quotes/backslashes), got: {rendered}"
        );
    }

    #[test]
    fn test_format_arg_value_sanitizes_string_values_and_keys() {
        assert_eq!(format_arg_value(&serde_json::json!("a\"b\\c\td")), "abcd");
        let object = format_arg_value(&serde_json::json!({"a\"b": "x\\y"}));
        assert!(
            !object.contains('"') && !object.contains('\\'),
            "object keys and values must be sanitized: {object}"
        );
        assert!(
            object.contains("ab:xy"),
            "expected sanitized key:value, got: {object}"
        );
    }

    #[test]
    fn test_titan_swap_idl_loads() {
        let idl = get_titan_swap_idl();
        assert!(idl.is_some(), "Titan Swap IDL should load successfully");
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_titan_swap_idl_has_discriminators() {
        let idl = get_titan_swap_idl().unwrap();
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
        let result = parse_titan_swap_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_titan_swap_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }
}
