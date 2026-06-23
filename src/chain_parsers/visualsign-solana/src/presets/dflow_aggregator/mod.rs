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
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::OnceLock;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_number_field, create_raw_data_field, create_text_field};
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
                // When the account is ALT-unresolved but the IDL pins a constant
                // `address` for this slot (Anchor enforces `address == X` at
                // runtime, so a successful tx guarantees it), surface that known
                // address instead of an opaque `unresolved(N)`.
                let value = if is_unresolved(account_str) {
                    idl_known_addresses()
                        .get(&idl_account.name)
                        .cloned()
                        .unwrap_or_else(|| account_str.clone())
                } else {
                    account_str.clone()
                };
                named_accounts.insert(idl_account.name.clone(), value);
            } else {
                extra_accounts.push(account_str.clone());
            }
        }
    }

    (named_accounts, extra_accounts)
}

/// Account name -> a statically-known address declared in the bundled IDL,
/// covering two cases (both guaranteed by Anchor at runtime, so safe to show for
/// an ALT-unresolved slot):
///
/// 1. A constant `address` constraint (e.g. `token_program`).
/// 2. A PDA whose seeds are all `const`, with no program override -- derived
///    offline via `find_program_address(seeds, <DFlow program>)` (e.g.
///    `event_authority` from the const seed `__event_authority`).
///
/// `solana_parser` drops both the IDL `address` and `pda`, so they are parsed
/// from the raw IDL JSON here. Names map to the same address across every
/// instruction, so a flat name->address map is sufficient.
fn idl_known_addresses() -> &'static BTreeMap<String, String> {
    static MAP: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = BTreeMap::new();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(DFLOW_AGGREGATOR_IDL_JSON) else {
            return map;
        };
        let program = Pubkey::from_str(DFLOW_AGGREGATOR_PROGRAM_ID).ok();
        let instructions = value
            .get("instructions")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for instruction in &instructions {
            let Some(accounts) = instruction.get("accounts").and_then(|a| a.as_array()) else {
                continue;
            };
            for account in accounts {
                let Some(name) = account.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                if map.contains_key(name) {
                    continue;
                }
                // 1. constant address constraint
                if let Some(address) = account.get("address").and_then(|a| a.as_str()) {
                    map.insert(name.to_string(), address.to_string());
                    continue;
                }
                // 2. const-seed PDA derived against the DFlow program (no program override)
                if let (Some(program), Some(pda)) = (program.as_ref(), account.get("pda")) {
                    if pda.get("program").is_none() {
                        if let Some(seeds) = const_pda_seeds(pda) {
                            let seed_refs: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
                            let (pubkey, _bump) = Pubkey::find_program_address(&seed_refs, program);
                            map.insert(name.to_string(), pubkey.to_string());
                        }
                    }
                }
            }
        }
        map
    })
}

/// Extract all-`const` seeds from an IDL `pda`, or `None` if any seed is not a
/// constant (so the PDA cannot be derived offline -- it depends on runtime args
/// or accounts we don't have).
fn const_pda_seeds(pda: &serde_json::Value) -> Option<Vec<Vec<u8>>> {
    let seeds = pda.get("seeds")?.as_array()?;
    let mut out = Vec::with_capacity(seeds.len());
    for seed in seeds {
        if seed.get("kind").and_then(|k| k.as_str()) != Some("const") {
            return None;
        }
        let bytes = seed
            .get("value")?
            .as_array()?
            .iter()
            .map(|n| n.as_u64().filter(|v| *v <= u8::MAX as u64).map(|v| v as u8))
            .collect::<Option<Vec<u8>>>()?;
        out.push(bytes);
    }
    Some(out)
}

