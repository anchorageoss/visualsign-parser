//! Validation that a [`SignablePayload`] only uses fields the **Anchorage
//! wallet** can render.
//!
//! The wallet's Visual Signing Protocol engine decodes each field's `Type`
//! against a fixed set; anything else is treated as unknown and surfaced to the
//! user as an *unsupported field*. This module mirrors that support matrix so
//! the parser can catch payloads that would fail to render BEFORE they ship —
//! the parser's WYSIWYS contract means an unrenderable field is a correctness
//! bug, not a cosmetic one.
//!
//! Support matrix (kept in lockstep with the wallet's field-type decoder):
//!
//! | Parser type      | Anchorage renders? | Notes                                   |
//! |------------------|--------------------|-----------------------------------------|
//! | `text_v2`        | yes                |                                         |
//! | `address_v2`     | yes                |                                         |
//! | `amount_v2`      | yes                | use this for numeric values, NOT number |
//! | `diagnostic`     | yes                |                                         |
//! | `preview_layout` | yes (container)    | needs both Condensed/Expanded lists on the wire, and every descendant to render |
//! | `list_layout`    | only as a container| INVALID as a standalone field entry     |
//! | `number`         | no                 | not a VSP type; use `amount_v2`         |
//! | `text` (v1)      | no                 | superseded by `text_v2`                 |
//! | `address` (v1)   | no                 | superseded by `address_v2`              |
//! | `amount` (v1)    | no                 | superseded by `amount_v2`               |
//! | `divider`        | no                 | not in the wallet decoder               |
//! | `unknown`        | no                 | explicit fallback/unsupported           |
//!
//! The wallet additionally renders the `delta`, `highlight`, and `rule` leaf
//! types and the `accordion` container, but the parser has no field variant that
//! emits any of them and no plans to add one. They are therefore deliberately
//! excluded — the leaves are absent from [`ANCHORAGE_RENDERABLE_LEAF_TYPES`] and
//! there is no `accordion` container handling — so a payload carrying any of them
//! would be flagged as unsupported. Add support here only when a parser field
//! variant starts emitting the type.
//!
//! `list_layout` is special: the wallet decodes it only as the `Condensed` /
//! `Expanded` list inside a `preview_layout` (or `accordion` section), never via
//! the field-type decoder. So a `list_layout` appearing as an entry in a `Fields`
//! array is unrenderable — to nest, wrap the group in a `preview_layout`.

use crate::errors::VisualSignError;
use crate::{FieldSerializer, SignablePayload, SignablePayloadField};

/// Field types the Anchorage wallet renders as standalone (non-container)
/// fields. Mirrors the recognized cases in the wallet's field-type decoder.
pub const ANCHORAGE_RENDERABLE_LEAF_TYPES: &[&str] =
    &["text_v2", "address_v2", "amount_v2", "diagnostic"];

/// Why a field cannot be rendered by the Anchorage wallet. Mirrors the wallet's
/// unsupported-reason diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorageUnsupportedReason {
    /// The field's `Type` is not in the wallet's renderable set (e.g. `number`,
    /// legacy `text`/`address`/`amount`, `divider`, `unknown`).
    UnsupportedFieldType,
    /// `list_layout` appears as a standalone field entry. It is only valid as
    /// the `Condensed`/`Expanded` container of a `preview_layout`/`accordion`.
    ListLayoutAsStandaloneField,
    /// A `preview_layout` container whose `Condensed`/`Expanded` holds one or
    /// more unsupported descendants. Mirrors the wallet's
    /// contains-unsupported-nested-fields flag.
    ContainsUnsupportedNestedFields,
    /// A `preview_layout` container is missing its `Condensed` or `Expanded`
    /// list on the wire. The wallet's model requires both; a missing list
    /// fails to decode and the whole container falls back to plain text.
    MissingRequiredList,
}

impl AnchorageUnsupportedReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedFieldType => "unsupported field type",
            Self::ListLayoutAsStandaloneField => "list_layout used as a standalone field",
            Self::ContainsUnsupportedNestedFields => "contains unsupported nested fields",
            Self::MissingRequiredList => "missing required Condensed/Expanded list",
        }
    }
}

