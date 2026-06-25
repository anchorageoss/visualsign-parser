# Extract Shared Arg Rendering Utilities Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `format_arg_value`, `charset_safe`, and `bytes_as_hex` from per-preset duplicates into a single `crate::core::arg_rendering` module, fixing a latent charset-safety bug in 16 presets along the way.

**Architecture:** A new `arg_rendering.rs` file joins `crate::core` and is re-exported from `crate::core::mod.rs`. All 17 affected preset `mod.rs` files drop their local copies and import from `crate::core`. The skill instructions are updated to reference the import rather than embedding the function bodies.

**Tech Stack:** Rust (edition 2024, nightly 1.88), `serde_json`, `hex` crate, `cargo clippy --all-targets -D warnings`, `cargo test -p visualsign-solana`.

## Global Constraints

- `unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"` at workspace level; test modules use `#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`
- `unsafe_code = "forbid"` — no unsafe blocks
- `use` statements at top of module, never inside functions
- Inline format strings: `format!("{variable}")` not `format!("{}", variable)`
- ASCII only in user-visible strings: `>=` not `≥`, `->` not `→`
- `BTreeMap` not `HashMap` for deterministic ordering in rendered output
- Run `make -C src test` for full suite; run `cargo test -p visualsign-solana` for focused

---

## Background: The Bug

16 of 17 presets with `format_arg_value` use an old implementation:

```rust
// OLD — charset-unsafe
fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),   // emits JSON: {"key":"value"} — `"` fails validate_charset
    }
}
```

`SignablePayload::validate_charset` (added in #332) rejects `"` and `\`. Any preset whose IDL produces object or array args will hit "Restricted Characters Detected" errors on real transactions.

The canonical fix (from Opus in #387, applied only to `dflow_aggregator`) renders objects quote-free and collapses all-byte arrays to `0x` hex. This plan propagates that fix to all presets via a shared module.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| **Create** | `src/chain_parsers/visualsign-solana/src/core/arg_rendering.rs` | Canonical `format_arg_value`, `charset_safe`, `bytes_as_hex` with tests |
| **Modify** | `src/chain_parsers/visualsign-solana/src/core/mod.rs` | Add `mod arg_rendering; pub use arg_rendering::format_arg_value;` |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/dflow_aggregator/mod.rs` | Remove local copies, add import |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/kamino_vault/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/kamino_borrow/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/kamino_farms/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/kamino_limit/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/jupiter_earn/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/jupiter_borrow/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/jupiter_perps/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/drift/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/exponent_finance/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/metadao_conditional_vault/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/metadao_futarchy/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/meteora_damm_v2/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/meteora_dlmm/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/neutral_trade/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/onre_app/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/orca_whirlpool/mod.rs` | Same |
| **Modify** | `src/chain_parsers/visualsign-solana/src/presets/squads_multisig/mod.rs` | Same |
| **Modify** | `.claude/skills/solana-add-idl/SKILL.md` | Reference import instead of embedding bodies |

---

### Task 1: Create `crate::core::arg_rendering` with tests

**Files:**
- Create: `src/chain_parsers/visualsign-solana/src/core/arg_rendering.rs`
- Modify: `src/chain_parsers/visualsign-solana/src/core/mod.rs`

**Interfaces:**
- Produces: `pub fn format_arg_value(value: &serde_json::Value) -> String` — the only public export; `charset_safe` and `bytes_as_hex` are private helpers tested indirectly

- [ ] **Step 1: Write the failing tests first**

Add `arg_rendering.rs` with only the test module and stub:

```rust
// src/chain_parsers/visualsign-solana/src/core/arg_rendering.rs

pub fn format_arg_value(_value: &serde_json::Value) -> String {
    todo!()
}

fn charset_safe(_text: &str) -> String {
    todo!()
}

fn bytes_as_hex(_items: &[serde_json::Value]) -> Option<String> {
    todo!()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scalars() {
        assert_eq!(format_arg_value(&json!("hello")), "hello");
        assert_eq!(format_arg_value(&json!(42)), "42");
        assert_eq!(format_arg_value(&json!(true)), "true");
        assert_eq!(format_arg_value(&serde_json::Value::Null), "null");
    }

    #[test]
    fn test_byte_array_renders_as_hex() {
        assert_eq!(format_arg_value(&json!([1u8, 2, 3])), "0x010203");
        assert_eq!(format_arg_value(&json!([0u8, 255])), "0x00ff");
    }

    #[test]
    fn test_non_byte_array_renders_as_bracketed_list() {
        // 256 is out of u8 range — disqualifies the whole array
        assert_eq!(format_arg_value(&json!([1, 2, 256])), "[1,2,256]");
        assert_eq!(format_arg_value(&json!(["a", "b"])), "[a,b]");
    }

    #[test]
    fn test_empty_array_renders_as_brackets() {
        assert_eq!(format_arg_value(&json!([])), "[]");
    }

    #[test]
    fn test_object_renders_quote_free() {
        // Key order may vary by serde_json feature flags — assert both orderings
        let result = format_arg_value(&json!({"side": "buy", "amount": 100}));
        assert!(
            result == "{amount:100,side:buy}" || result == "{side:buy,amount:100}",
            "unexpected: {result}"
        );
    }

    #[test]
    fn test_empty_object_renders_as_braces() {
        assert_eq!(format_arg_value(&json!({})), "{}");
    }

    #[test]
    fn test_charset_safe_no_quotes_or_backslashes() {
        let result = format_arg_value(&json!({"k\"ey": "val\\ue"}));
        assert!(!result.contains('"'), "must not contain quote: {result}");
        assert!(!result.contains('\\'), "must not contain backslash: {result}");
    }

    #[test]
    fn test_string_with_forbidden_chars_is_stripped() {
        // `"` and `\` are stripped; printable ASCII and spaces are kept
        assert_eq!(format_arg_value(&json!("a\"b\\c d")), "a b d");
        // No tab, CR, non-ASCII
        assert_eq!(format_arg_value(&json!("a\tb\rc")), "abc");
    }

    #[test]
    fn test_nested_object_does_not_emit_quotes() {
        let nested = json!({"outer": {"inner": "value"}});
        let result = format_arg_value(&nested);
        assert!(!result.contains('"'), "nested object must be quote-free: {result}");
    }
}
```

- [ ] **Step 2: Wire module into `crate::core` so it compiles**

In `src/chain_parsers/visualsign-solana/src/core/mod.rs`, add after the existing `mod` declarations:

```rust
mod arg_rendering;
pub use arg_rendering::format_arg_value;
```

- [ ] **Step 3: Run tests to confirm they fail**

```bash
cargo test -p visualsign-solana arg_rendering 2>&1 | tail -5
```

Expected: compile error or `panicked at 'not yet implemented'`.

- [ ] **Step 4: Implement the three functions**

Replace the stubs in `arg_rendering.rs`:

```rust
pub fn format_arg_value(value: &serde_json::Value) -> String {
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

fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
}

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
```

- [ ] **Step 5: Run tests and confirm they pass**

```bash
cargo test -p visualsign-solana arg_rendering
cargo clippy -p visualsign-solana --all-targets -- -D warnings
```

Expected: all `arg_rendering::tests::*` pass, no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/core/arg_rendering.rs \
        src/chain_parsers/visualsign-solana/src/core/mod.rs
git commit -m "feat(solana/core): add shared arg_rendering module with format_arg_value"
```

