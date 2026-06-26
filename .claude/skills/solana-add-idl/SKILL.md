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
use std::collections::BTreeMap;

pub struct {PascalName}Config;

impl SolanaIntegrationConfig for {PascalName}Config {
    fn new() -> Self { Self }

    fn data(&self) -> &SolanaIntegrationConfigData {
        static DATA: std::sync::OnceLock<SolanaIntegrationConfigData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut programs = BTreeMap::new();
            let mut instructions = BTreeMap::new();
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
- `"DFlow Aggregator"` display strings → the constant `{SCREAMING_SNAKE}_DISPLAY_NAME` (define it once as `const {SCREAMING_SNAKE}_DISPLAY_NAME: &str = "{display_name}";`)
- `include_str!("dflow_aggregator.json")` → `include_str!("{snake_name}.json")`
- `kind()` → returns `{VisualizerKind}("{display_name}")`

**Account resolution** — use `InstructionView`, not `resolve_accounts()`:
```rust
let view = InstructionView::from_context(context);
let data = context.data();
```

`InstructionView::from_context` is infallible and degrades gracefully on v0+ALT
transactions (unresolvable account indices become empty strings rather than
aborting). `context.resolve_accounts()?` aborts on those — do not use it for IDL
presets. Use `view.program_id` for the program ID string and `view.accounts` (a
`Vec<String>`) wherever account pubkeys are needed. Inner helpers take
`accounts: &[String]`.

**Required imports** (at top of module, NOT inside functions):
```rust
use crate::core::{
    format_arg_value, InstructionView, InstructionVisualizer, SolanaIntegrationConfig,
    VisualizerContext, VisualizerKind,
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

### Arg rendering: import from crate::core

`format_arg_value` is already included via the import above. Do **not** copy the
function body into the preset — the implementation lives in
`src/chain_parsers/visualsign-solana/src/core/arg_rendering.rs`.

It enforces two constraints that apply to every program:

**Constraint 1 — charset safety.** `SignablePayload::validate_charset` rejects `"`
and `\`. Compact JSON of any object emits `\"` for keys and fails validation.
`format_arg_value` never emits `"` or `\`.

**Constraint 2 — field explosion.** A `[u8; 32]` seed or 76-byte opaque ID would
produce 32 or 76 per-byte fields if recursed into. `format_arg_value` collapses
all-byte arrays into a single `0x`-prefixed hex string.

Use it in `build_parsed_fields` for every program-call arg:
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
    fn test_{snake_name}_idl_has_discriminators() {
        // IdlInstruction.discriminator is Option<Vec<u8>>
        let idl_json = include_str!("{snake_name}.json");
        let idl: Idl = serde_json::from_str(idl_json).unwrap();
        for ix in &idl.instructions {
            let len = ix.discriminator.as_ref().map(Vec::len).unwrap_or(0);
            assert_eq!(len, 8, "instruction {} missing 8-byte discriminator", ix.name);
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() { /* garbage 9-byte data returns error */ }

    #[test]
    fn test_short_data_returns_error() { /* 3-byte data returns error */ }
}
```

## Step D: Registration

No manual registration needed. `build.rs` auto-discovers `{PascalName}Visualizer`
from any directory under `src/presets/` — do not edit `presets/mod.rs`, it is
generated.

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
