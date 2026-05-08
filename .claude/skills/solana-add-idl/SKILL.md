---
name: solana-add-idl
description: Add a new Solana program IDL-based visualizer preset. Fetches IDL on-chain or accepts user-provided IDL, then scaffolds config.rs, mod.rs, and registers the preset.
user-invocable: true
---

# Add Solana IDL Visualizer Preset

You are scaffolding a new Solana program visualizer preset from an Anchor IDL.

## Step 1: Gather Information

Ask the user for:
1. **Program address** (base58 Solana program ID)
2. **Human-readable name** (e.g. "Squads Multisig", "Marinade Finance", "Jupiter Swap")
3. **VisualizerKind** — one of: `Dex`, `Lending`, `StakingPools`, `Payments`

Derive these from the human name:
- `snake_name`: lowercase with underscores (e.g. `marinade_finance`)
- `PascalName`: PascalCase (e.g. `MarinadeFinance`)
- `SCREAMING_SNAKE`: uppercase with underscores (e.g. `MARINADE_FINANCE`)
- `display_name`: the human name as-is for display strings

## Step 2: Fetch the IDL

Try these in order:

### Option A: Local Anchor CLI
```bash
anchor idl fetch <PROGRAM_ID> --provider.cluster mainnet
```

### Option B: Docker container
If `anchor` is not installed locally, use the project's Anchor CLI container:

```bash
# Build the image if it doesn't exist
docker images -q anchor-cli | grep -q . || \
  docker build -t anchor-cli -f images/anchor-cli/Containerfile .

# Fetch the IDL
docker run --rm anchor-cli idl fetch <PROGRAM_ID> --provider.cluster mainnet
```

### Option C: User-provided IDL
If both methods fail, ask the user to provide the IDL via:
- A local file path
- A URL to fetch
- Pasted JSON

Save the IDL to: `src/chain_parsers/visualsign-solana/src/presets/{snake_name}/{snake_name}.json`

### Validation
The IDL JSON **must** have an `instructions` array. Verify this before proceeding. If it's missing, tell the user the IDL is invalid.

## Step 3: Scaffold the Preset

Create directory: `src/chain_parsers/visualsign-solana/src/presets/{snake_name}/`

### File: `config.rs`

```rust
use super::{SCREAMING_SNAKE}_PROGRAM_ID;
use crate::core::{SolanaIntegrationConfig, SolanaIntegrationConfigData};
use std::collections::HashMap;

pub struct {PascalName}Config;

impl SolanaIntegrationConfig for {PascalName}Config {
    fn new() -> Self {
        Self
    }

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

### File: `mod.rs`

Use `src/chain_parsers/visualsign-solana/src/presets/unknown_program/mod.rs` as the working reference for the IDL parsing pattern — it is the preset that actually exercises `parse_instruction_with_idl` against a runtime-supplied IDL today. Your preset is the same pattern with the IDL **embedded at compile time** and the program ID hardcoded.

Substitutions to make when adapting it:
- **Hardcode the program ID const** at the top of `mod.rs`:
  ```rust
  pub(crate) const {SCREAMING_SNAKE}_PROGRAM_ID: &str = "{base58_program_id}";
  ```
  This is what `config.rs` resolves via `use super::{SCREAMING_SNAKE}_PROGRAM_ID;`. See `presets/jupiter_swap/mod.rs` line ~24 for the canonical placement.
- **Embed the IDL** via `const IDL_JSON: &str = include_str!("{snake_name}.json");` and replace any runtime `idl_registry.get_idl(...)` lookup with `decode_idl_data(IDL_JSON)?`.
- **Rename the visualizer/config/static**: `UnknownProgramVisualizer` → `{PascalName}Visualizer`, `UnknownProgramConfig` → `{PascalName}Config`, `UNKNOWN_PROGRAM_CONFIG` → `{SCREAMING_SNAKE}_CONFIG`.
- **`kind()`** returns your chosen `VisualizerKind` variant: `VisualizerKind::{Kind}("{display_name}")`.
- **Drop the no-IDL fallback path** (`create_unknown_program_preview_layout`) — for an IDL-driven preset, return `Err(VisualSignError::DecodeError(...))` if parsing fails. Do not display raw bytes as a substitute.

**Building `named_accounts` — what the IDL gives you and what you build manually**

`parse_instruction_with_idl` returns a `SolanaParsedInstructionData` whose `named_accounts` field is empty by default. There is no `build_named_accounts` helper in `solana_parser`; you build the map yourself by matching the on-chain instruction's accounts against the IDL instruction's account list, in order. The reference loop is in `unknown_program::try_parse_with_idl` (search for `named_accounts` in that file). Copy that loop verbatim — it is the supported pattern.

**Required imports** (at top of module, NOT inside functions; only symbols that actually exist in the current dependency graph):
```rust
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::{PascalName}Config;
use solana_parser::{SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl};
use std::collections::HashMap;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};
```

**Required tests** (in `#[cfg(test)] mod tests`):
- `test_{snake_name}_idl_loads` — `decode_idl_data(IDL_JSON)` succeeds and `instructions` is non-empty
- `test_{snake_name}_idl_has_discriminators` — every instruction in the IDL has an 8-byte discriminator