---

### Task 2: Migrate all presets to the shared module

**Files:** All 18 preset `mod.rs` files listed in the file structure table above.

**Interfaces:**
- Consumes: `crate::core::format_arg_value` from Task 1

For each preset, the mechanical change is:
1. Remove the local `fn format_arg_value`, `fn charset_safe`, `fn bytes_as_hex` definitions
2. Add `use crate::core::format_arg_value;` to the imports block at the top of the file
3. Leave `append_raw_data` in place — its signatures differ too much across presets to share; callers should eventually replace it with direct `create_raw_data_field(data, None)?` calls, but that's out of scope here

**Behavior change to expect:** Presets that had the old `format_arg_value` (returning `other.to_string()` for objects/arrays) now produce quote-free output. Any test that asserted old JSON-with-quotes output will fail and must be updated to the canonical quote-free form. This is a bug fix, not a regression.

- [ ] **Step 1: Migrate dflow_aggregator** (already has canonical version — verify no behavior change)

In `src/chain_parsers/visualsign-solana/src/presets/dflow_aggregator/mod.rs`:

Remove these three function bodies (they appear after `build_fallback_fields`):
```rust
fn format_arg_value(value: &serde_json::Value) -> String { ... }
fn charset_safe(text: &str) -> String { ... }
fn bytes_as_hex(items: &[serde_json::Value]) -> Option<String> { ... }
```

Add to the imports block at the top of the file:
```rust
use crate::core::format_arg_value;
```

Run: `cargo test -p visualsign-solana dflow_aggregator`

Expected: all pass (behavior unchanged — same canonical implementation).

- [ ] **Step 2: Migrate the 8 presets with `append_raw_data` and the old format_arg_value**

For each of: `kamino_vault`, `kamino_borrow`, `kamino_farms`, `kamino_limit`, `jupiter_earn`, `jupiter_borrow`, `jupiter_perps`, `drift`, `squads_multisig`, `neutral_trade`:

