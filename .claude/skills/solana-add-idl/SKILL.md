---
name: solana-add-idl
description: Add a new Solana program IDL-based visualizer preset. Fetches IDL on-chain or accepts user-provided IDL, then scaffolds config.rs, mod.rs, and registers the preset.
user-invocable: true
---

# Add Solana IDL Visualizer Preset

## When to use this skill

**Adding a new program** — follow Steps 1–7 linearly. The output is a new preset directory with `config.rs`, `mod.rs`, and the bundled IDL JSON.

**Validating or improving the skill itself** — use the "Validation Mode" section at the bottom. Delete one or more existing presets, regenerate them with this skill, surface findings as PR comments, fix the skill, and repeat until clean. Any manual tweak needed to make generated output compile or pass tests belongs back in the skill template, not just in the file.

## Step 1: Gather Information

Ask the user for:
1. **Program address** (base58 Solana program ID)
2. **Human-readable name** (e.g. "Squads Multisig", "Marinade Finance", "Jupiter Swap")
3. **VisualizerKind** — one of: `Dex`, `Lending`, `StakingPools`, `Payments`
4. **Subtitle** — the short label shown under the instruction title in the preview layout. Default is empty (corpus-wide convention for IDL presets); ask the user if they want a custom string.

Derive these from the human name:
- `snake_name`: lowercase with underscores (e.g. `marinade_finance`)
- `PascalName`: PascalCase (e.g. `MarinadeFinance`)
- `SCREAMING_SNAKE`: uppercase with underscores (e.g. `MARINADE_FINANCE`)
- `display_name`: the human name as-is for display strings
- `subtitle_text`: the user-provided subtitle, or `String::new()` if none given

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

`build_named_accounts` has this exact signature — argument order matters:
```rust
fn build_named_accounts(
    data: &[u8],
    idl: &Idl,
    accounts: &[String],
) -> (BTreeMap<String, String>, Vec<String>)
```
The first return value is named accounts (IDL account name → pubkey string); the second is extra accounts (accounts beyond what the IDL instruction defines), rendered as `"Remaining Account N"` in the expanded view.

**Expanded view must include a `Program` display-name field.** The first field in `expanded_fields` must be the human-readable program name, before `Program ID`:
```rust
expanded_fields.push(create_text_field("Program", "{display_name}")?);
expanded_fields.push(create_text_field("Program ID", program_id)?);
```
The condensed view already has `Program: {display_name}` — expanded must match so the user sees the program name in both the summary and the detail view, not just a raw address.

**Prerequisite:** `InstructionView` must be present in `crate::core` (introduced in the v0+ALT graceful-degradation refactor). If the codebase predates that change, `InstructionView` will not resolve — check `core/mod.rs` before proceeding.

**Subtitle field uses the value gathered in Step 1.** In the `preview_layout` construction inside `visualize_tx_commands`, emit one of these two forms depending on what the user provided:

With a custom subtitle:
```rust
subtitle: Some(SignablePayloadFieldTextV2 {
    text: "My Custom Subtitle".to_string(),
}),
```
With no subtitle (default):
```rust
subtitle: Some(SignablePayloadFieldTextV2 {
    text: String::new(),
}),
```

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
- `test_push_arg_fields_renders_scalars` — string/number/bool/null each produce one `TextV2` field
- `test_push_arg_fields_recurses_into_array_with_indexed_labels` — `key[0]`, `key[1]` … labels
- `test_push_arg_fields_recurses_into_object_with_dotted_labels` — `parent.child` labels
- `test_push_arg_fields_renders_empty_collections` — empty array/object render as `[]`/`{}`
- `test_build_named_accounts_surfaces_extra_accounts` — pick an IDL instruction with N named accounts, provide N+2 account strings, assert `named.len()==N` and `extra.len()==2`
- `test_remaining_account_label_is_human_readable` — extra accounts appear as `"Remaining Account 1"`, `"Remaining Account 2"`, etc. in expanded view

Test helpers — add both to the test module:
```rust
fn dummy_account_strings(n: usize) -> Vec<String> {
    use solana_sdk::pubkey::Pubkey;
    (0..n).map(|_| Pubkey::new_unique().to_string()).collect()
}

fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
    match &field.signable_payload_field {
        SignablePayloadField::TextV2 { common, text_v2 } => {
            (common.label.clone(), text_v2.text.clone())
        }
        other => panic!("expected TextV2 field, got {other:?}"),
    }
}
```

The `push_arg_fields` and `build_named_accounts` tests require these imports at the top of the test module:
```rust
use serde_json::json;
use solana_parser::IdlSource;
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

## Step 7: Compare against the prior implementation

Skip this step for programs with no prior implementation — there is nothing to diff.

When regenerating an existing preset, diff the generated output against the version that existed before deletion (the base branch, one commit before the delete commit). Look for:

- **Structural regressions**: fields missing from condensed or expanded, fallback path removed, IDL caching changed from `OnceLock` to per-call
- **Behaviour changes**: field labels renamed, arg rendering changed (flat `format_arg_value` vs recursive `push_arg_fields`), subtitle content changed, extra accounts dropped or added
- **Improvements**: the generated output may be strictly better than what was hand-written (e.g. adding `Remaining Account N` handling, using recursive arg rendering, consistent `dummy_account_strings` test helpers) — call these out explicitly as they validate the skill is producing higher-quality output than the predecessor

```bash
# diff generated output against the pre-deletion version
git diff <base-branch>..<current-branch> -- \
  src/chain_parsers/visualsign-solana/src/presets/{snake_name}/mod.rs
```

Post a PR comment summarising observations — improvements, neutral changes, and any concerns — regardless of whether anything needs fixing. This creates a record that the regeneration was reviewed and the output was understood, not just "it compiles."

```bash
gh pr comment <PR_NUMBER> --body "**Comparison: generated vs prior {display_name} implementation**

Improvements:
- <list>

Neutral changes:
- <list>

Concerns / open questions:
- <list or 'none'>"
```

If the comparison surfaces a missing field or a behaviour regression that should always be present in new presets, fix the skill template and regenerate before marking the round complete.

## Validation Mode

The regeneration test is an iterative loop, not a one-shot pass. Expect multiple cycles:

```
delete preset implementations
  → run skill → cargo build/test
    → finding? → post PR comment + fix skill → repeat
      → all green → done
```

**Per finding:**

1. Post a PR comment immediately — don't batch findings:
   ```bash
   gh pr comment <PR_NUMBER> --body "**Finding: <short title>**

   <what went wrong and what the correct behavior should be>"
   ```
   Name the specific step or template section affected (e.g. "Step 4", "config.rs template", "Required imports list").

2. Fix the skill in the same PR commit. The skill and the generated output evolve together — a finding that isn't fixed in the skill will recur the next time someone runs it.

3. Re-delete and regenerate the affected preset to confirm the fix produces clean output before moving on.

**Do not aggregate findings into the PR description.** Post them as individual comments as they surface so reviewers can track each one's resolution independently.

**The skill is the artifact.** Generated preset files are evidence that the skill works; the skill file is what ships. If the generated output needed a manual tweak to compile or pass tests, that tweak belongs in the skill — not just in the generated file.
