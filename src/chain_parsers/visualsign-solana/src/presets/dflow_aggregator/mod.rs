//! DFlow Aggregator preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::DflowAggregatorConfig;
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

pub(crate) const DFLOW_AGGREGATOR_PROGRAM_ID: &str = "DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH";

const DFLOW_AGGREGATOR_IDL_JSON: &str = include_str!("dflow_aggregator.json");

static DFLOW_AGGREGATOR_CONFIG: DflowAggregatorConfig = DflowAggregatorConfig;

pub struct DflowAggregatorVisualizer;

impl InstructionVisualizer for DflowAggregatorVisualizer {
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

        let parsed = parse_dflow_aggregator_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                tracing::warn!("Failed to parse DFlow Aggregator instruction with IDL: {e}");
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
        Some(&DFLOW_AGGREGATOR_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex("DFlow Aggregator")
    }
}

fn get_dflow_aggregator_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(DFLOW_AGGREGATOR_IDL_JSON).ok())
        .as_ref()
}

fn parse_dflow_aggregator_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<DflowAggregatorParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_dflow_aggregator_idl().ok_or("DFlow Aggregator IDL not available")?;
    let parsed = parse_instruction_with_idl(data, DFLOW_AGGREGATOR_PROGRAM_ID, idl)?;

    let (named_accounts, extra_accounts) = build_named_accounts(data, idl, accounts);

    Ok(DflowAggregatorParsedInstruction {
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

struct DflowAggregatorParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
    extra_accounts: Vec<String>,
}

fn build_parsed_fields(
    instruction: &DflowAggregatorParsedInstruction,
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
    let title = format!("DFlow Aggregator: {instruction_name}");

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "DFlow Aggregator")?);
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
    let title = "DFlow Aggregator: Unknown Instruction".to_string();

    let mut condensed_fields = vec![];
    let mut expanded_fields = vec![];

    condensed_fields.push(create_text_field("Program", "DFlow Aggregator")?);
    condensed_fields.push(create_text_field("Status", "Unknown instruction type")?);

    expanded_fields.push(create_text_field("Program ID", program_id)?);
    expanded_fields.push(create_text_field("Status", "Unknown instruction type")?);

    Ok((title, condensed_fields, expanded_fields))
}

