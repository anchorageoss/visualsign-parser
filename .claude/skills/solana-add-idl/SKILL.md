---
name: solana-add-idl
description: Add a new Solana program IDL-based visualizer preset. Fetches IDL on-chain or accepts user-provided IDL, then scaffolds config.rs, mod.rs, and registers the preset.
user-invocable: true
---

# Add Solana IDL Visualizer Preset

You are orchestrating the scaffolding of a new Solana program visualizer preset.

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

## Step 2: Dispatch Implementation Subagent

Once you have all inputs, dispatch an Agent with **`model: "sonnet"`**. Substitute the gathered values into the prompt below before dispatching.

```
Agent:
  description: "Scaffold Solana IDL preset for {display_name}"
  model: "sonnet"
  prompt: |
    You are scaffolding a new Solana program visualizer preset from an Anchor IDL.

    ## Inputs

    - Program address: {PROGRAM_ID}
    - snake_name: {snake_name}
    - PascalName: {PascalName}
    - SCREAMING_SNAKE: {SCREAMING_SNAKE}
    - display_name: {display_name}
    - VisualizerKind: {VisualizerKind}

    ## Step A: Fetch the IDL

    Try these in order:

    ### Option A: Local Anchor CLI
    ```bash
    anchor idl fetch {PROGRAM_ID} --provider.cluster mainnet
    ```

    ### Option B: Docker container
    If `anchor` is not installed locally, use the project's Anchor CLI container:

    ```bash
    docker images -q anchor-cli | grep -q . || \
      docker build -t anchor-cli -f images/anchor-cli/Containerfile .
    docker run --rm anchor-cli idl fetch {PROGRAM_ID} --provider.cluster mainnet
    ```

    ### Option C: User-provided IDL
    If both methods fail, tell the user and ask them to provide the IDL via a local
    file path, a URL, or pasted JSON.

    Save the IDL to:
    `src/chain_parsers/visualsign-solana/src/presets/{snake_name}/{snake_name}.json`

    The IDL JSON **must** have an `instructions` array. Verify this before proceeding.
    If it's missing, stop and report the IDL as invalid.

    ## Step B: Scaffold the Preset

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

    Use the dflow_aggregator preset as a template:
    `src/chain_parsers/visualsign-solana/src/presets/dflow_aggregator/mod.rs`

    Read that file for the exact structure, then generate a generic version with:
    - Replace `DflowAggregator` / `dflow_aggregator` / `DFLOW_AGGREGATOR` with the
      appropriate casing of the new program name
    - Replace the program ID string with {PROGRAM_ID}
    - Replace `"DFlow Aggregator"` display strings with `{display_name}`
    - Replace IDL file reference: `include_str!("{snake_name}.json")`
    - Keep the `kind()` method returning `{VisualizerKind}` with `{display_name}` as
      the `&'static str` argument

    **Generic IDL pattern only:**
    The scaffold uses `build_named_accounts`, `build_parsed_fields`, and
    `build_fallback_fields` — all three work with any IDL. `append_raw_data` and
    `format_arg_value` are not in `dflow_aggregator`; add them only if the target IDL
    needs them, copying from `kamino_vault` or `jupiter_earn`.

    The parse function must: check `data.len() < 8`, load IDL, call
    `parse_instruction_with_idl`, call `build_named_accounts`, return a struct with
    parsed data + named accounts.

    **Visualizer body must use the wire-data context API.** At the top of
    `visualize_tx_commands`:
    ```rust
    let program_id = context.resolve_program_id()?.to_string();
    let accounts = context.resolve_accounts()?;
    let data = context.data();
    ```
    Use `context.instruction_index()` for any "Instruction N" labels.

    **Required imports** (at top of module, NOT inside functions):
    ```rust
    use crate::core::{
        InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
    };
    use config::{PascalName}Config;
    use solana_parser::{
        Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
    };
    use solana_sdk::instruction::AccountMeta;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    use visualsign::errors::VisualSignError;
    use visualsign::field_builders::{create_raw_data_field, create_text_field};
    use visualsign::{
        AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
        SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    };
    ```

    `BTreeMap` (not `HashMap`) keeps named-accounts order deterministic.

    **Required tests** (in `#[cfg(test)] mod tests`):
    - `test_{snake_name}_idl_loads` — IDL loads and has instructions
    - `test_{snake_name}_idl_has_discriminators` — every instruction has an 8-byte discriminator
    - `test_unknown_discriminator_returns_error` — garbage 9-byte data returns error
    - `test_short_data_returns_error` — 3-byte data returns error

    ## Step C: Register in presets/mod.rs

    Add `pub mod {snake_name};` to
    `src/chain_parsers/visualsign-solana/src/presets/mod.rs`.

    Keep entries in alphabetical order.

    No other registration is needed. `build.rs` auto-discovers `{PascalName}Visualizer`
    from any directory under `src/presets/`.

    ## Step D: Code Quality

    - `use` statements at top of module, never inside functions
    - Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
    - Use `create_text_field` and `create_raw_data_field` from `visualsign::field_builders`
    - For raw-data fields, pass `None` as the second arg of `create_raw_data_field` unless
      you already have a precomputed hex string to reuse. Do not call `hex::encode(data)`
      solely to populate this arg.
    - ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
    - Rust edition 2024 on nightly

    ## Step E: Verify

    Run these commands and fix any issues before reporting done:

    ```bash
    cargo fmt -p visualsign-solana
    cargo clippy -p visualsign-solana --all-targets -- -D warnings
    cargo clippy -p visualsign-solana --features diagnostics --all-targets -- -D warnings
    cargo test -p visualsign-solana
    cargo test -p visualsign-solana --features diagnostics
    make -C src test
    ```

    Both feature configurations (diagnostics on and off) must compile and test cleanly.
```