/// An account value the parser could not resolve (an ALT-loaded slot).
fn is_unresolved(account: &str) -> bool {
    account.starts_with("unresolved")
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

    // `swap`/`swap2` get a DFlow-aware grouped layout: a route summary +
    // economics in the condensed (at-a-glance) view; the expanded view holds
    // collapsible "Swap Details" / "Route" / "Accounts" cards (each a nested
    // `preview_layout` with a condensed summary + expanded detail -- the same
    // inner-program pattern squads_multisig uses). A section that reduces to a
    // single item is rendered as a plain field instead of a card. Other
    // instructions keep the flat generic layout.
    match parsed
        .program_call_args
        .iter()
        .find_map(|(key, value)| as_swap_params(key, value))
    {
        Some(params) => {
            if let Some(route) = route_summary(params) {
                condensed_fields.push(create_text_field("Route", &route)?);
            }
            condensed_fields.extend(economics_fields(params)?);

            expanded_fields.extend(swap_details_section(
                program_id,
                &parsed.discriminator,
                params,
            )?);
            expanded_fields.extend(route_section(params)?);
            expanded_fields.extend(accounts_section(
                &instruction.named_accounts,
                &instruction.extra_accounts,
            )?);
        }
        None => {
            expanded_fields.push(create_text_field("Program ID", program_id)?);
            expanded_fields.push(create_text_field("Instruction", instruction_name)?);
            expanded_fields.push(create_text_field("Discriminator", &parsed.discriminator)?);
            for (key, value) in &parsed.program_call_args {
                condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
            }
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
        }
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

// --- DFlow-aware rendering of `swap` / `swap2` params ---------------------
//
// The `swap` family carries a `SwapParams { actions: vec<Action>,
// quoted_out_amount, slippage_bps, platform_fee_bps }`. Rather than collapse it
// into one opaque field, surface the economics (what the signer pays/receives)
// and a per-action route breakdown. Every field is a renderable leaf
// (`text_v2`, or `number` which serializes as `text_v2`); arrays of bytes stay
// hex via `format_arg_value`. See the Anchorage wallet render rules.

/// Recognize a `swap`/`swap2` `params` argument by shape: an object carrying
/// the `actions` route and the `quoted_out_amount` economics. Returns the
/// object so the caller can render it structurally; anything else falls back to
/// the generic [`format_arg_value`] path.
fn as_swap_params<'a>(
    key: &str,
    value: &'a serde_json::Value,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    if key != "params" {
        return None;
    }
    let map = value.as_object()?;
    if map.contains_key("actions") && map.contains_key("quoted_out_amount") {
        Some(map)
    } else {
        None
    }
}

// Accounts and per-action detail are rendered FLAT (not nested
// `preview_layout` groups): the wallet only shows a nested expandable's
// `addedInformation`, hiding its `expandedDetails` behind a second tap that
// does not usefully surface the addresses. Listing them flat under header rows
// keeps every value visible when the instruction is expanded.

/// `snake_case` IDL account name -> `Title Case` label.
fn humanize(name: &str) -> String {
    name.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A collapsible card: the wallet renders a `preview_layout`'s Condensed inline
/// and drills into its Expanded on tap (the inner-program pattern
/// `squads_multisig` uses). Build sections with [`section`] for single-item
/// collapse.
fn nested_group(
    label: &str,
    condensed: Vec<AnnotatedPayloadField>,
    expanded: Vec<AnnotatedPayloadField>,
) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: label.to_string(),
                fallback_text: label.to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: label.to_string(),
                }),
                subtitle: None,
                condensed: Some(SignablePayloadFieldListLayout { fields: condensed }),
                expanded: Some(SignablePayloadFieldListLayout { fields: expanded }),
            },
        },
    }
}

/// A section: a collapsible `summary` -> `detail` card when it has multiple
/// detail rows; the single row itself when there's exactly one (a lone item
/// shouldn't be wrapped in a card); nothing when empty.
fn section(
    label: &str,
    summary: Vec<AnnotatedPayloadField>,
    detail: Vec<AnnotatedPayloadField>,
) -> Vec<AnnotatedPayloadField> {
    match detail.len() {
        0 => Vec::new(),
        1 => detail,
        _ => vec![nested_group(label, summary, detail)],
    }
}