Crash-safety against unknown discriminators / short data is **already covered**: by `tests/fuzz_idl_parsing.rs` (proptest, generative — exercises arbitrary discriminator/data combinations) and by `tests/surfpool_fuzz.rs::surfpool_preset_idls` (auto-iterates `PRESET_IDLS`). Do not duplicate those assertions in the preset's own test module.

## Step 4: Register in presets/mod.rs

Add `pub mod {snake_name};` to `src/chain_parsers/visualsign-solana/src/presets/mod.rs`.

**Keep entries in alphabetical order.** The existing entries are sorted — insert the new module in the correct position.

No other registration is needed for the visualizer itself. `build.rs` auto-discovers `{PascalName}Visualizer` from any directory under `src/presets/`.

## Step 5: Test coverage — what's auto-discovered, what isn't

You do **not** need to edit any test file. The harness picks up the new IDL by reflection:

- **`build.rs`** scans `src/presets/<name>/<name>.json` and emits `pub const PRESET_IDLS: &[(&str, &str)]` exposed from the library. The IDL JSON file you saved in Step 2 is the only input.
- **`tests/surfpool_fuzz.rs::surfpool_preset_idls`** iterates `PRESET_IDLS` and runs each through `run_idl_roundtrip` against a `surfpool` mainnet fork (decode IDL → build synthetic tx with the first instruction's discriminator → convert → assert non-empty payload). The new preset is exercised on every run with no test-file edit.
- **Proptest (`tests/fuzz_idl_parsing.rs`)** is *generative*. Strategies in `solana_parser_fuzz_core::proptest` synthesize arbitrary IDL shapes and feed them through `decode_idl_data` / `parse_instruction_with_idl`. New IDLs are covered structurally by the existing strategies — no per-IDL registration, ever.
- **cargo-fuzz (`fuzz/fuzz_targets/`)** runs against random byte streams through `transaction_string_to_visual_sign`. Same story: generative, no per-IDL registration.

Tests that *do* need hand-written assertions (and therefore can't be auto-discovered):

- **`tests/semantic_pipeline.rs`** — correctness assertions on parsed-field shape, label text, amounts, etc. These are program-specific. If the new preset's behavior matters in CI beyond "doesn't crash on a roundtrip," add a fixture-based test here. Otherwise the auto-roundtrip is enough.

CI: `surfpool_preset_idls` is `#[ignore]`. It runs when the PR carries the `surfpool` label (see `.github/workflows/surfpool-solana.yml`); local runs need `HELIUS_API_KEY`.

## Step 6: Code Quality

Follow these rules in all generated code:
- `use` statements at top of module, never inside functions
- Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
- Use `create_text_field` and `create_raw_data_field` from `visualsign::field_builders` — never construct field structs directly
- ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
- Rust edition 2024 on nightly

## Step 7: Verify

Run these commands and fix any issues:

```bash
cargo fmt -p visualsign-solana
cargo clippy -p visualsign-solana --all-targets -- -D warnings
cargo test -p visualsign-solana
make -C src test
```

All must pass before the task is complete.

To confirm the surfpool roundtrip picked up the new preset's IDL via auto-discovery, run:

```bash
cargo build -p visualsign-solana
grep -- '"{snake_name}"' src/chain_parsers/visualsign-solana/target/debug/build/visualsign-solana-*/out/preset_idls.rs
```

The `PRESET_IDLS` slice should contain a `("{snake_name}", include_str!(...))` entry. If it doesn't, the IDL JSON file is at the wrong path — `build.rs` looks for exactly `src/presets/{snake_name}/{snake_name}.json`.
