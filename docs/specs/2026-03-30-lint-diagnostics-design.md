# Lint Diagnostics in SignablePayload — Design Spec

Issue: #228

## Goal

Add structured lint diagnostics to `SignablePayload` as a new `Diagnostic` variant of `SignablePayloadField`. Diagnostics are attested alongside display fields in the signed payload. This first slice implements rules for Solana legacy and v0 transactions and introduces a `LintConfig` framework for configuring rule severity, replacing the current silent data dropping.

## Error categorization

| Category | Handled by | Example |
|----------|------------|---------|
| **Parser errors** (`VisualSignError`) | Collected per-instruction in `errors` vec, surfaced as `decode::visualizer_error` diagnostics | No visualizer found |
| **Data quality diagnostics** (attested) | Emitted as `Diagnostic` fields in `SignablePayload` | OOB indices, empty account keys |
| **Configuration** (`LintConfig`) | Caller controls severity per-rule | Override `oob_program_id` to `Allow` |

## Core Types (`visualsign` crate)

### `SignablePayloadFieldDiagnostic`

```rust
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldDiagnostic {
    #[serde(rename = "Rule")]
    pub rule: String,
    #[serde(rename = "Domain")]
    pub domain: String,
    #[serde(rename = "Level")]
    pub level: String,
    #[serde(rename = "Message")]
    pub message: String,
    #[serde(rename = "InstructionIndex", skip_serializing_if = "Option::is_none")]
    pub instruction_index: Option<u32>,
}
```

Custom `Serialize` impl using `BTreeMap` for alphabetical field ordering (same pattern as `SignablePayloadFieldAmountV2`). Implements `DeterministicOrdering`.

### New variant in `SignablePayloadField`

```rust
#[serde(rename = "diagnostic")]
Diagnostic {
    #[serde(flatten)]
    common: SignablePayloadFieldCommon,
    #[serde(rename = "Diagnostic")]
    diagnostic: SignablePayloadFieldDiagnostic,
},
```

### Field builder

```rust
pub fn create_diagnostic_field(
    rule: &str,
    domain: &str,
    level: &str,
    message: &str,
    instruction_index: Option<u32>,
) -> AnnotatedPayloadField
```

Sets `label` to the rule ID, `fallback_text` to `"{level}: {message}"`.

### `LintConfig`

```rust
pub struct LintConfig {
    pub overrides: HashMap<String, Severity>,
    pub report_all_rules: bool,
}
```

Severity levels: `Ok`, `Warn`, `Error`, `Allow`.

Currently constructed as `LintConfig::default()` in the conversion functions. Future work will wire overrides from `VisualSignOptions` / request metadata.

## Solana Integration (`visualsign-solana` crate)

### `decode_instructions()` and `decode_v0_instructions()`

Functions always succeed. Return `DecodeInstructionsResult` with separate `fields`, `errors`, and `diagnostics` vecs.

Three rules:

| Rule | Domain | Default Level | When |
|------|--------|---------------|------|
| `transaction::oob_program_id` | `transaction` | `warn` | `ci.program_id_index >= account_keys.len()` |
| `transaction::oob_account_index` | `transaction` | `warn` | account index `>= account_keys.len()` |
| `transaction::empty_account_keys` | `transaction` | `error` | `account_keys.is_empty()` |

Account indices are checked on all instructions, including those with OOB program IDs. Original instruction indices are preserved through the visualizer loop for consistent labeling.

### Boot-metric attestation

When `report_all_rules` is true (default), every rule emits a diagnostic — either `ok` (no issues) or `warn`/`error` (issues found). The attester can verify all expected rules ran.

### What does NOT change

- No changes to `ParseRequest`, `ChainMetadata`, or proto definitions
- No changes to the signing/attestation flow — only the contents of `SignablePayload` have been extended to include diagnostics
- `LintConfig` uses defaults only in this slice — wiring from request metadata is future work

## Serialized Output Example

```json
{
  "Fields": [
    {
      "FallbackText": "Solana",
      "Label": "Network",
      "TextV2": { "Text": "Solana" },
      "Type": "text_v2"
    },
    {
      "FallbackText": "warn: instruction 1 skipped: program_id_index 8 out of bounds (5 accounts)",
      "Label": "transaction::oob_program_id",
      "Diagnostic": {
        "Domain": "transaction",
        "InstructionIndex": 1,
        "Level": "warn",
        "Message": "instruction 1 skipped: program_id_index 8 out of bounds (5 accounts)",
        "Rule": "transaction::oob_program_id"
      },
      "Type": "diagnostic"
    }
  ],
  "Title": "Solana Transaction",
  "Version": "0"
}
```

## Tests

1. **Serialization roundtrip** — `SignablePayloadFieldDiagnostic` serializes to JSON with alphabetical keys, deserializes back, passes `verify_deterministic_ordering()`
2. **Integration** — construct a `SolanaTransaction` with an OOB program_id_index, parse it, verify the output contains a `Diagnostic` field with rule `transaction::oob_program_id`
3. **Mixed output** — transaction with some valid and some OOB instructions produces both display fields and diagnostic fields
4. **Boot metrics** — valid transaction emits ok-level diagnostics for all rules
5. **Builder** — `create_diagnostic_field()` produces expected output

## Backwards Compatibility

- Wallets that don't know `Type: "diagnostic"` will hit the `Unknown` deserialization path and can display `FallbackText`
- Payloads without diagnostics are unchanged when `report_all_rules` is false
- Enum variant is appended (index 11), not inserted — existing variant indices are stable