/// "Swap Details" card: economics as the summary, with program id + discriminator
/// added on drill-in.
fn swap_details_section(
    program_id: &str,
    discriminator: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let economics = economics_fields(params)?;
    let mut detail = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Discriminator", discriminator)?,
    ];
    detail.extend(economics.iter().cloned());
    Ok(section("Swap Details", economics, detail))
}

/// "Route" card: a one-line venue summary, with the per-action breakdown on
/// drill-in. A single-field action collapses to one `<Variant>` row; a
/// multi-field action lists its fields under `<Variant> / <field>`.
fn route_section(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut detail = Vec::new();
    if let Some(actions) = params.get("actions").and_then(|a| a.as_array()) {
        for action in actions {
            let Some((variant, payload)) = single_variant(action) else {
                continue;
            };
            let options = payload
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_object());
            match options {
                // Single-item action -> just a field (e.g. RecordId -> its id).
                Some(options) if options.len() == 1 => {
                    if let Some((field_name, field_value)) = options.iter().next() {
                        detail.push(create_text_field(
                            variant,
                            &action_field_value(field_name, field_value),
                        )?);
                    }
                }
                Some(options) => {
                    for (field_name, field_value) in options {
                        // `candidate_actions` reads better as "candidates".
                        let display = if field_name == "candidate_actions" {
                            "candidates"
                        } else {
                            field_name
                        };
                        detail.push(create_text_field(
                            &format!("{variant} / {display}"),
                            &action_field_value(field_name, field_value),
                        )?);
                    }
                }
                None => detail.push(create_text_field(variant, "")?),
            }
        }
    }
    let summary = match route_summary(params) {
        Some(route) => vec![create_text_field("Route", &route)?],
        None => Vec::new(),
    };
    Ok(section("Route", summary, detail))
}

/// Render an action option value, expanding a `candidate_actions` venue list to
/// its variant names.
fn action_field_value(field_name: &str, field_value: &serde_json::Value) -> String {
    if field_name == "candidate_actions" {
        if let Some(venues) = variant_names(field_value) {
            return venues.join(", ");
        }
    }
    format_arg_value(field_value)
}

/// "Accounts" card: a `<n> resolved, <m> unresolved` count summary, with the
/// detail on drill-in -- the resolved, IDL-named accounts listed by address,
/// plus a single "Unresolved (lookup table)" row listing the ALT account
/// indices. The unresolved entries carry no address (they live in a lookup
/// table the parser can't read), so they collapse to a compact index list
/// rather than one placeholder row each.
fn accounts_section(
    named: &BTreeMap<String, String>,
    extra: &[String],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut resolved = Vec::new();
    let mut unresolved_indices = Vec::new();

    // Named accounts keep their humanized IDL role as the label.
    for (name, address) in named {
        match unresolved_index(address) {
            Some(index) => unresolved_indices.push(index.to_string()),
            None => resolved.push(create_text_field(&humanize(name), address)?),
        }
    }
    // Remaining (unnamed) resolved accounts list their address under "Account".
    for address in extra {
        match unresolved_index(address) {
            Some(index) => unresolved_indices.push(index.to_string()),
            None => resolved.push(create_text_field("Account", address)?),
        }
    }

    let summary = create_text_field(
        "Accounts",
        &format!(
            "{} resolved, {} unresolved",
            resolved.len(),
            unresolved_indices.len()
        ),
    )?;

    let mut detail = resolved;
    if !unresolved_indices.is_empty() {
        detail.push(create_text_field(
            "Unresolved (lookup table)",
            &unresolved_indices.join(", "),
        )?);
    }
    if detail.is_empty() {
        // Nothing to enumerate -- surface just the count.
        return Ok(vec![summary]);
    }
    Ok(vec![nested_group("Accounts", vec![summary], detail)])
}