/// A single field the Anchorage wallet cannot render, with a JSON-ish path so
/// the offending location is easy to find (e.g. `Fields[4].Expanded.Fields[6]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorageRenderFinding {
    pub path: String,
    pub label: String,
    pub field_type: String,
    pub reason: AnchorageUnsupportedReason,
}

impl SignablePayload {
    /// Collect every field the Anchorage wallet cannot render. An empty result
    /// means the payload renders clean.
    pub fn anchorage_render_findings(&self) -> Vec<AnchorageRenderFinding> {
        let mut findings = Vec::new();
        for (index, field) in self.fields.iter().enumerate() {
            check_field(field, &format!("Fields[{index}]"), &mut findings);
        }
        findings
    }

    /// Returns `Err` if any field is unrenderable by the Anchorage wallet.
    /// Suitable as a hard gate (CI / pre-ship) alongside [`Self::validate_charset`].
    pub fn validate_anchorage_wallet_renderable(&self) -> Result<(), VisualSignError> {
        let findings = self.anchorage_render_findings();
        if findings.is_empty() {
            return Ok(());
        }
        let summary = findings
            .iter()
            .map(|f| format!("{} [{}] ({})", f.path, f.field_type, f.reason.as_str()))
            .collect::<Vec<_>>()
            .join("; ");
        Err(VisualSignError::ValidationError(format!(
            "payload contains {} field(s) the Anchorage wallet cannot render: {summary}",
            findings.len()
        )))
    }
}

/// Walk `field`, pushing a finding for every unrenderable field. Returns whether
/// `field` (and all of its descendants) render cleanly.
fn check_field(
    field: &SignablePayloadField,
    path: &str,
    out: &mut Vec<AnchorageRenderFinding>,
) -> bool {
    match field {
        // Container: supported iff both lists are present on the wire and
        // every descendant is.
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // The wallet's PreviewLayout model requires both `Condensed` and
            // `Expanded`. Check the actual wire output rather than the
            // in-memory `Option`s: whether a `None` list is omitted or
            // defaulted on the wire is a serialization-layer decision
            // independent of this check, and this stays correct either way.
            let missing_list = !wire_has_key(field, "PreviewLayout", "Condensed")
                || !wire_has_key(field, "PreviewLayout", "Expanded");

            let mut descendants_ok = true;
            if let Some(condensed) = &preview_layout.condensed {
                for (index, child) in condensed.fields.iter().enumerate() {
                    let ok = check_field(
                        &child.signable_payload_field,
                        &format!("{path}.Condensed.Fields[{index}]"),
                        out,
                    );
                    descendants_ok = descendants_ok && ok;
                }
            }
            if let Some(expanded) = &preview_layout.expanded {
                for (index, child) in expanded.fields.iter().enumerate() {
                    let ok = check_field(
                        &child.signable_payload_field,
                        &format!("{path}.Expanded.Fields[{index}]"),
                        out,
                    );
                    descendants_ok = descendants_ok && ok;
                }
            }

            if missing_list {
                out.push(finding(
                    path,
                    field,
                    AnchorageUnsupportedReason::MissingRequiredList,
                ));
            } else if !descendants_ok {
                out.push(finding(
                    path,
                    field,
                    AnchorageUnsupportedReason::ContainsUnsupportedNestedFields,
                ));
            }
            !missing_list && descendants_ok
        }
        // `list_layout` is never a valid standalone field.
        SignablePayloadField::ListLayout { .. } => {
            out.push(finding(
                path,
                field,
                AnchorageUnsupportedReason::ListLayoutAsStandaloneField,
            ));
            false
        }
        // Every other variant is a leaf: renderable iff its wire `Type` is in
        // the set. Checked against the wire type, not `field_type()`: a
        // variant's wire representation can be remapped (e.g. serialized
        // under a different `Type`) without changing what `field_type()`
        // reports, and this must track what actually reaches the wallet.
        _ => {
            if ANCHORAGE_RENDERABLE_LEAF_TYPES.contains(&wire_type(field).as_str()) {
                true
            } else {
                out.push(finding(
                    path,
                    field,
                    AnchorageUnsupportedReason::UnsupportedFieldType,
                ));
                false
            }
        }
    }
}