- Remove the local `fn format_arg_value` (and `fn charset_safe`, `fn bytes_as_hex` if present — most don't have them separately)
- Add `use crate::core::format_arg_value;` to imports

Run after each batch of 3-4:
```bash
cargo test -p visualsign-solana
```

If a test fails because it expected old JSON-with-quotes output (e.g. `{"amount":100}` instead of `{amount:100}`), update the test assertion to the canonical quote-free form.

- [ ] **Step 3: Migrate the remaining presets without `append_raw_data`**

For each of: `exponent_finance`, `metadao_conditional_vault`, `metadao_futarchy`, `meteora_damm_v2`, `meteora_dlmm`, `orca_whirlpool`, `onre_app`:

Same mechanical change — remove local `fn format_arg_value`, add `use crate::core::format_arg_value;`.

Run: `cargo test -p visualsign-solana`

- [ ] **Step 4: Full suite + clippy**

```bash
cargo fmt -p visualsign-solana
cargo clippy -p visualsign-solana --all-targets -- -D warnings
cargo clippy -p visualsign-solana --features diagnostics --all-targets -- -D warnings
cargo test -p visualsign-solana
cargo test -p visualsign-solana --features diagnostics
make -C src test
```

Expected: all pass, no warnings. If `make -C src test` fails due to integration tests needing built binaries, run `make -C src build` first.

- [ ] **Step 5: Commit**

```bash
git add src/chain_parsers/visualsign-solana/src/presets/
git commit -m "refactor(solana/presets): replace per-preset format_arg_value with crate::core import

Propagates the charset-safe, field-explosion-safe arg rendering (originally
introduced in dflow_aggregator by #387) to all 17 remaining presets.

The old implementations used serde_json::Value::to_string() for objects and
arrays, which emits JSON with quoted keys. SignablePayload::validate_charset
(added in #332) rejects the resulting backslash-escaped quotes, causing
'Restricted Characters Detected' errors on any transaction whose IDL produces
object or array args. The shared crate::core::format_arg_value renders
objects quote-free and collapses all-byte arrays to 0x hex."
```

---

### Task 3: Update the skill to reference the shared import

**Files:**
- Modify: `.claude/skills/solana-add-idl/SKILL.md`

**Interfaces:**
- Consumes: Task 1 (the shared module now exists)

- [ ] **Step 1: Replace embedded function bodies with an import line**

In the `## Implementation Prompt (for subagents)` section, under `### Required: arg rendering helpers`, replace the full `fn format_arg_value`, `fn charset_safe`, and `fn bytes_as_hex` bodies with:

```markdown
### Required: arg rendering import

Add to the imports block:
```rust
use crate::core::format_arg_value;
```

The two constraints this solves — and why they apply to every program:

**Constraint 1 — charset safety.** `SignablePayload::validate_charset` rejects `"` and `\`.
Compact JSON of any object contains quoted keys (`{"key":"val"}`), which fail validation.
`format_arg_value` renders objects and strings quote-free.

**Constraint 2 — field explosion.** Recursing into nested structs/arrays produces one field
per leaf. A `[u8; 32]` seed or 76-byte opaque ID becomes 32–76 per-byte fields, burying
meaningful arguments. `format_arg_value` keeps each top-level arg as exactly one field,
collapsing all-byte arrays to `0x` hex.

Use it for every program-call arg in `build_parsed_fields`:
```rust
for (key, value) in &parsed.program_call_args {
    condensed_fields.push(create_text_field(key, &format_arg_value(value))?);
}
// same in expanded_fields
```
```

- [ ] **Step 2: Update required tests in the skill**

Replace the two arg-rendering test stubs in the skill's `### Required tests` section:

```rust
#[test]
fn test_format_arg_value_is_charset_safe() {
    // No `"` or `\` in output — validate_charset would reject them (#332)
    let obj = serde_json::json!({"k": "v\"alue"});
    let result = crate::core::format_arg_value(&obj);  // or just format_arg_value if imported
    assert!(!result.contains('"') && !result.contains('\\'));
}

#[test]
fn test_format_arg_value_no_field_explosion() {
    // 32-element byte array → one hex string, not 32 comma-separated numbers
    let bytes: Vec<u8> = (0..32).collect();
    let val = serde_json::json!(bytes);
    let result = crate::core::format_arg_value(&val);
    assert!(result.starts_with("0x"), "byte array should be hex: {result}");
    assert!(!result.contains(','), "must not expand to list: {result}");
}
```

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/solana-add-idl/SKILL.md
git commit -m "docs(skills): reference crate::core::format_arg_value instead of embedding bodies

Now that the shared module exists, the scaffold instructions are a one-line
import rather than three embedded function bodies. Constraints and tests
remain so the subagent understands why the import matters."
```