/// The ALT account index `N` from an `unresolved(N)` placeholder, or `None` if
/// the value is a resolved address.
fn unresolved_index(account: &str) -> Option<&str> {
    account.strip_prefix("unresolved(")?.strip_suffix(')')
}

/// `quoted_out_amount` / `slippage_bps` / `platform_fee_bps`, each as its own
/// number field (which renders as text -- these are not amounts).
///
/// `quoted_out_amount` is shown in raw base units: the output mint lives in an
/// ALT-loaded account we can't resolve offline, and there is no wallet->Solana
/// token-metadata channel yet. Once both exist, this is the single place to
/// render it via `create_amount_field` (symbol + decimals). Tracked by #388
/// (Solana wallet token-metadata channel, mirroring Ethereum's ContractRegistry).
fn economics_fields(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = Vec::new();
    for (key, label, unit) in [
        ("quoted_out_amount", "Quoted Out Amount", ""),
        ("slippage_bps", "Slippage", "bps"),
        ("platform_fee_bps", "Platform Fee", "bps"),
    ] {
        if let Some(text) = params.get(key).and_then(number_text) {
            fields.push(create_number_field(label, &text, unit)?);
        }
    }
    Ok(fields)
}

/// One-line route summary: the venues the order flows through. A
/// `DFlowDynamicRouteV1` is expanded to its candidate venues; bookkeeping
/// actions (`RecordId`, `TransferFee`, ...) are omitted.
fn route_summary(params: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let actions = params.get("actions")?.as_array()?;
    let mut venues: Vec<String> = Vec::new();
    let mut dynamic_route = false;
    for action in actions {
        let Some((variant, payload)) = single_variant(action) else {
            continue;
        };
        if variant == "DFlowDynamicRouteV1" {
            dynamic_route = true;
            if let Some(candidates) = payload
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_object())
                .and_then(|o| o.get("candidate_actions"))
                .and_then(variant_names)
            {
                venues.extend(candidates);
            }
        } else if let Some(name) = variant.strip_suffix("Swap") {
            venues.push(name.to_string());
        }
    }
    if venues.is_empty() {
        return None;
    }
    let joined = venues.join(", ");
    Some(if dynamic_route {
        format!("DFlow Dynamic Route via {joined}")
    } else {
        format!("via {joined}")
    })
}

/// If `action` is a single-key object (a serde-encoded enum variant), return its
/// `(variant_name, payload)`.
fn single_variant(action: &serde_json::Value) -> Option<(&str, &serde_json::Value)> {
    let map = action.as_object()?;
    if map.len() != 1 {
        return None;
    }
    map.iter().next().map(|(k, v)| (k.as_str(), v))
}

/// Collect the variant names from an array of single-key-object enum values
/// (e.g. a `candidate_actions` list -> `["TesseraV", "BisonFi"]`).
fn variant_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| single_variant(item).map(|(name, _)| name.to_string()))
            .collect(),
    )
}

