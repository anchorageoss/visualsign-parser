# Anchorage Wallet Render Validation Rules

Status: draft · Owner: parser team

## Purpose

The parser's output is consumed by the **Anchorage wallet**, whose Visual
Signing Protocol engine renders each `SignablePayload` field according to a
fixed support matrix. Any field the wallet does not recognize is surfaced to the
user as an **unsupported field** (and nested containers are flagged as
containing unsupported nested fields). Because VSP is WYSIWYS ("what you see is what
you sign"), an unrenderable field is a **correctness bug**, not a cosmetic one.

These rules let the parser validate its own output against the wallet's support
matrix *before* shipping, the same way `validate_charset` guards the character
set.

Source of truth: the Anchorage wallet's Visual Signing Protocol engine (its
field-type decoder, preview-layout and accordion parsing, and field-diagnostic
model). Mirror this spec to the wallet whenever that engine changes.

## Field support matrix

The wallet's field-type decoder recognizes exactly these `Type` strings; every
other string decodes to unknown → unsupported.

| `Type`           | Renders? | Kind       | Notes                                              |
|------------------|----------|------------|----------------------------------------------------|
| `text_v2`        | yes      | leaf       |                                                    |
| `address_v2`     | yes      | leaf       |                                                    |
| `amount_v2`      | yes      | leaf       | **use this for numeric values** (amounts, bps, fees) |
| `diagnostic`     | yes      | leaf       | data-quality surfacing; never omitted (WYSIWYS)    |
| `delta`          | wallet-only | leaf    | wallet renders it, but **not in the validator's allowed set** (parser emits none; no plans) |
| `highlight`      | wallet-only | leaf    | wallet renders it, but **not in the validator's allowed set** (parser emits none; no plans) |
| `rule`           | wallet-only | leaf    | wallet renders it, but **not in the validator's allowed set** (parser emits none; no plans) |
| `preview_layout` | yes      | container  | every descendant must render                       |
| `accordion`      | wallet-only | container | wallet renders it, but **not in the validator's allowed set** (parser emits none; no plans) |
| `list_layout`    | **structural only** | — | valid only as a container's `Condensed`/`Expanded`; **never as a field entry** |
| `number`         | yes, as `amount_v2` | leaf | not a VSP type on its own; the in-memory `Number` variant serializes to `amount_v2` on the wire (see #393), so it renders fine |
| `text` (v1)      | **no**   | —          | superseded by `text_v2`                            |
| `address` (v1)   | **no**   | —          | superseded by `address_v2`                         |
| `amount` (v1)    | **no**   | —          | superseded by `amount_v2`                          |
| `divider`        | **no**   | —          | not in the wallet decoder                          |
| `unknown`        | **no**   | —          | explicit fallback / unsupported                    |

The validator's allowed leaf set is intentionally narrower than the wallet's
decoder: only the `yes` rows (`text_v2`, `address_v2`, `amount_v2`, `diagnostic`,
plus the `preview_layout` container). The `wallet-only` types render in the wallet
but the parser has no field variant that emits them and no plans to add one, so a
payload using one is flagged. Promote a `wallet-only` row to the allowed set only
when a parser field variant starts emitting it.

## Structural rules

1. **`list_layout` is not a field type.** The wallet decodes `list_layout` only
   as the `Condensed`/`Expanded` list inside a `preview_layout`, or the
   `Condensed`/`Expanded` of an `accordion` section. A `list_layout` appearing as
   an entry in a `Fields` array is unrenderable. **To nest a group, wrap it in a
   `preview_layout`** (title + condensed + expanded), not a bare `list_layout`.

2. **Containers propagate unsupported-ness.** A `preview_layout` (or `accordion`)
   is rendered only if *every* descendant in its `Condensed`/`Expanded` lists is
   renderable. A single unsupported descendant flags the whole container as
   containing unsupported nested fields. Nesting of supported leaves is unlimited in
   depth; nesting via `preview_layout` is the supported way to express hierarchy.

3. **`preview_layout` always carries both `Condensed` and `Expanded` on the
   wire.** The `SignablePayloadFieldPreviewLayout` serializer emits an empty
   list for whichever is unset (see #403), so the wallet's decoder always
   receives both — there is no missing-list failure mode for the validator to
   catch. `create_preview_layout` always leaves `Condensed` unset in memory,
   and several Ethereum visualizers (e.g. the ERC-20 `Transfer` preview) leave
   `Expanded` unset too; both cases render clean.

4. **`number` fields render fine, via `amount_v2` on the wire.** VSP has no
   `number` type; the in-memory `Number` variant is serialized to `amount_v2`
   (see #393), so it is not flagged as unsupported. New code should still
   prefer building `amount_v2` fields directly (via `create_amount_field`)
   rather than `create_number_field`, since the remap is a compatibility
   shim, not the primary path.

## Validator

`visualsign::SignablePayload` exposes:

- `anchorage_render_findings() -> Vec<AnchorageRenderFinding>` — every
  unrenderable field, each with a JSON-ish `path` (e.g.
  `Fields[2].Expanded.Fields[1]`), `label`, `field_type`, and `reason`.
- `validate_anchorage_wallet_renderable() -> Result<(), VisualSignError>` — a
  hard gate that errors (listing offending paths) if any field is unrenderable.

`reason` mirrors the wallet's `UnsupportedReason`:

- `UnsupportedFieldType` — type not in the renderable set. Checked against the
  wire `Type` the field actually serializes to, not the in-memory variant name,
  since a variant can be remapped to a different wire representation.
- `ListLayoutAsStandaloneField` — `list_layout` used as a field entry.
- `ContainsUnsupportedNestedFields` — a container with an unsupported descendant.

Ad-hoc check of a payload JSON:

```bash
cargo run -p visualsign --features diagnostics \
  --example check_anchorage_render -- path/to/payload.json
```

## Known parser violations (as of this writing)

None. `create_number_field` (used by the `compute_budget`, `system`,
`token_2022`, and `jupiter_swap` presets) previously emitted the unrenderable
`number` type; #393 made the `Number` variant serialize as `amount_v2` on the
wire, so those call sites render fine without needing a migration.
