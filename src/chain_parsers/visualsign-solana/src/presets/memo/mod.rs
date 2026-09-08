//! SPL Memo preset implementation for Solana.
//!
//! The Memo program (`MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`) is a
//! native, non-Anchor program: its instruction data carries no 8-byte
//! discriminator and no borsh-encoded arguments. The entire data buffer *is*
//! the memo, which the program requires to be valid UTF-8. This visualizer
//! therefore decodes the data directly as text rather than going through the
//! IDL/discriminator path used by Anchor-program presets (e.g. dflow_aggregator).
//! It mirrors the native-program pattern of `compute_budget` and
//! `associated_token_account`.

mod config;

use crate::core::{
    InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::MemoConfig;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// Canonical display name for the program, matching
/// `idl::builtin_programs::canonical_name`.
const PROGRAM_DISPLAY_NAME: &str = "Memo Program";

static MEMO_CONFIG: MemoConfig = MemoConfig;

pub struct MemoVisualizer;

impl InstructionVisualizer for MemoVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let program_id = match context.program_id() {
            ProgramRef::Resolved(pk) => pk.to_string(),
            ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
        };
        render_memo(&program_id, context.data(), context.instruction_index())
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&MEMO_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("Memo")
    }
}

/// Render the raw instruction data as the memo's text.
///
/// The SPL Memo program requires the memo to be valid UTF-8, so the common
/// case is a clean decode. Empty data and non-UTF-8 data fall back to an ASCII
/// placeholder; the raw bytes stay available in the "Raw Data" field either
/// way.
fn memo_display_text(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok("") => "(empty memo)".to_string(),
        Ok(text) => text.to_string(),
        Err(_) => "(non-UTF-8 data; see Raw Data)".to_string(),
    }
}

/// Build the preview layout for a memo instruction. Split out from
/// `visualize_tx_commands` so the rendering can be unit-tested without
/// constructing a full `VisualizerContext`.
fn render_memo(
    program_id: &str,
    data: &[u8],
    instruction_index: usize,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let memo_text = memo_display_text(data);
    let hex_fallback = hex::encode(data);

    let condensed = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program", PROGRAM_DISPLAY_NAME)?,
            create_text_field("Memo", &memo_text)?,
        ],
    };

    let expanded = SignablePayloadFieldListLayout {
        fields: vec![
            create_text_field("Program ID", program_id)?,
            create_text_field("Memo", &memo_text)?,
            create_raw_data_field(data, Some(hex_fallback.clone()))?,
        ],
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: "Memo".to_string(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: format!("Instruction {}", instruction_index + 1),
                fallback_text: format!("Program ID: {program_id}\nData: {hex_fallback}"),
            },
            preview_layout,
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// (label, value) pairs extracted from a list of text fields.
    type LabeledFields = Vec<(String, String)>;

    fn field_label_value(field: &AnnotatedPayloadField) -> (String, String) {
        match &field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                (common.label.clone(), text_v2.text.clone())
            }
            other => panic!("expected TextV2 field, got {other:?}"),
        }
    }

    fn preview_parts(field: &AnnotatedPayloadField) -> (String, LabeledFields, LabeledFields) {
        match &field.signable_payload_field {
            SignablePayloadField::PreviewLayout { preview_layout, .. } => {
                let title = preview_layout
                    .title
                    .as_ref()
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let condensed = preview_layout
                    .condensed
                    .as_ref()
                    .map(|c| c.fields.iter().map(field_label_value).collect())
                    .unwrap_or_default();
                let expanded = preview_layout
                    .expanded
                    .as_ref()
                    .map(|e| e.fields.iter().map(field_label_value).collect())
                    .unwrap_or_default();
                (title, condensed, expanded)
            }
            other => panic!("expected PreviewLayout field, got {other:?}"),
        }
    }

    #[test]
    fn test_memo_display_text_decodes_utf8() {
        assert_eq!(
            memo_display_text(b"Payment for invoice 42"),
            "Payment for invoice 42"
        );
    }

    #[test]
    fn test_memo_display_text_preserves_unicode() {
        // The Memo program accepts any valid UTF-8; non-ASCII content must pass
        // through unchanged rather than being mangled or rejected. Built from
        // escapes to keep this source file ASCII-only.
        let memo = "caf\u{e9} \u{1f600} \u{65e5}\u{672c}\u{8a9e}";
        assert_eq!(memo_display_text(memo.as_bytes()), memo);
    }

    #[test]
    fn test_memo_display_text_empty() {
        assert_eq!(memo_display_text(b""), "(empty memo)");
    }

    #[test]
    fn test_memo_display_text_invalid_utf8() {
        // 0xFF is never a valid UTF-8 byte.
        assert_eq!(
            memo_display_text(&[0xff, 0xfe, 0xfd]),
            "(non-UTF-8 data; see Raw Data)"
        );
    }

    #[test]
    fn test_render_memo_builds_preview() {
        let field = render_memo(MEMO_PROGRAM_ID, b"hello world", 0).unwrap();
        let (title, condensed, expanded) = preview_parts(&field);

        assert_eq!(title, "Memo");
        assert_eq!(
            condensed,
            vec![
                ("Program".to_string(), "Memo Program".to_string()),
                ("Memo".to_string(), "hello world".to_string()),
            ]
        );
        assert_eq!(
            expanded,
            vec![
                ("Program ID".to_string(), MEMO_PROGRAM_ID.to_string()),
                ("Memo".to_string(), "hello world".to_string()),
                ("Raw Data".to_string(), hex::encode(b"hello world")),
            ]
        );
    }

    #[test]
    fn test_render_memo_instruction_label_is_one_indexed() {
        let field = render_memo(MEMO_PROGRAM_ID, b"x", 2).unwrap();
        match &field.signable_payload_field {
            SignablePayloadField::PreviewLayout { common, .. } => {
                assert_eq!(common.label, "Instruction 3");
            }
            other => panic!("expected PreviewLayout field, got {other:?}"),
        }
    }

    #[test]
    fn test_render_memo_invalid_utf8_keeps_raw_data() {
        let bytes = [0xff_u8, 0x00, 0x10];
        let field = render_memo(MEMO_PROGRAM_ID, &bytes, 0).unwrap();
        let (_title, _condensed, expanded) = preview_parts(&field);

        assert!(
            expanded
                .iter()
                .any(|(l, v)| l == "Memo" && v == "(non-UTF-8 data; see Raw Data)"),
            "expanded view should show the non-UTF-8 placeholder for the Memo field"
        );
        assert!(
            expanded
                .iter()
                .any(|(l, v)| l == "Raw Data" && *v == hex::encode(bytes)),
            "expanded view should still carry the raw bytes as hex"
        );
    }

    #[test]
    fn test_memo_config_handles_program_id() {
        let config = MemoConfig::new();
        assert!(config.can_handle(MEMO_PROGRAM_ID));
        assert!(!config.can_handle("11111111111111111111111111111111"));
    }

    #[test]
    fn test_memo_kind_is_payments() {
        assert_eq!(MemoVisualizer.kind(), VisualizerKind::Payments("Memo"));
    }
}