/// Render a single program-call argument as one field value.
///
/// Each top-level argument becomes exactly ONE field. Objects and arrays are
/// rendered as a compact, JSON-like string -- but WITHOUT the `"` quotes that
/// real JSON puts around keys and strings. This matters for two reasons:
///
/// 1. **No field explosion.** We do not recurse into separate fields. A byte
///    array such as `RecordId.id` (76 bytes) would otherwise blow up into 76
///    per-byte fields and bury the meaningful arguments (`quoted_out_amount`,
///    `slippage_bps`, ...). See `test_format_arg_value_does_not_blow_up_nested_fields`.
/// 2. **Charset safety.** `SignablePayload::validate_charset` rejects the `\"`
///    JSON escape (see #332). Real compact JSON of an object contains quoted
///    keys, which serialize to `\"` and fail validation. Emitting quote-free
///    output keeps the whole payload charset-valid. See
///    `test_format_arg_value_is_charset_safe`.
///
/// Arrays whose elements are all byte-sized integers (0..=255) -- e.g. a
/// `[u8; 32]` market id or the 76-byte `RecordId.id` -- render as a single
/// `0x`-prefixed hex string instead of a bracketed number list, which is both
/// shorter and the conventional way to show opaque ids.
///
/// This is a deliberately type-agnostic stopgap. A DFlow-aware renderer that
/// understands the IDL types (Action enum, RecordId, DynamicRoute, ...) and
/// presents proper nested fields is intended to stack on top of this.
fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
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
                .map(|(k, v)| format!("{k}:{}", format_arg_value(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
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
    use serde_json::json;
    use solana_parser::IdlSource;
    use solana_sdk::pubkey::Pubkey;

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

    #[test]
    fn test_dflow_aggregator_idl_loads() {
        let idl = get_dflow_aggregator_idl();
        assert!(
            idl.is_some(),
            "DFlow Aggregator IDL should load successfully"
        );
        let idl = idl.unwrap();
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_dflow_aggregator_idl_has_discriminators() {
        let idl = get_dflow_aggregator_idl().unwrap();
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
        let result = parse_dflow_aggregator_instruction(&garbage_data, &accounts);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let accounts = vec![];
        let result = parse_dflow_aggregator_instruction(&short_data, &accounts);
        assert!(result.is_err(), "Short data should return error");
    }

    #[test]
    fn test_format_arg_value_renders_scalars() {
        assert_eq!(format_arg_value(&json!("hello")), "hello");
        assert_eq!(format_arg_value(&json!(42)), "42");
        assert_eq!(format_arg_value(&json!(true)), "true");
        assert_eq!(format_arg_value(&serde_json::Value::Null), "null");
    }

    #[test]
    fn test_format_arg_value_renders_objects_and_arrays_quote_free() {
        // Objects and arrays collapse into a single quote-free, JSON-like string
        // rather than recursing into per-key / per-element fields. No `"` is
        // emitted (see test_format_arg_value_is_charset_safe).
        assert_eq!(format_arg_value(&json!([])), "[]");
        assert_eq!(format_arg_value(&json!({})), "{}");
        // serde_json::Map is a BTreeMap without the `preserve_order` feature, so
        // keys come out sorted -- assert against that deterministic order.
        assert_eq!(
            format_arg_value(&json!({"side": "buy", "amount": 100})),
            "{amount:100,side:buy}"
        );
        // Arrays of non-byte values render as a bracketed, comma-joined list.
        assert_eq!(
            format_arg_value(&json!([5_000_000u64, 68_980_730u64])),
            "[5000000,68980730]"
        );
        assert_eq!(format_arg_value(&json!(["a", "b"])), "[a,b]");
    }

    #[test]
    fn test_format_arg_value_renders_byte_arrays_as_hex() {
        // All-byte arrays (every element in 0..=255) collapse to one 0x-hex
        // string -- the conventional, compact form for opaque ids like
        // RecordId.id ([u8; 76]) or a [u8; 32] market id.
        assert_eq!(format_arg_value(&json!([1, 2, 3])), "0x010203");
        assert_eq!(format_arg_value(&json!([0, 255])), "0x00ff");
        // A single out-of-byte-range element disqualifies the whole array, so it
        // falls back to the bracketed list rather than mis-rendering as hex.
        assert_eq!(format_arg_value(&json!([1, 2, 256])), "[1,2,256]");
    }

    #[test]
    fn test_format_arg_value_is_charset_safe() {
        // SignablePayload::validate_charset rejects the `\"` and `\\` JSON
        // escapes (#332). Real compact JSON of an object would emit quoted keys
        // (-> `\"`) and fail. Our quote-free rendering must contain neither `"`
        // nor `\` so the whole payload stays charset-valid end to end.
        let nested = json!({
            "actions": [{"RecordId": [{"id": (0u8..76).collect::<Vec<u8>>()}]}],
            "quoted_out_amount": 68_980_730u64,
        });
        let rendered = format_arg_value(&nested);
        assert!(
            !rendered.contains('"') && !rendered.contains('\\'),
            "rendered arg must be charset-safe (no quotes/backslashes), got: {rendered}"
        );
    }

    #[test]
    fn test_real_swap_matches_pre286_field_structure() {
        // Real `swap` instruction bytes from a mainnet DFlow tx.
        //
        // Pins equivalence with the pre-#286 output: that code emitted exactly
        // one field per top-level argument with the value rendered via
        // `serde_json::Value::to_string()` (compact JSON). We assert the SAME
        // single-field structure and the SAME underlying data, with the value
        // re-encoded charset-safe (no `"`; #332) and the byte array shown as hex.
        let data = hex::decode(
            "f8c69e91e17587c802000000252daaa2dfe9ae6201ec11a78f6acf1feffe9ca87508eeee3918bb36d974972fc032d35027909ca6bf0454ff3da70dcafbb3d3f63c230546c40a985cafaf71520a29498619000130a9a20000001f020000000208404b4c000000000001fa8f1c040000000014000000",
        )
        .unwrap();
        let idl = get_dflow_aggregator_idl().unwrap();
        let parsed = parse_instruction_with_idl(&data, DFLOW_AGGREGATOR_PROGRAM_ID, idl).unwrap();

        // Same field structure as pre-#286: exactly one top-level arg -> one field.
        let keys: Vec<&str> = parsed.program_call_args.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["params"]);

        let value = parsed.program_call_args.get("params").unwrap();
        let now = format_arg_value(value);

        // Charset-safe (the pre-#286 compact JSON would fail validate_charset).
        assert!(!now.contains('"') && !now.contains('\\'), "not charset-safe: {now}");
        // Same data as pre-#286, re-encoded: scalars verbatim, byte array as hex,
        // and crucially NOT exploded into a per-byte number list.
        assert!(now.contains("id:0x2daaa2dfe9ae6201"), "byte id not hex-encoded: {now}");
        assert!(!now.contains("[45,170,162"), "byte id leaked as number list: {now}");
        assert!(
            now.contains("quoted_out_amount:68980730")
                && now.contains("slippage_bps:20")
                && now.contains("platform_fee_bps:0"),
            "scalars not preserved: {now}"
        );
    }

    #[test]
    fn test_format_arg_value_does_not_blow_up_nested_fields() {
        // Regression guard for the field explosion (introduced by #286's
        // recursive push_arg_fields, reverted here): a deeply-nested `params`
        // argument that contains a 76-byte array (e.g. RecordId.id) must render
        // as exactly ONE field per top-level argument, NOT one field per byte.
        let id_bytes: Vec<u8> = (0..76).collect();
        let params = json!({
            "actions": [
                {"RecordId": [{"id": id_bytes}]},
                {"DFlowDynamicRouteV1": [{"amount": 5_000_000u64,
                                          "orchestrator_flags": {"flags": 1}}]},
            ],
            "quoted_out_amount": 68_980_730u64,
            "slippage_bps": 20,
            "platform_fee_bps": 0,
        });
        let mut program_call_args = serde_json::Map::new();
        program_call_args.insert("params".to_string(), params.clone());

        let parsed = DflowAggregatorParsedInstruction {
            parsed: SolanaParsedInstructionData {
                instruction_name: "swap".to_string(),
                discriminator: "00".to_string(),
                named_accounts: Default::default(),
                program_call_args,
                idl_source: IdlSource::Custom,
                idl_hash: String::new(),
            },
            named_accounts: BTreeMap::new(),
            extra_accounts: Vec::new(),
        };

        let (_title, condensed, expanded) = build_parsed_fields(&parsed, "PROGRAM_ID").unwrap();

        for (view, fields) in [("condensed", &condensed), ("expanded", &expanded)] {
            let labels: Vec<String> = fields.iter().map(|f| field_label_value(f).0).collect();
            // Exactly one field carries the whole `params` argument.
            let params_fields: Vec<&String> = labels.iter().filter(|l| *l == "params").collect();
            assert_eq!(
                params_fields.len(),
                1,
                "{view} view should render `params` as a single field, got labels: {labels:?}"
            );
            // No flattened / indexed leaf labels leaked through.
            assert!(
                !labels
                    .iter()
                    .any(|l| l.starts_with("params.") || l.contains('[')),
                "{view} view must not explode nested params into per-leaf fields: {labels:?}"
            );
        }

        // The single field holds the whole value quote-free, with the 76-byte
        // RecordId.id collapsed to one 0x-hex string (not a per-byte list) and
        // the meaningful scalars still legible inline.
        let params_value = expanded
            .iter()
            .map(field_label_value)
            .find(|(label, _)| label == "params")
            .map(|(_, value)| value)
            .expect("params field present");
        assert!(
            params_value.contains("id:0x000102030405"),
            "byte array should render as inline hex, got: {params_value}"
        );
        assert!(
            !params_value.contains("[0,1,2,3"),
            "byte array must not render as a per-byte number list, got: {params_value}"
        );
        assert!(
            params_value.contains("quoted_out_amount:68980730")
                && params_value.contains("slippage_bps:20"),
            "meaningful scalars should remain legible inline, got: {params_value}"
        );
        assert!(
            !params_value.contains('"') && !params_value.contains('\\'),
            "rendered params must be charset-safe, got: {params_value}"
        );
    }

    #[test]
    fn test_build_named_accounts_surfaces_extra_accounts() {
        // Look up the discriminator from the IDL rather than hard-coding the
        // bytes, so the test stays correct across IDL regenerations.
        // close_empty_token_account has 4 named accounts; provide 6 entries so
        // the last 2 land in extra_accounts.
        let idl = get_dflow_aggregator_idl().unwrap();
        let close_ix = idl
            .instructions
            .iter()
            .find(|ix| ix.name == "close_empty_token_account")
            .expect("close_empty_token_account exists in the bundled IDL");
        let close_disc = close_ix
            .discriminator
            .as_ref()
            .expect("instruction has a computed discriminator")
            .clone();
        let pubkeys: Vec<Pubkey> = (0..6).map(|_| Pubkey::new_unique()).collect();
        let accounts: Vec<String> = pubkeys.iter().map(|pk| pk.to_string()).collect();

        let (named, extra) = build_named_accounts(&close_disc, idl, &accounts);

        assert_eq!(named.len(), 4, "first 4 accounts should be named");
        assert_eq!(extra.len(), 2, "remaining 2 accounts should be extras");
        assert_eq!(extra[0], pubkeys[4].to_string());
        assert_eq!(extra[1], pubkeys[5].to_string());
    }

    #[test]
    fn test_remaining_account_label_is_human_readable() {
        // Render the parsed-fields path with extra accounts and assert that the labels
        // are "Remaining Account 1", "Remaining Account 2", etc., not snake_case,
        // and that their text values are the corresponding pubkeys in order.
        let pubkeys: Vec<String> = (0..3).map(|_| Pubkey::new_unique().to_string()).collect();
        let parsed = DflowAggregatorParsedInstruction {
            parsed: SolanaParsedInstructionData {
                instruction_name: "test_ix".to_string(),
                discriminator: "00".to_string(),
                named_accounts: Default::default(),
                program_call_args: serde_json::Map::new(),
                idl_source: IdlSource::Custom,
                idl_hash: String::new(),
            },
            named_accounts: BTreeMap::new(),
            extra_accounts: pubkeys.clone(),
        };

        let (_title, _condensed, expanded) = build_parsed_fields(&parsed, "PROGRAM_ID").unwrap();
        let entries: Vec<(String, String)> = expanded
            .iter()
            .map(field_label_value)
            .filter(|(label, _)| label.starts_with("Remaining Account"))
            .collect();
        assert_eq!(
            entries,
            vec![
                ("Remaining Account 1".to_string(), pubkeys[0].clone()),
                ("Remaining Account 2".to_string(), pubkeys[1].clone()),
                ("Remaining Account 3".to_string(), pubkeys[2].clone()),
            ]
        );
    }
}
