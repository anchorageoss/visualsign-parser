# Lint Diagnostics in SignablePayload — Design Spec

Issue: #228

## Goal

Add structured lint diagnostics to `SignablePayload` as a new `Diagnostic` variant of `SignablePayloadField`. Diagnostics are attested alongside display fields in the signed payload. This first slice implements two rules for Solana legacy transactions, replacing the current silent data dropping.

## Core Types (`visualsign` crate)

### `SignablePayloadFieldDiagnostic`

```rust
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
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

Updates required:
- `serialize_to_map()` — add `Diagnostic` arm returning the diagnostic fields
- `get_expected_fields()` — add `Diagnostic` arm returning `["Diagnostic", "FallbackText", "Label", "Type"]`
- Borsh enum variant index — append after `Unknown` (index 11)

### Field builder

```rust
pub fn create_diagnostic_field(
    rule: &str,
    domain: &str,
    level: &str,
    message: &str,
    instruction_index: Option<u32>,
) -> Result<AnnotatedPayloadField, VisualSignError>
```

Sets `label` to the rule ID, `fallback_text` to `"{level}: {message}"`.

## Solana Integration (`visualsign-solana` crate)

### `instructions.rs` — `decode_instructions()`

Current behavior: `filter_map` silently drops instructions with OOB `program_id_index` and accounts with OOB indices.

New behavior: collect instructions that can be decoded normally, and emit `Diagnostic` fields for dropped data.

Two rules:

| Rule | Domain | Default Level | When |
|------|--------|---------------|------|
| `transaction::oob_program_id` | `transaction` | `warn` | `ci.program_id_index >= account_keys.len()` |
| `transaction::oob_account_index` | `transaction` | `warn` | account index `>= account_keys.len()` |

The function signature changes from returning `Result<Vec<AnnotatedPayloadField>, VisualSignError>` to include diagnostics in the returned fields vec. Diagnostics are appended after the instruction fields.

### What does NOT change

- v0 transaction handling (`txtypes/v0.rs`) — left for a follow-up, keeps current silent drop behavior
- No lint configuration in this slice — all rules use hardcoded default severity
- No changes to `ParseRequest`, `ChainMetadata`, or proto definitions
- No changes to the signing/attestation flow — diagnostics are in `SignablePayload` which is already signed

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
2. **Borsh roundtrip** — diagnostic field survives borsh serialize/deserialize
3. **Integration** — construct a `SolanaTransaction` with an OOB program_id_index, parse it, verify the output contains a `Diagnostic` field with rule `transaction::oob_program_id`
4. **Mixed output** — transaction with some valid and some OOB instructions produces both display fields and diagnostic fields
5. **Builder** — `create_diagnostic_field()` produces expected output

## Backwards Compatibility

- Wallets that don't know `Type: "diagnostic"` will hit the `Unknown` deserialization path and can display `FallbackText`
- Payloads without diagnostics are unchanged — no new fields appear unless a rule fires
- Borsh enum variant is appended (index 11), not inserted — existing variant indices are stable
