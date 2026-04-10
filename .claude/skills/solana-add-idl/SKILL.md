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

Use the squads_multisig preset as a template: `src/chain_parsers/visualsign-solana/src/presets/squads_multisig/mod.rs`

Read that file for the exact structure, then generate a generic version with these substitutions:
- Replace `SquadsMultisig` / `squads_multisig` / `SQUADS_MULTISIG` with the appropriate casing of the new program name
- Replace the program ID string with the new program address
- Replace `"Squads Multisig"` display strings with `{display_name}`
- Replace IDL file reference: `include_str!("{snake_name}.json")`
- Keep the `kind()` method returning the user's chosen `VisualizerKind` variant with `display_name` as the `&'static str` argument

**Important — generic IDL pattern only:**
- DO NOT copy any squads-specific code (e.g. `VaultTransactionMessage` decoding, inner instruction handling)
- The generic scaffold uses `build_named_accounts`, `build_parsed_fields`, `build_fallback_fields`, `append_raw_data`, `format_arg_value` — all of which work with any IDL
- The parse function should: check `data.len() < 8`, load IDL, call `parse_instruction_with_idl`, call `build_named_accounts`, return a struct with parsed data + named accounts

**Required imports** (at top of module, NOT inside functions):
```rust
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::{PascalName}Config;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::HashMap;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};
```

**Required tests** (in `#[cfg(test)] mod tests`):
- `test_{snake_name}_idl_loads` — IDL loads and has instructions
- `test_{snake_name}_idl_has_discriminators` — every instruction has an 8-byte discriminator
- `test_unknown_discriminator_returns_error` — garbage 9-byte data returns error
- `test_short_data_returns_error` — 3-byte data returns error

## Step 4: Register in presets/mod.rs

Add `pub mod {snake_name};` to `src/chain_parsers/visualsign-solana/src/presets/mod.rs`.

**Keep entries in alphabetical order.** The existing entries are sorted — insert the new module in the correct position.

No other registration is needed. `build.rs` auto-discovers `{PascalName}Visualizer` from any directory under `src/presets/`.

## Step 5: Code Quality

Follow these rules in all generated code:
- `use` statements at top of module, never inside functions
- Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
- Use `create_text_field` and `create_raw_data_field` from `visualsign::field_builders` — never construct field structs directly
- ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
- Rust edition 2024 on nightly

## Step 6: Verify

Run these commands and fix any issues:

```bash
cargo fmt -p visualsign-solana
cargo clippy -p visualsign-solana --all-targets -- -D warnings
cargo test -p visualsign-solana
make -C src test
```

All must pass before the task is complete.
