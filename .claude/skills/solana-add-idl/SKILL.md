---
name: solana-add-idl
description: Add a new Solana program IDL-based visualizer preset. Fetches IDL on-chain or accepts user-provided IDL, then scaffolds config.rs, mod.rs, and registers the preset.
user-invocable: true
---

# Add Solana IDL Visualizer Preset

You are orchestrating the scaffolding of a new Solana program visualizer preset.

## Mode Selection

- **Standard** (`/solana-add-idl`): gather inputs, dispatch one Sonnet subagent to scaffold the preset
- **Compare** (`/solana-add-idl compare`): gather inputs, dispatch Sonnet and Opus subagents in parallel to separate temp dirs, then diff their outputs to surface capability gaps

## Step 1: Gather Information

Ask the user for:
1. **Program address** (base58 Solana program ID)
2. **Human-readable name** (e.g. "Squads Multisig", "Marinade Finance", "Jupiter Swap")
3. **VisualizerKind** — one of: `Dex`, `Lending`, `StakingPools`, `Payments`

Derive from the human name:
- `snake_name`: lowercase with underscores (e.g. `marinade_finance`)
- `PascalName`: PascalCase (e.g. `MarinadeFinance`)
- `SCREAMING_SNAKE`: uppercase with underscores (e.g. `MARINADE_FINANCE`)
- `display_name`: human name as-is

## Step 2: Dispatch

### Standard mode

Dispatch one Agent with **`model: "sonnet"`**, substituting gathered values into the implementation prompt below.

### Compare mode

Dispatch two Agents **in parallel** — one with `model: "sonnet"`, one with `model: "opus"`. Give each a different output directory:

- Sonnet writes to: `/tmp/solana-add-idl-compare/sonnet/`
- Opus writes to: `/tmp/solana-add-idl-compare/opus/`

Both receive the same implementation prompt (below), with only the output directory differing.

After both complete, diff the two output directories:

```bash
diff -ru /tmp/solana-add-idl-compare/sonnet/ /tmp/solana-add-idl-compare/opus/
```

Present the diff to the user and summarize: what did Opus add or do differently? Are any of those patterns general enough to encode in this skill's instructions?

---

## Implementation Prompt (for subagents)

```
You are scaffolding a Solana program visualizer preset from an Anchor IDL.

## Inputs

- Program address: {PROGRAM_ID}
- snake_name: {snake_name}
- PascalName: {PascalName}
- SCREAMING_SNAKE: {SCREAMING_SNAKE}
- display_name: {display_name}
- VisualizerKind: {VisualizerKind}
- Output directory: {OUTPUT_DIR}   ← for compare mode; omit in standard mode (write to repo)

## Step A: Fetch the IDL

### Option A: Local Anchor CLI
```bash
anchor idl fetch {PROGRAM_ID} --provider.cluster mainnet
```

### Option B: Docker container
```bash
docker images -q anchor-cli | grep -q . || \
  docker build -t anchor-cli -f images/anchor-cli/Containerfile .
docker run --rm anchor-cli idl fetch {PROGRAM_ID} --provider.cluster mainnet
```

### Option C: User-provided
If both fail, ask the user for the IDL (file path, URL, or pasted JSON).

Save to: `{OUTPUT_DIR}src/chain_parsers/visualsign-solana/src/presets/{snake_name}/{snake_name}.json`

The IDL **must** have an `instructions` array. Stop and report if missing.

## Step B: Scaffold config.rs

```rust
use super::{SCREAMING_SNAKE}_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct {PascalName}Config;

