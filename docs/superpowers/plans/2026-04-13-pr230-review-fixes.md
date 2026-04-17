# PR #230 Review Fixes: VisualizerContext Refactor + Diagnostic Model

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address all open review comments on PR #230 by refactoring `VisualizerContext` to work directly with transaction wire data (`&CompiledInstruction` + `&[Pubkey]`), eliminating instruction skipping/filtering, and simplifying the diagnostic model.

**Architecture:** Currently, compiled instructions are eagerly resolved into owned `solana_sdk::Instruction` copies, with OOB indices causing instructions to be skipped. This creates a filtered vec whose positions diverge from original instruction indices — the root cause of the critical index mismatch bug. The fix: `VisualizerContext` holds references to the transaction's own data (`&CompiledInstruction` + `&[Pubkey]`), resolving indices lazily via helper methods. No instructions are ever skipped. Diagnostics are derived from `None` returns (inaccessible indices), with severity controlled by `LintConfig`.

**Tech Stack:** Rust (nightly 1.88, edition 2024), solana-sdk, serde, visualsign workspace

**Branch:** `shahankhatch/228-lint-diagnostics`

**Review comments addressed:**
- Critical: index mismatch in instructions.rs and v0.rs (#1, #2) — eliminated by removing filtered vec
- High: misleading ok-diagnostic (#3) — `oob_account_index_in_skipped_instruction` rule removed entirely
- High: V0 behavioral change (#4) — verified, no dependents
- High: text/human untested (#5) — restored
- High: shallow diagnostic assertions (#6) — strengthened
- Medium: .unwrap() in serialize (#7) — fixed
- Medium: &str instead of Severity (#8) — fixed
- Medium: unregistered rule (#9) — documented
- Medium: code duplication (#10) — eliminated by shared diagnostic scan
- Low: LintConfig::default() twice (#11) — threaded through
- Low: doc comment placement (#12) — fixed

---

## File Structure

**Core changes (VisualizerContext + traits):**
- Modify: `src/chain_parsers/visualsign-solana/src/core/mod.rs` — `VisualizerContext` struct, `InstructionVisualizer` trait, `SolanaIntegrationConfig` trait, `visualize_with_any`

**Preset updates (mechanical — change data access pattern):**
- Modify: `src/chain_parsers/visualsign-solana/src/presets/system/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/compute_budget/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/associated_token_account/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/stakepool/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/token_2022/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/swig_wallet/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/unknown_program/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/unknown_program/config.rs`

**Instruction processing refactor (no more skipping):**
- Modify: `src/chain_parsers/visualsign-solana/src/core/instructions.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/core/txtypes/v0.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/core/visualsign.rs`

**Independent review fixes:**
- Modify: `src/visualsign/src/lib.rs` — Diagnostic serialize impl
- Modify: `src/visualsign/src/field_builders.rs` — Severity enum parameter
- Modify: `src/parser/cli/tests/cli_test.rs` — test updates
- Modify/Create: `src/parser/cli/tests/fixtures/` — fixture files

---

### Task 1: Refactor VisualizerContext to hold &CompiledInstruction + &[Pubkey]

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/core/mod.rs`

**Context:** `VisualizerContext` currently holds `instruction_index: usize` and `instructions: &'a Vec<Instruction>`, using the index to look up the current instruction. This is the root cause of the critical index mismatch bug. The new context holds `&CompiledInstruction` + `&[Pubkey]` directly — no index, no vec, no copies. Resolution happens lazily via helper methods.

- [ ] **Step 1: Write tests for the new VisualizerContext helper methods**

Add to the test module at the bottom of `mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::instruction::CompiledInstruction;
    use solana_sdk::pubkey::Pubkey;

    fn make_context<'a>(
        ci: &'a CompiledInstruction,
        account_keys: &'a [Pubkey],
        sender: &'a SolanaAccount,
        idl_registry: &'a crate::idl::IdlRegistry,
    ) -> VisualizerContext<'a> {
        VisualizerContext::new(sender, ci, account_keys, idl_registry)
    }

    #[test]
    fn test_program_id_resolved() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 1,
            accounts: vec![0],
            data: vec![0xAA, 0xBB],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.program_id(), Some(&keys[1]));
    }

    #[test]
    fn test_program_id_inaccessible() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 99,
            accounts: vec![],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.program_id(), None);
    }

    #[test]
    fn test_account_resolved_and_inaccessible() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 1,
            accounts: vec![0, 50], // 0 valid, 50 OOB
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.account(0), Some(&keys[0]));
        assert_eq!(ctx.account(1), None); // index 50 is OOB
        assert_eq!(ctx.account(99), None); // position doesn't exist
    }

    #[test]
    fn test_data_returns_instruction_bytes() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: vec![0xDE, 0xAD],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.data(), &[0xDE, 0xAD]);
    }

    #[test]
    fn test_num_accounts() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![0, 1, 0],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.num_accounts(), 3);
    }

    #[test]
    fn test_raw_account_index() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![0, 77],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = make_context(&ci, &keys, &sender, &registry);
        assert_eq!(ctx.raw_account_index(0), Some(0u8));
        assert_eq!(ctx.raw_account_index(1), Some(77u8));
        assert_eq!(ctx.raw_account_index(5), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (struct doesn't exist yet)**

Run: `cargo test -p visualsign-solana --lib core::tests 2>&1`

Expected: Compilation failure — new methods don't exist.

- [ ] **Step 3: Replace VisualizerContext struct and implement helper methods**

Replace the entire `VisualizerContext` definition and impl block in `mod.rs`:

```rust
/// Context for visualizing a Solana instruction.
///
/// Holds references to the transaction's wire data — no copies.
/// Resolution of compiled instruction indices to pubkeys happens
/// lazily via helper methods. `None` means the index is inaccessible
/// (out of bounds, or references a lookup table account in v0).
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    /// The address sending the transaction.
    sender: &'a SolanaAccount,
    /// The compiled instruction from the transaction message.
    compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
    /// All account keys from the transaction message.
    account_keys: &'a [solana_sdk::pubkey::Pubkey],
    /// IDL registry for parsing unknown programs with Anchor IDLs.
    idl_registry: &'a crate::idl::IdlRegistry,
}

impl<'a> VisualizerContext<'a> {
    /// Creates a new `VisualizerContext`.
    pub fn new(
        sender: &'a SolanaAccount,
        compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
        account_keys: &'a [solana_sdk::pubkey::Pubkey],
        idl_registry: &'a crate::idl::IdlRegistry,
    ) -> Self {
        Self {
            sender,
            compiled_instruction,
            account_keys,
            idl_registry,
        }
    }

    /// Returns a reference to the IDL registry.
    pub fn idl_registry(&self) -> &crate::idl::IdlRegistry {
        self.idl_registry
    }

    /// Returns the sender address.
    pub fn sender(&self) -> &SolanaAccount {
        self.sender
    }

    /// Resolves the program_id_index to a pubkey.
    /// Returns `None` if the index is out of bounds (inaccessible).
    pub fn program_id(&self) -> Option<&'a solana_sdk::pubkey::Pubkey> {
        self.account_keys
            .get(self.compiled_instruction.program_id_index as usize)
    }

    /// Resolves the account at `position` in the instruction's accounts list.
    /// Returns `None` if the position doesn't exist in the instruction or
    /// the account index is out of bounds in account_keys.
    pub fn account(&self, position: usize) -> Option<&'a solana_sdk::pubkey::Pubkey> {
        let &idx = self.compiled_instruction.accounts.get(position)?;
        self.account_keys.get(idx as usize)
    }

    /// Returns the raw u8 account index at `position` in the instruction's
    /// accounts list, without resolving it. Useful for diagnostics.
    pub fn raw_account_index(&self, position: usize) -> Option<u8> {
        self.compiled_instruction.accounts.get(position).copied()
    }

    /// Returns the raw instruction data bytes. No copy — borrows from
    /// the compiled instruction.
    pub fn data(&self) -> &'a [u8] {
        &self.compiled_instruction.data
    }

    /// Returns the number of account references in this instruction.
    pub fn num_accounts(&self) -> usize {
        self.compiled_instruction.accounts.len()
    }

    /// Returns a reference to the underlying compiled instruction.
    pub fn compiled_instruction(&self) -> &'a solana_sdk::instruction::CompiledInstruction {
        self.compiled_instruction
    }

    /// Returns a reference to the account keys array.
    pub fn account_keys(&self) -> &'a [solana_sdk::pubkey::Pubkey] {
        self.account_keys
    }
}
```

Remove the old `instruction_index()`, `instructions()`, and `current_instruction()` methods entirely.

- [ ] **Step 4: Update InstructionVisualizer::can_handle default implementation**

In the same file, update the `can_handle` default method:

```rust
    fn can_handle(&self, context: &VisualizerContext) -> bool {
        let Some(config) = self.get_config() else {
            return false;
        };

        let Some(program_id) = context.program_id() else {
            return false;
        };

        config.can_handle(&program_id.to_string())
    }
```

- [ ] **Step 5: Simplify SolanaIntegrationConfig::can_handle signature**

Change the trait method from:

```rust
    fn can_handle(&self, program_id: &str, _instruction: &Instruction) -> bool {
```

To:

```rust
    fn can_handle(&self, program_id: &str) -> bool {
```

No implementation uses `_instruction`. Remove the `Instruction` import if it becomes unused.

- [ ] **Step 6: Update visualize_with_any**

The function currently receives `&[&dyn InstructionVisualizer]` and `&VisualizerContext`. The context no longer has `instruction_index()`. The debug logging line needs to change:

```rust
pub fn visualize_with_any(
    visualizers: &[&dyn InstructionVisualizer],
    context: &VisualizerContext,
) -> Option<Result<VisualizeResult, VisualSignError>> {
    visualizers.iter().find_map(|v| {
        if !v.can_handle(context) {
            return None;
        }

        Some(
            v.visualize_tx_commands(context)
                .map(|field| VisualizeResult {
                    field,
                    kind: v.kind(),
                }),
        )
    })
}
```

Remove the `eprintln!` debug logging line (it referenced `instruction_index()`). The framework loop will handle logging.

- [ ] **Step 7: Run the VisualizerContext unit tests**

Run: `cargo test -p visualsign-solana --lib core::tests 2>&1`

Expected: All 6 new tests pass. Other tests will fail (presets still use old API) — that's expected.

- [ ] **Step 8: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/core/mod.rs
git commit -S -m "refactor: VisualizerContext backed by &CompiledInstruction + &[Pubkey]

No copies of instruction data. Resolution of indices to pubkeys happens
lazily via helper methods. program_id(), account(n), data() return
Option/references. No instruction_index field — the caller owns position.

Eliminates the root cause of the index mismatch bug: there is no
filtered vec to index into, so there is no index to get wrong."
```

---

### Task 2: Update simple presets (system, compute_budget, associated_token_account, stakepool)

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/presets/system/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/compute_budget/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/associated_token_account/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/stakepool/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/*/config.rs` (4 files)

**Context:** Every preset follows the same pattern:
```rust
// OLD:
let instruction = context.current_instruction().ok_or_else(|| ...)?;
bincode::deserialize::<T>(&instruction.data)?;
instruction.program_id.to_string();
instruction.accounts.first().map(|m| m.pubkey.to_string());
format!("Instruction {}", context.instruction_index() + 1)
```
Becomes:
```rust
// NEW:
let data = context.data();
bincode::deserialize::<T>(data)?;
context.program_id().map(|pk| pk.to_string()).unwrap_or_else(|| "unknown".to_string());
context.account(0).map(|pk| pk.to_string()).unwrap_or_else(|| "unknown".to_string());
// No instruction label — framework applies it
```

- [ ] **Step 1: Update all four config.rs files**

Each config has `can_handle(&self, program_id: &str, _instruction: &Instruction) -> bool`. Only `unknown_program` overrides it — the other configs use the default trait method. But the trait signature changed (dropped `&Instruction` parameter), so if any config overrides `can_handle`, update it. Check each:

- `system/config.rs` — uses default, no override. No change needed.
- `compute_budget/config.rs` — uses default. No change needed.
- `associated_token_account/config.rs` — uses default. No change needed.
- `stakepool/config.rs` — uses default. No change needed.

If these configs had explicit overrides, remove the `_instruction` parameter. Since they don't, only the trait definition (already changed in Task 1) matters.

- [ ] **Step 2: Update system/mod.rs**

The system visualizer uses:
- `context.current_instruction()` → replace with direct `context` method calls
- `instruction.data` → `context.data()`
- `instruction.program_id.to_string()` → `context.program_id().map(|pk| pk.to_string()).unwrap_or_else(|| "unresolved".to_string())`
- `instruction.accounts.first()/.get(1)` → `context.account(0)`, `context.account(1)`
- `context.instruction_index() + 1` in labels → remove (framework handles)

Key changes pattern:
```rust
// Before:
let instruction = context
    .current_instruction()
    .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;
let system_instruction = bincode::deserialize::<SystemInstruction>(&instruction.data)
    .map_err(|e| ...)?;

// After:
let system_instruction = bincode::deserialize::<SystemInstruction>(context.data())
    .map_err(|e| ...)?;
```

For program_id display:
```rust
// Before:
&instruction.program_id.to_string()
// After:
&context.program_id().map(|pk| pk.to_string()).unwrap_or_else(|| "unresolved".to_string())
```

For account access:
```rust
// Before:
instruction.accounts.first().map(|meta| meta.pubkey.to_string()).unwrap_or_else(|| "Unknown".to_string())
// After:
context.account(0).map(|pk| pk.to_string()).unwrap_or_else(|| "unknown".to_string())
```

For instruction labels: remove `context.instruction_index() + 1` from label format strings. The label will be set to the operation name (e.g., "Transfer", "Create Account") without the instruction number prefix. The framework wraps with the position.

Also remove the `use solana_sdk::instruction::Instruction;` import if it becomes unused.

- [ ] **Step 3: Update compute_budget/mod.rs**

Same pattern as system. Key differences:
- Uses `ComputeBudgetInstruction::try_from_slice(&instruction.data)` → `ComputeBudgetInstruction::try_from_slice(context.data())`
- `instruction.program_id.to_string()` → `context.program_id()...`
- `instruction.data` for hex encoding → `context.data()`
- Remove instruction index from labels

- [ ] **Step 4: Update associated_token_account/mod.rs**

Same pattern:
- `parse_ata_instruction(&instruction.data)` → `parse_ata_instruction(context.data())`
- Account and program_id access same as above
- Remove instruction index from labels

- [ ] **Step 5: Update stakepool/mod.rs**

Same pattern:
- `parse_stake_pool_instruction(&instruction.data)` → `parse_stake_pool_instruction(context.data())`
- Note: this preset passes `instruction` (the solana_sdk Instruction) to helper functions. Those helpers need to accept the context or individual data instead. Check what `create_stakepool_preview_layout` uses and update its signature.

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p visualsign-solana 2>&1`

Expected: Compilation errors only in the presets not yet updated (jupiter_swap, token_2022, swig_wallet, unknown_program) and in instructions.rs/v0.rs.

- [ ] **Step 7: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/presets/system/ src/chain_parsers/visualsign-solana/src/presets/compute_budget/ src/chain_parsers/visualsign-solana/src/presets/associated_token_account/ src/chain_parsers/visualsign-solana/src/presets/stakepool/
git commit -S -m "refactor: update simple presets to use new VisualizerContext API

system, compute_budget, associated_token_account, stakepool now use
context.data(), context.program_id(), context.account(n) instead of
accessing owned Instruction fields. No instruction index in labels."
```

---

### Task 3: Update complex presets (jupiter_swap, token_2022, swig_wallet)

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/token_2022/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/swig_wallet/mod.rs`

**Context:** These presets pass `&instruction.accounts` (the full `Vec<AccountMeta>`) to internal parsing functions. With the new model, they need to either:
a) Build a local accounts list from `context.account(i)` for each position, or
b) Change internal parsers to accept the context directly

Option (a) is less invasive — build a compatibility shim:

```rust
/// Build a Vec of resolved account pubkey strings from the context.
/// Positions with inaccessible indices get "unresolved(N)" placeholder.
fn resolve_accounts(context: &VisualizerContext) -> Vec<String> {
    (0..context.num_accounts())
        .map(|i| {
            context.account(i)
                .map(|pk| pk.to_string())
                .unwrap_or_else(|| {
                    format!("unresolved({})",
                        context.raw_account_index(i).unwrap_or(0))
                })
        })
        .collect()
}
```

- [ ] **Step 1: Update jupiter_swap/mod.rs**

Jupiter does:
```rust
let instruction_accounts: Vec<String> = instruction.accounts.iter()
    .map(|account| account.pubkey.to_string()).collect();
parse_jupiter_swap_instruction(&instruction.data, &instruction_accounts)
```

Replace with:
```rust
let instruction_accounts: Vec<String> = (0..context.num_accounts())
    .map(|i| context.account(i).map(|pk| pk.to_string())
        .unwrap_or_else(|| format!("unresolved({})", context.raw_account_index(i).unwrap_or(0))))
    .collect();
parse_jupiter_swap_instruction(context.data(), &instruction_accounts)
```

Also update `instruction.program_id`, `instruction.data` references, and remove instruction index from labels.

- [ ] **Step 2: Update token_2022/mod.rs**

Token 2022 passes `&instruction.accounts` (as `&[AccountMeta]`) to `parse_token_2022_instruction`. The internal parser uses `accounts[0].pubkey.to_string()` etc. This is the most invasive change because the parser accesses `AccountMeta` directly.

Options:
a) Build a `Vec<AccountMeta>` shim from context (requires constructing AccountMeta with placeholder values for inaccessible accounts)
b) Change `parse_token_2022_instruction` to accept resolved pubkey strings

Option (a) preserves the existing parser:
```rust
let accounts: Vec<solana_sdk::instruction::AccountMeta> = (0..context.num_accounts())
    .map(|i| {
        let pubkey = context.account(i).copied()
            .unwrap_or_default(); // Pubkey::default() for inaccessible
        solana_sdk::instruction::AccountMeta::new_readonly(pubkey, false)
    })
    .collect();
let token_2022_instruction = parse_token_2022_instruction(context.data(), &accounts)?;
```

This preserves the downstream parser unchanged. `Pubkey::default()` for inaccessible accounts will show as "11111111..." in the output — acceptable since the diagnostic reports the real issue.

- [ ] **Step 3: Update swig_wallet/mod.rs**

Same approach as token_2022 — build `Vec<AccountMeta>` shim. Swig wallet is the largest preset (2631 lines) but the change is at the entry point only:

```rust
// Before:
let instruction = context.current_instruction().ok_or_else(|| ...)?;
parse_swig_instruction(&instruction.data, &instruction.accounts)

// After:
let accounts: Vec<solana_sdk::instruction::AccountMeta> = (0..context.num_accounts())
    .map(|i| {
        let pubkey = context.account(i).copied().unwrap_or_default();
        solana_sdk::instruction::AccountMeta::new_readonly(pubkey, false)
    })
    .collect();
parse_swig_instruction(context.data(), &accounts)
```

Update `instruction.program_id` references to `context.program_id()...` and remove instruction index from labels.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p visualsign-solana 2>&1`

Expected: Compilation errors only in unknown_program preset and instructions.rs/v0.rs.

- [ ] **Step 5: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/ src/chain_parsers/visualsign-solana/src/presets/token_2022/ src/chain_parsers/visualsign-solana/src/presets/swig_wallet/
git commit -S -m "refactor: update complex presets to use new VisualizerContext API

jupiter_swap, token_2022, swig_wallet build account lists from
context.account(i) to feed their existing parsers."
```

---

### Task 4: Update unknown_program preset (catch-all)

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/presets/unknown_program/mod.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/presets/unknown_program/config.rs`

**Context:** The unknown_program preset is the catch-all — `can_handle` always returns true. With the new model, it also handles instructions with inaccessible program_ids (where `context.program_id()` returns `None`). It needs to override `InstructionVisualizer::can_handle` directly (not just config.can_handle) because the default trait method returns false for `None` program_id.

- [ ] **Step 1: Update config.rs**

Update the `can_handle` signature to match the new trait:

```rust
    fn can_handle(&self, _program_id: &str) -> bool {
        true
    }
```

- [ ] **Step 2: Override can_handle on the InstructionVisualizer impl**

In `mod.rs`, add to the `InstructionVisualizer` impl for `UnknownProgramVisualizer`:

```rust
    fn can_handle(&self, _context: &VisualizerContext) -> bool {
        true // catch-all: handles everything including unresolved program_ids
    }
```

This ensures the unknown_program visualizer catches instructions where `program_id()` returns `None`.

- [ ] **Step 3: Update visualize_tx_commands**

```rust
fn visualize_tx_commands(
    &self,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let idl_registry = context.idl_registry();

    // Try IDL-based parsing if program_id is resolvable and has an IDL
    if let Some(program_id) = context.program_id() {
        if idl_registry.has_idl(program_id) {
            if let Ok(field) = try_idl_parsing(context, idl_registry) {
                return Ok(field);
            }
        }
    }

    create_unknown_program_preview_layout(context)
}
```

- [ ] **Step 4: Update try_idl_parsing and helper functions**

`try_idl_parsing` currently gets `&Instruction` from `context.current_instruction()`. Update it to use context methods:

```rust
fn try_idl_parsing(
    context: &VisualizerContext,
    idl_registry: &crate::idl::IdlRegistry,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id = context.program_id()
        .ok_or_else(|| VisualSignError::MissingData("No program_id".into()))?;
    let program_id_str = program_id.to_string();
    let instruction_data_hex = hex::encode(context.data());
    // ... rest uses program_id_str and instruction_data_hex
```

For account iteration in IDL matching:
```rust
// Before:
for (index, account_meta) in instruction.accounts.iter().enumerate() {
    named_accounts.insert(name, account_meta.pubkey.to_string());
}
// After:
for index in 0..context.num_accounts() {
    if let Some(idl_account) = idl_instruction.accounts.get(index) {
        let pubkey_str = context.account(index)
            .map(|pk| pk.to_string())
            .unwrap_or_else(|| format!("unresolved({})",
                context.raw_account_index(index).unwrap_or(0)));
        named_accounts.insert(idl_account.name.clone(), pubkey_str);
    }
}
```

`create_unknown_program_preview_layout` similarly:
```rust
fn create_unknown_program_preview_layout(
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id = context.program_id()
        .map(|pk| pk.to_string())
        .unwrap_or_else(|| format!("unresolved({})",
            context.compiled_instruction().program_id_index));
    let instruction_data_hex = hex::encode(context.data());
    // ... rest uses program_id and instruction_data_hex
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p visualsign-solana 2>&1`

Expected: Errors only in instructions.rs and v0.rs (not yet updated).

- [ ] **Step 6: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/presets/unknown_program/
git commit -S -m "refactor: update unknown_program to catch-all including unresolved program_ids

Overrides InstructionVisualizer::can_handle to return true for all
instructions including those with inaccessible program_id_index.
Shows 'unresolved(N)' for inaccessible indices."
```

---

### Task 5: Refactor instruction processing — no more skipping

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/core/instructions.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/core/txtypes/v0.rs`

**Context:** The big change. Currently `decode_instructions` builds `indexed_instructions: Vec<(usize, Instruction)>` by skipping OOB program_ids and filtering OOB accounts. With the new model: iterate `message.instructions` directly, construct `VisualizerContext` for each one, run through visualizer pipeline. No skipping. No filtering. Diagnostics are emitted by a separate scan.

- [ ] **Step 1: Write test for legacy path — all instructions processed, none skipped**

Add to the test module in `instructions.rs`:

```rust
#[test]
fn test_oob_program_id_instruction_not_skipped() {
    // Instruction 0 has OOB program_id. Previously it was skipped.
    // Now it should be processed (unknown_program visualizer catches it).
    let key0 = Pubkey::new_unique();
    let key1 = Pubkey::new_unique();
    let message = Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 0,
        },
        account_keys: vec![key0, key1],
        recent_blockhash: Hash::default(),
        instructions: vec![
            solana_sdk::instruction::CompiledInstruction {
                program_id_index: 99, // OOB
                accounts: vec![0],
                data: vec![0xAA],
            },
            solana_sdk::instruction::CompiledInstruction {
                program_id_index: 1, // valid
                accounts: vec![0],
                data: vec![0xBB],
            },
        ],
    };
    let tx = SolanaTransaction { signatures: vec![], message };
    let registry = IdlRegistry::new();
    let config = LintConfig::default();
    let result = decode_instructions(&tx, &registry, &config);

    // Both instructions should produce fields (or errors) — none skipped
    let total_outputs = result.fields.len() + result.errors.len();
    assert_eq!(
        total_outputs, 2,
        "Expected output for all 2 instructions (none skipped), got {} fields + {} errors",
        result.fields.len(), result.errors.len()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p visualsign-solana test_oob_program_id_instruction_not_skipped 2>&1`

Expected: FAIL — currently skips OOB instruction, only produces 1 output.

- [ ] **Step 3: Rewrite decode_instructions**

Replace the entire function body. The new structure:

1. Emit diagnostics for OOB indices (separate scan)
2. Iterate all instructions, create VisualizerContext for each, run through visualizer pipeline
3. No filtered vec, no indexed_instructions, no Instruction construction

```rust
pub fn decode_instructions(
    transaction: &SolanaTransaction,
    idl_registry: &IdlRegistry,
    lint_config: &LintConfig,
) -> DecodeInstructionsResult {
    let visualizers: Vec<Box<dyn InstructionVisualizer>> = available_visualizers();
    let visualizers_refs: Vec<&dyn InstructionVisualizer> =
        visualizers.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

    let message = &transaction.message;
    let account_keys = &message.account_keys;

    if account_keys.is_empty() {
        return DecodeInstructionsResult {
            fields: Vec::new(),
            errors: Vec::new(),
            diagnostics: vec![create_diagnostic_field(
                "transaction::empty_account_keys",
                "transaction",
                lint_config.severity_for("transaction::empty_account_keys", visualsign::lint::Severity::Error).as_str(),
                "legacy transaction has no account keys",
                None,
            )],
        };
    }

    // Diagnostic scan: check all indices, emit diagnostics for inaccessible ones
    let diagnostics = scan_instruction_diagnostics(
        &message.instructions,
        account_keys,
        lint_config,
    );

    // Visualization: process every instruction (no skipping)
    let mut fields: Vec<AnnotatedPayloadField> = Vec::new();
    let mut errors: Vec<(usize, VisualSignError)> = Vec::new();

    for (i, ci) in message.instructions.iter().enumerate() {
        let sender = SolanaAccount {
            account_key: account_keys[0].to_string(),
            signer: false,
            writable: false,
        };

        let context = VisualizerContext::new(&sender, ci, account_keys, idl_registry);

        match visualize_with_any(&visualizers_refs, &context) {
            Some(Ok(viz_result)) => fields.push(viz_result.field),
            Some(Err(e)) => errors.push((i, e)),
            None => errors.push((
                i,
                VisualSignError::DecodeError(format!(
                    "No visualizer available for instruction at index {i}"
                )),
            )),
        }
    }

    DecodeInstructionsResult {
        fields,
        errors,
        diagnostics,
    }
}
```

- [ ] **Step 4: Implement `scan_instruction_diagnostics` (shared function)**

Add a new function that both legacy and v0 paths can use:

```rust
/// Scan compiled instructions for inaccessible indices and emit diagnostics.
/// Does not modify or filter instructions — purely informational.
fn scan_instruction_diagnostics(
    instructions: &[solana_sdk::instruction::CompiledInstruction],
    account_keys: &[solana_sdk::pubkey::Pubkey],
    lint_config: &LintConfig,
) -> Vec<AnnotatedPayloadField> {
    let mut diagnostics: Vec<AnnotatedPayloadField> = Vec::new();
    let mut oob_program_id_count: usize = 0;
    let mut oob_account_index_count: usize = 0;

    let oob_pid_severity = lint_config.severity_for(
        "transaction::oob_program_id",
        visualsign::lint::Severity::Warn,
    );
    let oob_acct_severity = lint_config.severity_for(
        "transaction::oob_account_index",
        visualsign::lint::Severity::Warn,
    );

    for (ci_index, ci) in instructions.iter().enumerate() {
        // Check program_id_index
        if (ci.program_id_index as usize) >= account_keys.len() {
            oob_program_id_count += 1;
            if !matches!(oob_pid_severity, visualsign::lint::Severity::Allow) {
                diagnostics.push(create_diagnostic_field(
                    "transaction::oob_program_id",
                    "transaction",
                    oob_pid_severity.as_str(),
                    &format!(
                        "instruction {}: program_id_index {} out of bounds ({} account keys)",
                        ci_index, ci.program_id_index, account_keys.len()
                    ),
                    Some(ci_index as u32),
                ));
            }
        }

        // Check all account indices (unified — no separate "skipped" rule)
        for &account_idx in &ci.accounts {
            if (account_idx as usize) >= account_keys.len() {
                oob_account_index_count += 1;
                if !matches!(oob_acct_severity, visualsign::lint::Severity::Allow) {
                    diagnostics.push(create_diagnostic_field(
                        "transaction::oob_account_index",
                        "transaction",
                        oob_acct_severity.as_str(),
                        &format!(
                            "instruction {}: account index {} out of bounds ({} account keys)",
                            ci_index, account_idx, account_keys.len()
                        ),
                        Some(ci_index as u32),
                    ));
                }
                break; // one diagnostic per instruction for account OOB
            }
        }
    }

    // Boot-metric ok diagnostics
    if oob_program_id_count == 0
        && lint_config.should_report_ok("transaction::oob_program_id")
    {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_program_id",
            "transaction",
            "ok",
            &format!(
                "all {} instructions have valid program_id_index",
                instructions.len()
            ),
            None,
        ));
    }
    if oob_account_index_count == 0
        && lint_config.should_report_ok("transaction::oob_account_index")
    {
        diagnostics.push(create_diagnostic_field(
            "transaction::oob_account_index",
            "transaction",
            "ok",
            &format!(
                "all {} instructions have valid account indices",
                instructions.len()
            ),
            None,
        ));
    }

    diagnostics
}
```

Note: `create_diagnostic_field` still accepts `&str` at this point (Task 8 changes it to `Severity`). Use `.as_str()` on severity values here. Task 8 will remove the `.as_str()` calls when updating the signature.

- [ ] **Step 5: Remove old OOB-checking loop, indexed_instructions, Instruction construction**

Delete all the code between the `account_keys.is_empty()` check and the visualization loop — the entire OOB detection loop that built `indexed_instructions`, the `oob_account_index_in_skipped_instruction` logic, the `Instruction` construction, and the `instructions` clone. The `scan_instruction_diagnostics` function replaces all of it.

Also remove `DecodeInstructionsResult` if unused (the struct may stay the same or simplify — keep `fields`, `errors`, `diagnostics`).

- [ ] **Step 6: Apply the same refactor to v0.rs**

Rewrite `decode_v0_instructions` following the same pattern. Call `scan_instruction_diagnostics` with `&v0_message.instructions` and `&v0_message.account_keys`. The visualization loop iterates all `v0_message.instructions` directly.

Remove `DecodeV0InstructionsResult` if identical to `DecodeInstructionsResult` — unify into one type exported from a shared location.

Fix the doc comment placement (moves from struct to function — addresses review comment #12).

- [ ] **Step 7: Run the test**

Run: `cargo test -p visualsign-solana test_oob_program_id_instruction_not_skipped 2>&1`

Expected: PASS

- [ ] **Step 8: Update existing tests**

The old tests checked for `oob_account_index_in_skipped_instruction` diagnostics — this rule no longer exists. Update:
- `test_oob_program_id_emits_diagnostic` — remove assertion for `oob_account_index_in_skipped_instruction` ok diagnostic. Now only 2 ok diagnostics (oob_program_id warn + oob_account_index ok).
- `test_oob_program_id_and_oob_account_index_emits_both_diagnostics` — the OOB account in a "skipped" instruction now emits `transaction::oob_account_index` (not the old "in_skipped_instruction" variant).
- `test_valid_transaction_emits_pass_diagnostics` — only 2 ok diagnostics now.
- Same updates for v0 tests.

- [ ] **Step 9: Run all solana tests**

Run: `cargo test -p visualsign-solana 2>&1`

Expected: All pass.

- [ ] **Step 10: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/core/instructions.rs src/chain_parsers/visualsign-solana/src/core/txtypes/v0.rs
git commit -S -m "refactor: eliminate instruction skipping, unified diagnostic scan

No instructions are ever skipped. VisualizerContext is created for
every compiled instruction. Diagnostics for inaccessible indices are
emitted by scan_instruction_diagnostics (shared between legacy and v0).

Eliminates: filtered instruction vec, oob_account_index_in_skipped_instruction
rule, code duplication between legacy and v0 diagnostic logic."
```

---

### Task 6: Framework-level instruction labeling + convert function updates

**Files:**
- Modify: `src/chain_parsers/visualsign-solana/src/core/visualsign.rs`

**Context:** Instruction labels ("Instruction 1", "Instruction 2") were previously set by each visualizer preset. Now the framework applies them in the convert functions after visualization. Also thread `&LintConfig` through convert functions (addresses review comment #11) and surface errors as diagnostics (addresses review comment #9).

- [ ] **Step 1: Create label-wrapping helper**

```rust
/// Wrap a visualization field with the instruction's position label.
fn label_instruction_field(
    position: usize,
    mut field: AnnotatedPayloadField,
) -> AnnotatedPayloadField {
    // Prepend "Instruction N: " to the label if not already present
    let label = &field.signable_payload_field.label();
    if !label.starts_with("Instruction ") {
        let new_label = format!("Instruction {}: {}", position + 1, label);
        // Update the label in the field's common struct
        match &mut field.signable_payload_field {
            SignablePayloadField::PreviewLayout { common, .. }
            | SignablePayloadField::TextV2 { common, .. }
            | SignablePayloadField::Text { common, .. }
            | SignablePayloadField::Number { common, .. }
            | SignablePayloadField::AmountV2 { common, .. }
            | SignablePayloadField::AddressV2 { common, .. }
            | SignablePayloadField::Diagnostic { common, .. } => {
                common.label = new_label;
            }
            _ => {} // ListLayout, Divider, Unknown don't have labels in the same way
        }
    }
    field
}
```

- [ ] **Step 2: Thread LintConfig through convert functions**

Change `convert_to_visual_sign_payload`, `convert_versioned_to_visual_sign_payload`, and `convert_v0_to_visual_sign_payload` to accept `&LintConfig` parameter. Create default once in `to_visual_sign_payload`:

```rust
fn to_visual_sign_payload(&self, wrapper: SolanaTransactionWrapper, options: VisualSignOptions)
    -> Result<SignablePayload, VisualSignError> {
    let lint_config = visualsign::lint::LintConfig::default();
    match wrapper {
        SolanaTransactionWrapper::Legacy(tx) =>
            convert_to_visual_sign_payload(&tx, options.decode_transfers, options.transaction_name.clone(), &options, &lint_config),
        SolanaTransactionWrapper::Versioned(vtx) =>
            convert_versioned_to_visual_sign_payload(&vtx, options.decode_transfers, options.transaction_name.clone(), &options, &lint_config),
    }
}
```

- [ ] **Step 3: Apply framework labeling in convert functions**

In `convert_to_visual_sign_payload`, after getting `decode_result`:

```rust
    // Apply framework-level instruction labels
    for (i, field) in decode_result.fields.iter_mut().enumerate() {
        *field = label_instruction_field(i, field.clone());
    }
```

Or better, if decode_instructions returns fields without labels:

```rust
    let decode_result = instructions::decode_instructions(transaction, &idl_registry, lint_config);
    fields.extend(
        decode_result.fields.into_iter().enumerate().map(|(i, f)| {
            label_instruction_field(i, f).signable_payload_field
        }),
    );
```

- [ ] **Step 4: Surface errors as diagnostics with comment**

```rust
    // Surface per-instruction errors as diagnostics.
    // decode::visualizer_error is intentionally not routed through LintConfig —
    // visualizer failures are always surfaced so consumers know which
    // instructions could not be decoded.
    for (idx, err) in &decode_result.errors {
        fields.push(
            visualsign::field_builders::create_diagnostic_field(
                "decode::visualizer_error",
                "decode",
                "error",
                &format!("instruction {idx}: {err}"),
                Some(*idx as u32),
            )
            .signable_payload_field,
        );
    }
```

Apply same changes to the v0 convert function.

- [ ] **Step 5: Run tests**

Run: `cargo test -p visualsign-solana 2>&1 && cargo test -p parser_cli 2>&1`

Expected: All pass (fixture outputs may need updating due to label format changes).

- [ ] **Step 6: Update fixtures if needed**

If CLI fixture tests fail due to label changes (e.g., "Instruction 1: Transfer" instead of "Transfer: 10000000000 lamports"), regenerate the expected fixture files:

Run: `cargo run --bin parser_cli -- $(cat src/parser/cli/tests/fixtures/solana-json.input | tr '\n' ' ') 2>/dev/null > src/parser/cli/tests/fixtures/solana-json.display.expected.tmp`

Compare and update the fixture file.

- [ ] **Step 7: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/core/visualsign.rs src/parser/cli/tests/fixtures/
git commit -S -m "refactor: framework-level instruction labeling, thread LintConfig

Instruction position labels applied by the framework, not individual
presets. LintConfig threaded from to_visual_sign_payload to both
legacy and v0 convert functions."
```

---

### Task 7: Replace .unwrap() in Diagnostic serialize impl

**Files:**
- Modify: `src/visualsign/src/lib.rs:633-657`

**Context:** Addresses review comment #7. The `Serialize` impl for `SignablePayloadFieldDiagnostic` uses `serde_json::to_value().unwrap()`. Replace with direct `serialize_entry` calls.

- [ ] **Step 1: Replace the Serialize impl**

```rust
impl Serialize for SignablePayloadFieldDiagnostic {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let len = if self.instruction_index.is_some() { 5 } else { 4 };
        let mut map = serializer.serialize_map(Some(len))?;
        map.serialize_entry("Domain", &self.domain)?;
        if let Some(ref idx) = self.instruction_index {
            map.serialize_entry("InstructionIndex", idx)?;
        }
        map.serialize_entry("Level", &self.level)?;
        map.serialize_entry("Message", &self.message)?;
        map.serialize_entry("Rule", &self.rule)?;
        map.end()
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p visualsign diagnostic 2>&1`

Expected: All pass.

- [ ] **Step 3: Commit**

```bash
git add src/visualsign/src/lib.rs
git commit -S -m "fix: remove unwrap from Diagnostic serialize impl

Use serialize_entry directly instead of intermediate BTreeMap with
serde_json::to_value().unwrap()."
```

---

### Task 8: Accept Severity enum in create_diagnostic_field

**Files:**
- Modify: `src/visualsign/src/field_builders.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/core/instructions.rs` (all callers)
- Modify: `src/chain_parsers/visualsign-solana/src/core/txtypes/v0.rs` (all callers)
- Modify: `src/chain_parsers/visualsign-solana/src/core/visualsign.rs` (all callers)

**Context:** Addresses review comment #8. Change `level: &str` to `level: Severity`.

- [ ] **Step 1: Update builder signature**

In `field_builders.rs`:

```rust
pub fn create_diagnostic_field(
    rule: &str,
    domain: &str,
    level: crate::lint::Severity,
    message: &str,
    instruction_index: Option<u32>,
) -> AnnotatedPayloadField {
    let level_str = level.as_str();
    match level {
        crate::lint::Severity::Warn | crate::lint::Severity::Error => {
            tracing::warn!(rule, domain, level = level_str, ?instruction_index, "{message}");
        }
        _ => {}
    }
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::Diagnostic {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{level_str}: {message}"),
                label: rule.to_string(),
            },
            diagnostic: SignablePayloadFieldDiagnostic {
                rule: rule.to_string(),
                domain: domain.to_string(),
                level: level_str.to_string(),
                message: message.to_string(),
                instruction_index,
            },
        },
    }
}
```

- [ ] **Step 2: Update all callers**

In `instructions.rs` and `v0.rs` (`scan_instruction_diagnostics`): callers already pass `Severity` values — remove `.as_str()` calls. For the `"ok"` strings, use `Severity::Ok`. For `"error"` strings, use `Severity::Error`.

In `visualsign.rs`: the `decode::visualizer_error` calls use `"error"` — change to `Severity::Error`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p visualsign-solana 2>&1 && cargo test -p visualsign 2>&1`

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add src/visualsign/src/field_builders.rs src/chain_parsers/visualsign-solana/src/core/
git commit -S -m "refactor: accept Severity enum in create_diagnostic_field

Prevents arbitrary strings from entering the attested payload."
```

---

### Task 9: Restore text/human test coverage + strengthen diagnostic assertions

**Files:**
- Create: `src/parser/cli/tests/fixtures/solana-text.input`
- Create: `src/parser/cli/tests/fixtures/solana-text.display.expected`
- Modify: `src/parser/cli/tests/cli_test.rs`
- Modify: `src/parser/cli/tests/fixtures/solana-json.diagnostics.expected`

**Context:** Addresses review comments #5 and #6.

- [ ] **Step 1: Recreate solana-text fixture**

Run: `git show main:src/parser/cli/tests/fixtures/solana-text.input > src/parser/cli/tests/fixtures/solana-text.input`

- [ ] **Step 2: Generate expected output**

Build and run:
```bash
cargo build --bin parser_cli 2>&1
cargo run --bin parser_cli -- $(cat src/parser/cli/tests/fixtures/solana-text.input | tr '\n' ' ') 2>/dev/null > src/parser/cli/tests/fixtures/solana-text.display.expected
```

- [ ] **Step 3: Update test loop to handle non-JSON output**

In `cli_test.rs`, replace the display/diagnostic comparison block with a try-JSON-first approach:

```rust
        match serde_json::from_str::<serde_json::Value>(actual_output.trim()) {
            Ok(actual_json) => {
                // JSON path: filter diagnostics, compare display, check diagnostics fixture
                // ... (existing JSON logic, enhanced with instruction_index checking)
            }
            Err(_) => {
                // Non-JSON (text/human): plain string comparison
                let expected_display = fs::read_to_string(&display_path)
                    .unwrap_or_else(|_| panic!("Failed to read: {display_path:?}"));
                assert_strings_match(test_name, "display", expected_display.trim(), actual_output.trim());
            }
        }
```

- [ ] **Step 4: Update diagnostics fixture**

The valid Solana transfer transaction now emits 2 ok diagnostics (oob_program_id, oob_account_index — no more oob_account_index_in_skipped_instruction):

```json
[
  { "rule": "transaction::oob_program_id", "level": "ok" },
  { "rule": "transaction::oob_account_index", "level": "ok" }
]
```

- [ ] **Step 5: Strengthen diagnostic assertions to check instruction_index**

In the diagnostics comparison block, also check `instruction_index` when present in the expected fixture.

- [ ] **Step 6: Run tests**

Run: `cargo test -p parser_cli 2>&1`

Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add src/parser/cli/tests/
git commit -S -m "test: restore text-format fixture, strengthen diagnostic assertions

Text/human output formats now have test coverage. Diagnostic assertions
check instruction_index when present in the fixture."
```

---

### Task 10: Full CI checks + reply to reviewers

**Files:** None (verification + PR comments)

- [ ] **Step 1: Run fmt**

Run: `make -C src fmt 2>&1`

- [ ] **Step 2: Run clippy**

Run: `make -C src lint 2>&1`

Expected: Clean.

- [ ] **Step 3: Run all tests**

Run: `make -C src test 2>&1`

Expected: All pass.

- [ ] **Step 4: Reply to review comments**

Use the `resolve-pr-reviews` skill to respond to each pepe-anchor comment with a summary of what was done.