/// String form of a JSON number, or `None` for non-numbers.
fn number_text(value: &serde_json::Value) -> Option<String> {
    value.as_number().map(|n| n.to_string())
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
        let keys: Vec<&str> = parsed
            .program_call_args
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["params"]);

        let value = parsed.program_call_args.get("params").unwrap();
        let now = format_arg_value(value);

        // Charset-safe (the pre-#286 compact JSON would fail validate_charset).
        assert!(
            !now.contains('"') && !now.contains('\\'),
            "not charset-safe: {now}"
        );
        // Same data as pre-#286, re-encoded: scalars verbatim, byte array as hex,
        // and crucially NOT exploded into a per-byte number list.
        assert!(
            now.contains("id:0x2daaa2dfe9ae6201"),
            "byte id not hex-encoded: {now}"
        );
        assert!(
            !now.contains("[45,170,162"),
            "byte id leaked as number list: {now}"
        );
        assert!(
            now.contains("quoted_out_amount:68980730")
                && now.contains("slippage_bps:20")
                && now.contains("platform_fee_bps:0"),
            "scalars not preserved: {now}"
        );
    }

    /// Recursively flatten fields -- descending into nested `preview_layout`
    /// groups -- into (label, display-value) pairs. The display value comes from
    /// fallback_text, so it works for `Number` economics fields too.
    fn flatten_fields(fields: &[AnnotatedPayloadField], out: &mut Vec<(String, String)>) {
        for field in fields {
            match &field.signable_payload_field {
                SignablePayloadField::PreviewLayout {
                    common,
                    preview_layout,
                } => {
                    out.push((common.label.clone(), common.fallback_text.clone()));
                    if let Some(condensed) = &preview_layout.condensed {
                        flatten_fields(&condensed.fields, out);
                    }
                    if let Some(expanded) = &preview_layout.expanded {
                        flatten_fields(&expanded.fields, out);
                    }
                }
                other => out.push((other.label().clone(), other.fallback_text().clone())),
            }
        }
    }

    #[test]
    fn test_swap_params_render_dflow_aware_grouped() {
        // `swap` params render into collapsible cards: route + economics in the
        // condensed view; Swap Details / Route / Accounts cards (each a nested
        // preview_layout: condensed summary + expanded detail) in expanded. A
        // single-field action collapses to one row; the 76-byte RecordId.id stays
        // ONE hex field; ALT-unresolved accounts are counted, not listed; every
        // value is charset-safe.
        let id_bytes: Vec<u8> = (0..76).collect();
        let params = json!({
            "actions": [
                {"RecordId": [{"id": id_bytes}]},
                {"DFlowDynamicRouteV1": [{
                    "candidate_actions": [{"TesseraV": [{}]}, {"BisonFi": [{}]}],
                    "amount": 5_000_000u64,
                    "orchestrator_flags": {"flags": 1}
                }]},
            ],
            "quoted_out_amount": 68_980_730u64,
            "slippage_bps": 20,
            "platform_fee_bps": 0,
        });
        let mut program_call_args = serde_json::Map::new();
        program_call_args.insert("params".to_string(), params);

        let parsed = DflowAggregatorParsedInstruction {
            parsed: SolanaParsedInstructionData {
                instruction_name: "swap".to_string(),
                discriminator: "00".to_string(),
                named_accounts: Default::default(),
                program_call_args,
                idl_source: IdlSource::Custom,
                idl_hash: String::new(),
            },
            // One resolved + one unresolved named account, and likewise extras,
            // to exercise the resolved/unresolved partitioning.
            named_accounts: BTreeMap::from([
                (
                    "token_program".to_string(),
                    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                ),
                ("event_authority".to_string(), "unresolved(18)".to_string()),
            ]),
            extra_accounts: vec![
                "oot7FZrQ9BEUWTABBSgE5dgmxvcFteG3Mg97N9SEyAe".to_string(),
                "unresolved(5)".to_string(),
            ],
        };

        let (_title, condensed, expanded) = build_parsed_fields(&parsed, "PROGRAM_ID").unwrap();
        let mut condensed_pairs = Vec::new();
        flatten_fields(&condensed, &mut condensed_pairs);
        let mut expanded_pairs = Vec::new();
        flatten_fields(&expanded, &mut expanded_pairs);
        let find = |pairs: &[(String, String)], label: &str| -> Option<String> {
            pairs
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.clone())
        };

        // Condensed: route summary + economics.
        assert_eq!(
            find(&condensed_pairs, "Route").as_deref(),
            Some("DFlow Dynamic Route via TesseraV, BisonFi")
        );
        assert_eq!(
            find(&condensed_pairs, "Quoted Out Amount").as_deref(),
            Some("68980730")
        );
        assert_eq!(
            find(&condensed_pairs, "Slippage").as_deref(),
            Some("20 bps")
        );
        assert_eq!(
            find(&condensed_pairs, "Platform Fee").as_deref(),
            Some("0 bps")
        );

        // Expanded holds collapsible cards in order: Swap Details -> Route -> Accounts.
        let section_index =
            |name: &str| -> Option<usize> { expanded_pairs.iter().position(|(l, _)| l == name) };
        let swap_details = section_index("Swap Details").expect("Swap Details card");
        let route = section_index("Route").expect("Route card");
        let accounts = section_index("Accounts").expect("Accounts card");
        assert!(
            swap_details < route && route < accounts,
            "cards must be ordered Swap Details < Route < Accounts: {expanded_pairs:?}"
        );
        // Swap Details surfaces the instruction metadata under its card.
        assert!(
            section_index("Program ID").is_some_and(|i| i > swap_details && i < route),
            "Program ID under Swap Details: {expanded_pairs:?}"
        );
        assert!(
            section_index("Discriminator").is_some_and(|i| i > swap_details && i < route),
            "Discriminator under Swap Details: {expanded_pairs:?}"
        );

        // Route: a single-field action collapses to one `<Variant>` row (RecordId
        // -> its id, one hex field); a multi-field action lists `<Variant> / <field>`.
        let record_id = find(&expanded_pairs, "RecordId").expect("RecordId row");
        assert!(
            record_id.starts_with("0x000102030405"),
            "id should be hex: {record_id}"
        );
        assert_eq!(
            find(&expanded_pairs, "DFlowDynamicRouteV1 / amount").as_deref(),
            Some("5000000")
        );
        assert_eq!(
            find(&expanded_pairs, "DFlowDynamicRouteV1 / candidates").as_deref(),
            Some("TesseraV, BisonFi")
        );

        // Accounts: count summary + resolved (named) accounts only; the
        // ALT-unresolved ones are counted, not listed.
        assert!(
            expanded_pairs
                .iter()
                .any(|(l, v)| l == "Accounts" && v == "2 resolved, 2 unresolved"),
            "accounts summary present: {expanded_pairs:?}"
        );
        assert_eq!(
            find(&expanded_pairs, "Token Program").as_deref(),
            Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        );
        // Unresolved accounts collapse to a single row listing their ALT indices
        // (event_authority -> 18, plus the extra unresolved(5)), not one row each.
        let unresolved = find(&expanded_pairs, "Unresolved (lookup table)")
            .expect("unresolved index row present");
        assert!(
            unresolved.contains("18") && unresolved.contains('5'),
            "unresolved row should list ALT indices: {unresolved}"
        );
        assert!(
            find(&expanded_pairs, "Event Authority").is_none(),
            "unresolved Event Authority must not be a named row: {expanded_pairs:?}"
        );

        // No per-byte / indexed explosion, and all values are charset-safe.
        for (label, value) in condensed_pairs.iter().chain(expanded_pairs.iter()) {
            assert!(!label.contains('['), "no indexed leaf labels: {label}");
            assert!(
                !value.contains('"') && !value.contains('\\'),
                "value must be charset-safe: {label} = {value}"
            );
        }
    }

    #[test]
    fn test_idl_known_addresses_resolves_constants_and_const_seed_pdas() {
        let map = idl_known_addresses();
        // Constant `address` constraints.
        assert_eq!(
            map.get("token_program").map(String::as_str),
            Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        );
        assert_eq!(
            map.get("system_program").map(String::as_str),
            Some("11111111111111111111111111111111")
        );
        // Const-seed PDA derived offline: find_program_address([b"__event_authority"], program).
        assert_eq!(
            map.get("event_authority").map(String::as_str),
            Some("8xeaWCsJYxRoudEZGJWURdfrtFhLYZz9b4iHJnW5tb3d")
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