/// The `Type` string `field` actually serializes to on the wire. Falls back
/// to [`SignablePayloadField::field_type`] if serialization fails.
fn wire_type(field: &SignablePayloadField) -> String {
    field
        .serialize_to_map()
        .ok()
        .and_then(|map| {
            map.get("Type")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| field.field_type().to_string())
}

/// Whether `field`'s wire serialization of its `container_key` object
/// actually includes `inner_key`.
fn wire_has_key(field: &SignablePayloadField, container_key: &str, inner_key: &str) -> bool {
    let Ok(map) = field.serialize_to_map() else {
        return false;
    };
    map.get(container_key)
        .and_then(|value| value.get(inner_key))
        .is_some()
}

fn finding(
    path: &str,
    field: &SignablePayloadField,
    reason: AnchorageUnsupportedReason,
) -> AnchorageRenderFinding {
    AnchorageRenderFinding {
        path: path.to_string(),
        label: field.label().clone(),
        field_type: wire_type(field),
        reason,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::field_builders::{create_amount_field, create_number_field, create_text_field};
    use crate::{
        AnnotatedPayloadField, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
        SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    };

    fn payload(fields: Vec<SignablePayloadField>) -> SignablePayload {
        SignablePayload {
            fields,
            payload_type: "Test".to_string(),
            subtitle: None,
            title: "Test".to_string(),
            version: "0".to_string(),
        }
    }

    fn text(label: &str) -> AnnotatedPayloadField {
        create_text_field(label, "x").unwrap()
    }

    fn number(label: &str) -> AnnotatedPayloadField {
        create_number_field(label, "1", "bps").unwrap()
    }

    fn amount(label: &str) -> AnnotatedPayloadField {
        create_amount_field(label, "1", "USDC").unwrap()
    }

    fn list_layout_field(label: &str) -> AnnotatedPayloadField {
        AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::ListLayout {
                common: SignablePayloadFieldCommon {
                    label: label.to_string(),
                    fallback_text: label.to_string(),
                },
                list_layout: SignablePayloadFieldListLayout { fields: vec![] },
            },
        }
    }

    fn preview(
        label: &str,
        condensed: Vec<AnnotatedPayloadField>,
        expanded: Vec<AnnotatedPayloadField>,
    ) -> AnnotatedPayloadField {
        preview_with_lists(label, Some(condensed), Some(expanded))
    }

    fn preview_with_lists(
        label: &str,
        condensed: Option<Vec<AnnotatedPayloadField>>,
        expanded: Option<Vec<AnnotatedPayloadField>>,
    ) -> AnnotatedPayloadField {
        AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: label.to_string(),
                    fallback_text: label.to_string(),
                },
                preview_layout: SignablePayloadFieldPreviewLayout {
                    title: Some(SignablePayloadFieldTextV2 {
                        text: label.to_string(),
                    }),
                    subtitle: None,
                    condensed: condensed.map(|fields| SignablePayloadFieldListLayout { fields }),
                    expanded: expanded.map(|fields| SignablePayloadFieldListLayout { fields }),
                },
            },
        }
    }

    fn bare(f: AnnotatedPayloadField) -> SignablePayloadField {
        f.signable_payload_field
    }

    #[test]
    fn supported_leaf_types_render_clean() {
        let p = payload(vec![bare(text("a")), bare(amount("b"))]);
        assert!(p.anchorage_render_findings().is_empty());
        assert!(p.validate_anchorage_wallet_renderable().is_ok());
    }

    #[test]
    fn number_field_is_unsupported() {
        let p = payload(vec![bare(number("Slippage"))]);
        let findings = p.anchorage_render_findings();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field_type, "number");
        assert_eq!(
            findings[0].reason,
            AnchorageUnsupportedReason::UnsupportedFieldType
        );
        assert_eq!(findings[0].path, "Fields[0]");
        assert!(p.validate_anchorage_wallet_renderable().is_err());
    }

    #[test]
    fn list_layout_as_standalone_field_is_unsupported() {
        let p = payload(vec![bare(list_layout_field("group"))]);
        let findings = p.anchorage_render_findings();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field_type, "list_layout");
        assert_eq!(
            findings[0].reason,
            AnchorageUnsupportedReason::ListLayoutAsStandaloneField
        );
    }

    #[test]
    fn preview_layout_with_supported_children_is_clean() {
        let p = payload(vec![bare(preview(
            "Instruction 1",
            vec![text("Program")],
            vec![text("Program ID"), amount("Quoted Out")],
        ))]);
        assert!(p.anchorage_render_findings().is_empty());
    }

    #[test]
    fn preview_layout_with_unsupported_child_flags_child_and_container() {
        // A `number` inside the expanded list makes the whole preview_layout
        // contain unsupported nested fields, and the child itself is reported.
        let p = payload(vec![bare(preview(
            "Instruction 1",
            vec![text("Program")],
            vec![number("Slippage")],
        ))]);
        let findings = p.anchorage_render_findings();
        assert_eq!(findings.len(), 2, "{findings:?}");

        let child = findings
            .iter()
            .find(|f| f.field_type == "number")
            .expect("number child reported");
        assert_eq!(child.path, "Fields[0].Expanded.Fields[0]");
        assert_eq!(
            child.reason,
            AnchorageUnsupportedReason::UnsupportedFieldType
        );

        let container = findings
            .iter()
            .find(|f| f.field_type == "preview_layout")
            .expect("container reported");
        assert_eq!(container.path, "Fields[0]");
        assert_eq!(
            container.reason,
            AnchorageUnsupportedReason::ContainsUnsupportedNestedFields
        );
    }

    #[test]
    fn nested_preview_layout_with_supported_children_renders_clean() {
        // The supported way to nest: a `preview_layout` (NOT a `list_layout`)
        // inside another preview_layout's expanded list, with renderable leaves.
        let inner = preview("Action 2", vec![text("venue")], vec![amount("amount")]);
        let outer = preview(
            "Instruction 3",
            vec![text("Route")],
            vec![text("Program ID"), inner],
        );
        let p = payload(vec![bare(outer)]);
        assert!(
            p.anchorage_render_findings().is_empty(),
            "nested preview_layout of supported leaves should render clean: {:?}",
            p.anchorage_render_findings()
        );
    }

    #[test]
    fn preview_layout_missing_condensed_is_unsupported_even_with_clean_descendants() {
        // Mirrors `create_preview_layout`, which always leaves `condensed:
        // None` (and several Ethereum visualizers leave `expanded: None`
        // too, e.g. the ERC-20 Transfer preview): the wallet's PreviewLayout
        // model requires both lists, so a missing one is a render failure
        // even though every present descendant is supported.
        let p = payload(vec![bare(preview_with_lists(
            "Instruction 1",
            None,
            Some(vec![text("Program")]),
        ))]);
        let findings = p.anchorage_render_findings();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].path, "Fields[0]");
        assert_eq!(
            findings[0].reason,
            AnchorageUnsupportedReason::MissingRequiredList
        );
        assert!(p.validate_anchorage_wallet_renderable().is_err());
    }

    #[test]
    fn preview_layout_missing_both_lists_is_unsupported() {
        let p = payload(vec![bare(preview_with_lists("Instruction 1", None, None))]);
        let findings = p.anchorage_render_findings();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].reason,
            AnchorageUnsupportedReason::MissingRequiredList
        );
    }

    #[test]
    fn validate_error_lists_offending_paths() {
        let p = payload(vec![
            bare(text("ok")),
            bare(preview("Instruction 2", vec![], vec![number("Fee")])),
        ]);
        let err = p
            .validate_anchorage_wallet_renderable()
            .expect_err("should be unrenderable");
        let msg = err.to_string();
        assert!(msg.contains("Fields[1].Expanded.Fields[0]"), "{msg}");
        assert!(msg.contains("number"), "{msg}");
    }
}
