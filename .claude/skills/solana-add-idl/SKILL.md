---
name: solana-add-idl
description: Scaffold a structural IDL-driven Solana visualizer preset. Fetches the IDL (on-chain or user-provided), drops it into the preset directory, and writes a generic decoder. Registration is fully reflective via build.rs. Semantic refinement (domain labels, token resolution) is a follow-up workflow.
user-invocable: true
---

# Add Solana IDL Visualizer Preset

You are scaffolding a new Solana program visualizer preset from an Anchor IDL.

## Scope: what this skill produces and doesn't produce

This skill scaffolds a **structurally correct, semantically generic** preset.

What you get:

- Binary instruction decoded against the IDL via `parse_instruction_with_idl`
- Each on-chain account paired with its IDL-declared name
- Each instruction argument shown as a `text` field with the raw decoded value
- Auto-registered in `available_visualizers()` and `PRESET_IDLS` by `build.rs` reflection — no edits to `presets/mod.rs` or any test file
- Crash-safety auto-covered by `tests/fuzz_idl_parsing.rs` (proptest, generative), `fuzz/fuzz_targets/` (cargo-fuzz, generative), and `tests/surfpool_fuzz.rs::surfpool_preset_idls` (reflective)

What it deliberately does **not** produce:

- Domain-specific labels — e.g. `"Swap 1.5 USDC for 0.001 SOL"` rather than `in_token=Pubkey(...), amount_in=1500000, ...`
- Token metadata resolution (mint decimals, symbol lookups) — amounts render as raw integers
- Per-instruction display logic — every instruction goes through the same generic path
- Cross-instruction correlation (e.g. CPI inner-instruction handling)
- Account-role disambiguation beyond IDL parameter names
- Semantic correctness assertions in `tests/semantic_pipeline.rs` — those are program-specific and hand-written

The skill's output is the equivalent of a typed-decoder dump: correct, but not yet wallet-readable.

For a **fully semantic** preset to model after, read `src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/mod.rs`. It hand-rolls a `JupiterSwapInstruction` enum, resolves token metadata via `get_token_info`, and uses format strings like `"Swap {amount} {in_token} for {amount} {out_token}"`. That's the destination; this skill produces the starting point.

Semantic refinement is intended as a separate workflow (planned: `solana-refine-idl-preset` skill). Until that exists, contributors who want wallet-readable output extend the generated `mod.rs` by hand using `jupiter_swap` as the reference. See **Step 7: What's next** at the end of this skill.

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

## Step 4: Registration is automatic — nothing to edit

`presets/mod.rs` is generated by `build.rs` (it `include!`s a file that emits `#[path = ...] pub mod <name>;` per direct subdirectory of `src/presets/` containing a `mod.rs`). Drop your preset directory in place; the next build picks it up.

`build.rs` also discovers your `{PascalName}Visualizer` for `available_visualizers()` and (because Step 2 saved an IDL JSON) adds an entry to `PRESET_IDLS`.

Skip ahead — there's no edit to make in this step.

## Step 5: Test coverage — what's auto-discovered, what's program-specific

You do **not** need to edit any test file for crash-safety coverage. The harness picks up the new IDL by reflection:

- **`build.rs`** emits `pub const PRESET_IDLS: &[(&str, &str)]` from `src/presets/<name>/<name>.json` (saved in Step 2). The slice is re-exported from the library.
- **`tests/surfpool_fuzz.rs::surfpool_preset_idls`** iterates `PRESET_IDLS` and runs each through `run_idl_roundtrip` against a `surfpool` mainnet fork (decode IDL → build synthetic tx with the first instruction's discriminator → convert → assert non-empty payload). Picked up on every run with no test-file edit.
- **Proptest (`tests/fuzz_idl_parsing.rs`)** is *generative*: strategies in `solana_parser_fuzz_core::proptest` synthesize arbitrary IDL shapes and feed them through `decode_idl_data` / `parse_instruction_with_idl`. Covers your IDL structurally without registration.
- **cargo-fuzz (`fuzz/fuzz_targets/`)** is also *generative* — random byte streams through `transaction_string_to_visual_sign`. No per-IDL registration.

CI: `surfpool_preset_idls` is `#[ignore]`. It runs when the PR carries the `surfpool` label (see `.github/workflows/surfpool-solana.yml`); local runs need `HELIUS_API_KEY`.

### Semantic correctness is NOT auto-covered

The auto-roundtrip only asserts the converter doesn't crash. It does **not** assert the displayed fields look correct semantically — that's by design (this skill produces a generic decoder, not a semantic one).

If the preset needs CI-level semantic guarantees (specific label text, amount formatting, multi-instruction flows, fixture-based snapshot expectations), add a hand-written test in `tests/semantic_pipeline.rs` modelled after the existing `JUPITER_IDL` / `RAYDIUM_IDL` blocks. Otherwise, ship as-is — semantic refinement is a separate workflow (see Step 8).

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

## Step 8: What's next — semantic refinement (optional, follow-up)

Your preset compiles, registers, and survives a roundtrip. A wallet user signing one of these transactions will, however, see raw arg names and integer values, not a recognizable summary. The skill's scope ends here. To make the preset wallet-readable, three options:

1. **Ship as-is.** For low-traffic programs or where structural display is enough, this is acceptable — the new preset is strictly better than the `unknown_program` fallback.

2. **Hand-extend the generated `mod.rs`**, modelled after `presets/jupiter_swap/mod.rs`. The patterns to copy:
   - Replace the wildcard `"*": ["*"]` in `config.rs` with explicit instruction names so each instruction can be dispatched separately.
   - Introduce a `{PascalName}Instruction` enum with one variant per IDL instruction you care about. See `JupiterSwapInstruction` for the shape (named fields like `in_token`, `out_token`, `slippage_bps`).
   - Add a `parse_{snake_name}_instruction` helper that dispatches on the 8-byte discriminator and decodes args into the enum.
   - Add a `format_{snake_name}_instruction` helper that turns the enum into a human string. Use `get_token_info` from `crate::utils` to resolve mint decimals and symbols for amount fields.
   - Replace generic `create_text_field` calls with semantic ones — `create_amount_field` for token quantities, `create_address_field` for accounts you want clickable in the UI.
   - Add a fixture test in `tests/semantic_pipeline.rs` asserting the formatted output for one or two real on-chain transactions.

3. **Wait for `solana-refine-idl-preset`** — a planned follow-up skill that automates the structural-to-semantic transition. Tracked as future work; not yet available.

Until option 3 exists, option 2 is the path. The structural decoder this skill produced is the scaffolding the semantic layer goes on top of, not a replacement for it.
