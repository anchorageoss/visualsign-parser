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
use std::collections::BTreeMap;

pub struct {PascalName}Config;

impl SolanaIntegrationConfig for {PascalName}Config {
    fn new() -> Self {
        Self
    }

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

### File: `mod.rs`

Use the dflow_aggregator preset as a template: `src/chain_parsers/visualsign-solana/src/presets/dflow_aggregator/mod.rs`

Read that file for the exact structure, then generate a generic version with these substitutions:
- Replace `DflowAggregator` / `dflow_aggregator` / `DFLOW_AGGREGATOR` with the appropriate casing of the new program name
- Replace the program ID string with the new program address
- Replace `"DFlow Aggregator"` display strings with `{display_name}`
- Replace IDL file reference: `include_str!("{snake_name}.json")`
- Keep the `kind()` method returning the user's chosen `VisualizerKind` variant with `display_name` as the `&'static str` argument

**Generic IDL pattern only:**
- The generic scaffold uses the three helpers `dflow_aggregator` defines: `build_named_accounts`, `build_parsed_fields`, and `build_fallback_fields`. All three work with any IDL.
- Two additional helpers — `append_raw_data` (for byte-blob args) and `format_arg_value` (for custom scalar rendering) — are not present in `dflow_aggregator`. Add them when the target IDL needs them, copying the pattern from another preset such as `kamino_vault` or `jupiter_earn`.
- The parse function should: check `data.len() < 8`, load IDL, call `parse_instruction_with_idl`, call `build_named_accounts`, return a struct with parsed data + named accounts

**Prerequisite:** `InstructionView` must be present in `crate::core` (introduced in the v0+ALT graceful-degradation refactor). If the codebase predates that change, `InstructionView` will not resolve — check `core/mod.rs` before proceeding.

**Visualizer body must use `InstructionView`.** At the top of `visualize_tx_commands`:
```rust
let view = InstructionView::from_context(context);
let data = context.data();
```

`InstructionView::from_context` is infallible: every account index resolves to either
its base58 pubkey string or `"unresolved(N)"` for ALT/OOB indices. This is the
correct pattern for all IDL-based presets — v0 transactions that use Address Lookup
Tables have ALT-referenced accounts that can never be resolved from `account_keys`,
and showing `"unresolved(N)"` is strictly better than aborting the whole transaction
display.

**Security asymmetry — do not use `resolve_accounts()?` in IDL presets.**
The old `resolve_accounts()?` pattern (and the newer `context.resolve_accounts()?`)
fails closed on unresolved accounts. It is intentionally kept only in `token_2022`,
where the account fields (mint, pause_authority, mint_authority, freeze_authority)
are themselves the security-critical content — showing `"unresolved(N)"` in place of
an authority would let a user approve an instruction whose authority they cannot
verify. For IDL presets, the accounts are display labels, not security gates, so
graceful degradation is correct. Do not import or call `resolve_accounts` in new
presets.

Use `view.program_id` for the program ID string and `view.accounts` (a `Vec<String>`)
wherever the old code passed `&[AccountMeta]`. Use `context.instruction_index()` for
"Instruction N" labels.

**Required imports** (at top of module, NOT inside functions):
```rust
use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
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

`BTreeMap` (not `HashMap`) keeps the rendered named-accounts order deterministic.
Do not import `solana_sdk::instruction::AccountMeta` — the new pattern passes
`&[String]` everywhere `AccountMeta` slices used to go.

**Required tests** (in `#[cfg(test)] mod tests`):
- `test_{snake_name}_idl_loads` — IDL loads and has instructions
- `test_{snake_name}_idl_has_discriminators` — every instruction has an 8-byte discriminator
- `test_unknown_discriminator_returns_error` — garbage 9-byte data returns error
- `test_short_data_returns_error` — 3-byte data returns error

Test helpers that supply dummy accounts must use `Vec<String>`, not `Vec<AccountMeta>`:
```rust
fn dummy_account_strings(n: usize) -> Vec<String> {
    use solana_sdk::pubkey::Pubkey;
    (0..n).map(|_| Pubkey::new_unique().to_string()).collect()
}
```

## Step 4: No manual registration needed

`src/chain_parsers/visualsign-solana/src/presets/mod.rs` is **fully auto-generated** by `build.rs` — its header says "DO NOT edit this file". Do not add a `pub mod` line there.

`build.rs` auto-discovers `{PascalName}Visualizer` from any directory under `src/presets/`. Creating the directory with `mod.rs` and `config.rs` is all that's needed.

## Step 5: Code Quality

Follow these rules in all generated code:
- `use` statements at top of module, never inside functions
- Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
- Use `create_text_field` and `create_raw_data_field` from `visualsign::field_builders` — never construct field structs directly
- For raw-data fields, pass `None` as the second arg of `create_raw_data_field` unless you already have a precomputed hex string to reuse (e.g. one you built for `fallback_text`). Do not call `hex::encode(data)` solely to populate this arg — the helper falls back to the same lowercase byte-by-byte hex on `None`.
- ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
- Rust edition 2024 on nightly

## Step 6: Verify

Run these commands and fix any issues:

```bash
cargo fmt -p visualsign-solana
cargo clippy -p visualsign-solana --all-targets -- -D warnings
cargo clippy -p visualsign-solana --features diagnostics --all-targets -- -D warnings
cargo test -p visualsign-solana
cargo test -p visualsign-solana --features diagnostics
make -C src test
```

All must pass before the task is complete. Both feature configurations
(diagnostics on and off) need to compile and test cleanly because parser_app
builds without `diagnostics` while parser_cli builds with it.

## When validating the skill via preset regeneration

If this skill is being run as part of a regeneration test (delete existing preset → regenerate → verify), post a PR comment for each finding encountered — compile errors, test failures, generated code that doesn't match expectations. This creates a paper trail for reviewers and drives iterative skill improvements within the same PR.

```bash
gh pr comment <PR_NUMBER> --body "**Finding: <short title>**

<description of what went wrong and what the correct behavior should be>"
```

Each comment should name the specific step or template section that needs updating (e.g. "Step 4", "config.rs template", "Required imports list"). After all findings are addressed and CI passes, update the skill accordingly in the same PR.

**Do not aggregate findings into the PR description.** Post them as individual comments as they surface so reviewers can track resolution per finding.