impl SolanaIntegrationConfig for {PascalName}Config {
    fn new() -> Self { Self }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = HashMap::new();
            let mut instructions = HashMap::new();
            instructions.insert("*", vec!["*"]);
            programs.insert({SCREAMING_SNAKE}_PROGRAM_ID, instructions);
            SolanaIntegrationConfigData { programs }
        })
    }
}
```

## Step C: Scaffold mod.rs

Use `src/chain_parsers/visualsign-solana/src/presets/dflow_aggregator/mod.rs` as the
structural template. Make these substitutions:
- `DflowAggregator` / `dflow_aggregator` / `DFLOW_AGGREGATOR` → appropriate casing
- Program ID string → {PROGRAM_ID}
- `"DFlow Aggregator"` display strings → `{display_name}`
- `include_str!("dflow_aggregator.json")` → `include_str!("{snake_name}.json")`
- `kind()` → returns `{VisualizerKind}("{display_name}")`

**Wire-data context API** — at the top of `visualize_tx_commands`:
```rust
let program_id = context.resolve_program_id()?.to_string();
let accounts = context.resolve_accounts()?;
let data = context.data();
```

**Required imports** (at top of module, NOT inside functions):
```rust
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::{PascalName}Config;
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
```

`BTreeMap` (not `HashMap`) — keeps named-account order deterministic.

### Required: arg rendering helpers

Every preset **must** include these three helpers. They solve two constraints that
apply to all programs, not just this one:

**Constraint 1 — charset safety.** `SignablePayload::validate_charset` rejects `"`
and `\`. Compact JSON of any object contains quoted keys, which serialize to `\"`
and fail validation. Arg values must never emit `"` or `\`.

**Constraint 2 — field explosion.** Recursing into nested structs/arrays produces
one field per leaf. A `[u8; 32]` seed or a 76-byte opaque ID becomes 32 or 76
per-byte fields, burying the meaningful human-readable arguments.

```rust
/// Renders one top-level program-call argument as a single field value.
/// Objects/arrays: quote-free (no `"` or `\`), JSON-like.
/// All-byte arrays (every element 0–255): `0x`-prefixed hex string.
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

/// Strips `"`, `\`, control chars, and non-ASCII from strings/keys so the
/// charset-safe contract holds even for unexpected arg values.
fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
}

/// Returns `Some(0x…)` if every element is an integer in 0–255; `None` otherwise.
/// Empty arrays return `None` (caller falls back to bracketed list).
fn bytes_as_hex(items: &[serde_json::Value]) -> Option<String> {
    if items.is_empty() { return None; }
    let mut bytes = Vec::with_capacity(items.len());
    for item in items {
        let byte = item.as_u64().filter(|n| *n <= u8::MAX as u64)? as u8;
        bytes.push(byte);
    }
    Some(format!("0x{}", hex::encode(bytes)))
}
```

Use `format_arg_value` in `build_parsed_fields` for every program-call arg:
```rust
for (key, value) in &parsed.program_call_args {
    condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
}
// same in expanded_fields
```

For raw-data fields, pass `None` as the second arg of `create_raw_data_field`
unless you already have a precomputed hex string to reuse.

### Required tests

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_{snake_name}_idl_loads() { /* IDL loads and has instructions */ }

    #[test]
    fn test_{snake_name}_idl_has_discriminators() { /* every instruction has 8-byte discriminator */ }

    #[test]
    fn test_unknown_discriminator_returns_error() { /* garbage 9-byte data returns error */ }

    #[test]
    fn test_short_data_returns_error() { /* 3-byte data returns error */ }

    #[test]
    fn test_format_arg_value_is_charset_safe() {
        // No `"` or `\` in output for any input shape
        let nested = serde_json::json!({"key": "val\"ue", "arr": [1u8, 2, 3]});
        let rendered = format_arg_value(&nested);
        assert!(!rendered.contains('"') && !rendered.contains('\\'));
    }

    #[test]
    fn test_format_arg_value_no_field_explosion() {
        // A 32-element byte array becomes one hex string, not 32 fields
        let bytes: Vec<u8> = (0..32).collect();
        let val = serde_json::json!(bytes);
        let rendered = format_arg_value(&val);
        assert!(rendered.starts_with("0x"), "byte array should be hex: {rendered}");
        assert!(!rendered.contains(','), "byte array must not expand to list: {rendered}");
    }
}
```

## Step D: Register in presets/mod.rs

Add `pub mod {snake_name};` to
`src/chain_parsers/visualsign-solana/src/presets/mod.rs`, maintaining alphabetical
order.

No other registration needed — `build.rs` auto-discovers `{PascalName}Visualizer`.

## Step E: Code Quality

- `use` statements at top of module, never inside functions
- Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
- ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
- Rust edition 2024 on nightly

## Step F: Verify

```bash
cargo fmt -p visualsign-solana
cargo clippy -p visualsign-solana --all-targets -- -D warnings
cargo clippy -p visualsign-solana --features diagnostics --all-targets -- -D warnings
cargo test -p visualsign-solana
cargo test -p visualsign-solana --features diagnostics
make -C src test
```

Both feature configurations must pass. Report any failures before marking done.
```
